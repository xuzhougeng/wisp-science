using System.Text;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Automation;
using Microsoft.Web.WebView2.Core;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeWorkspaceTerminal : UserControl, IDisposable
{
    private const string Page = "https://wisp-terminal.local/index.html";
    private readonly WorkspaceTerminalModel model;
    private readonly WispDesign design;
    private readonly WebView2 view = new();
    private readonly ComboBox selection = new() { MinWidth = 100, MaxWidth = 240, PlaceholderText = "选择终端" };
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Button resume = new() { Content = "已核对输出，恢复输入", Visibility = Visibility.Collapsed };
    private readonly TextBox fallback = new() { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
    private readonly CancellationTokenSource lifetime = new();
    private readonly Task initialLoad;
    private bool disposed, ready, initializing, syncingSelection;
    private string? stateFingerprint, terminalFingerprint;
    private readonly Button open = new() { Content = "新建本地终端" };
    private readonly Button close = new() { Content = "关闭终端" };
    private readonly Button interrupt = new() { Content = "中断" };

    public NativeWorkspaceTerminal(WorkspaceTerminalModel model, WispDesign design, Action hide)
    {
        this.model = model; this.design = design;
        Height = 300; MinHeight = 180;
        design.BindTypography(this);
        var root = new Grid { Padding = new Thickness(10, 0, 10, 10), RowSpacing = 6, Background = design.Brush("bg-sunken") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var grip = new Thumb { Height = 10, HorizontalAlignment = HorizontalAlignment.Stretch, IsTabStop = true };
        AutomationProperties.SetName(grip, "调整终端高度，向上或向下箭头调整");
        void Resize(double value) => Height = Math.Clamp(value, 180, Math.Max(180, (XamlRoot?.Size.Height ?? 800) * .65));
        grip.DragDelta += (_, e) => Resize(Height - e.VerticalChange);
        grip.KeyDown += (_, e) => { if (e.Key is Windows.System.VirtualKey.Up or Windows.System.VirtualKey.Down) { Resize(Height + (e.Key == Windows.System.VirtualKey.Up ? 24 : -24)); e.Handled = true; } };
        root.Children.Add(grip);
        var toolbar = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        var refresh = new Button { Content = "刷新" };
        refresh.Click += async (_, _) => { await model.LoadAsync(lifetime.Token); Render(); };
        open.Click += async (_, _) => await OpenContextAsync("local");
        interrupt.Click += async (_, _) => { await model.WriteAsync([3], lifetime.Token); Render(); };
        close.Click += async (_, _) => { await model.CloseSelectedAsync(lifetime.Token); Render(); };
        var dismiss = new Button { Content = "隐藏" };
        dismiss.Click += (_, _) => hide();
        selection.SelectionChanged += (_, _) => { if (!syncingSelection && selection.SelectedItem is ComboBoxItem row) { model.Select((string)row.Tag); Render(); } };
        AutomationProperties.SetName(selection, "当前终端");
        toolbar.Children.Add(selection); toolbar.Children.Add(refresh); toolbar.Children.Add(open); toolbar.Children.Add(interrupt); toolbar.Children.Add(close); toolbar.Children.Add(dismiss);
        var toolbarScroll = new ScrollViewer { Content = toolbar, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled };
        Grid.SetRow(toolbarScroll, 1); root.Children.Add(toolbarScroll);
        Grid.SetRow(view, 2); root.Children.Add(view);
        Grid.SetRow(fallback, 2); root.Children.Add(fallback);
        var feedback = new StackPanel { Spacing = 4 };
        feedback.Children.Add(status); feedback.Children.Add(resume);
        Grid.SetRow(feedback, 3); root.Children.Add(feedback);
        resume.Click += (_, _) => { model.ResumeInput(); Render(); };
        Content = root;
        model.OutputReceived += ReceiveOutput;
        Loaded += async (_, _) => await InitializeAsync();
        design.ApplyTypography(this);
        initialLoad = model.LoadAsync(lifetime.Token);
        _ = LoopAsync();
    }

    private async Task InitializeAsync()
    {
        if (disposed || initializing || ready) return;
        initializing = true;
        try
        {
            var environment = await CoreWebView2Environment.CreateWithOptionsAsync(null,
                Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "WispScience", "NativeWebView2"), null);
            if (disposed) return;
            await view.EnsureCoreWebView2Async(environment);
            if (disposed) return;
            var core = view.CoreWebView2;
            core.Settings.AreHostObjectsAllowed = false;
            core.Settings.AreDevToolsEnabled = false;
            core.Settings.AreDefaultContextMenusEnabled = false;
            core.Settings.IsStatusBarEnabled = false;
            core.SetVirtualHostNameToFolderMapping("wisp-terminal.local", Path.Combine(AppContext.BaseDirectory, "Assets", "Terminal"), CoreWebView2HostResourceAccessKind.DenyCors);
            core.NavigationStarting += (_, e) => { if (e.Uri != Page) e.Cancel = true; };
            core.NewWindowRequested += (_, e) => e.Handled = true;
            core.PermissionRequested += (_, e) => e.State = CoreWebView2PermissionState.Deny;
            core.WebMessageReceived += MessageReceived;
            core.NavigationCompleted += (_, e) => { if (!e.IsSuccess && !disposed) ShowFailure("终端视图加载失败：" + e.WebErrorStatus); };
            core.ProcessFailed += (_, _) => { if (!disposed) ShowFailure("终端视图进程已停止。输出保留为只读文本；重新打开会话可重新加载视图。"); };
            core.Navigate(Page);
            await Task.Delay(TimeSpan.FromSeconds(10), lifetime.Token);
            if (!disposed && !ready) ShowFailure("终端视图未能就绪。输出保留为只读文本；重新打开会话可重新加载视图。");
        }
        catch (Exception ex) { if (!disposed) ShowFailure("无法加载终端视图：" + ex.Message); }
    }

    private void ShowFailure(string message)
    {
        ready = false; status.Text = message;
        view.Visibility = Visibility.Collapsed; fallback.Visibility = Visibility.Visible; fallback.Text = model.Output;
    }

    private async void MessageReceived(object? sender, CoreWebView2WebMessageReceivedEventArgs e)
    {
        if (disposed || e.Source != Page) return;
        try
        {
            using var json = JsonDocument.Parse(e.WebMessageAsJson);
            var data = json.RootElement;
            var type = data.GetProperty("type").GetString();
            if (type == "ready") { ready = true; stateFingerprint = null; model.Select(model.SelectedId); Render(); return; }
            if (!ready || data.GetProperty("version").GetInt32() != model.SelectionVersion) return;
            if (type == "input") await model.WriteAsync(Encoding.UTF8.GetBytes(data.GetProperty("data").GetString() ?? ""), lifetime.Token);
            else if (type == "binary") await model.WriteAsync(Convert.FromBase64String(data.GetProperty("data").GetString() ?? ""), lifetime.Token);
            else if (type == "resize" && data.GetProperty("rows").TryGetUInt16(out var rows) && data.GetProperty("cols").TryGetUInt16(out var cols))
                await model.ResizeAsync(rows, cols, lifetime.Token);
            Render();
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!disposed) status.Text = "终端消息无法处理：" + ex.Message; }
    }

    public async Task OpenContextAsync(string context)
    {
        await initialLoad;
        if (disposed) return;
        await model.OpenAsync(context, lifetime.Token); Render();
    }

    public bool HandleEscape()
    {
        if (!selection.IsDropDownOpen) return false;
        selection.IsDropDownOpen = false; return true;
    }

    private async Task LoopAsync()
    {
        try
        {
            await initialLoad;
            Render();
            while (!lifetime.IsCancellationRequested)
            {
                await model.ReadAsync(lifetime.Token); Render();
                await Task.Delay(200, lifetime.Token);
            }
        }
        catch (OperationCanceledException) { }
    }

    private void ReceiveOutput(NativeTerminalOutput chunk)
    {
        if (!ready || disposed) return;
        Render();
        view.CoreWebView2.PostWebMessageAsJson(JsonSerializer.Serialize(new { type = "output", version = model.SelectionVersion, reset = chunk.Reset, base64 = chunk.Base64 }));
    }

    private void Render()
    {
        if (disposed) return;
        var terminals = JsonSerializer.Serialize(model.Terminals);
        syncingSelection = true;
        if (terminalFingerprint != terminals)
        {
            terminalFingerprint = terminals; selection.Items.Clear();
            foreach (var terminal in model.Terminals) selection.Items.Add(new ComboBoxItem { Content = terminal.Title + (terminal.Running ? "" : " · 已退出"), Tag = terminal.Id });
        }
        selection.SelectedItem = selection.Items.Cast<ComboBoxItem>().FirstOrDefault(row => (string)row.Tag == model.SelectedId);
        syncingSelection = false;
        open.IsEnabled = close.IsEnabled = !model.Busy;
        var enabled = model.SelectedId != null && !model.InputUncertain && model.ExitCode is null;
        interrupt.IsEnabled = enabled;
        resume.Visibility = model.InputUncertain ? Visibility.Visible : Visibility.Collapsed;
        if (fallback.Visibility == Visibility.Visible) fallback.Text = model.Output;
        else status.Text = model.InputUncertain ? "输入结果未确认。请核对终端输出后恢复输入；不会自动重发。"
            : model.Error ?? (model.SelectedId == null ? "选择已有终端或新建终端。隐藏面板不会结束进程。" : model.ExitCode is { } code ? $"进程已退出 · {code}" : "");
        if (!ready) return;
        string Color(string token) { var c = design.Brush(token).Color; return $"#{c.R:X2}{c.G:X2}{c.B:X2}"; }
        var state = JsonSerializer.Serialize(new { type = "state", version = model.SelectionVersion, enabled,
            fontSize = design.FontSize(12, true), fontFamily = design.Typography.CodeFamily, background = Color("bg-sunken"), foreground = Color("text") });
        if (state != stateFingerprint) { stateFingerprint = state; view.CoreWebView2.PostWebMessageAsJson(state); }
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true; lifetime.Cancel(); model.OutputReceived -= ReceiveOutput; model.Detach();
        if (view.CoreWebView2 != null) view.CoreWebView2.WebMessageReceived -= MessageReceived;
        view.Close(); lifetime.Dispose();
    }
}
