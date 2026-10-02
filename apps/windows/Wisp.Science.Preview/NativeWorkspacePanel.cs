using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel : UserControl, IDisposable
{
    private readonly WorkspacePanelModel model;
    private readonly WispDesign design;
    private readonly Func<ConversationItem[]> transcript;
    private readonly WorkspaceSideChatModel? sideChat;
    private readonly Action<string>? openTerminal;
    private readonly Action close;
    private readonly WorkspaceRunReviewModel? runReview;
    private readonly Action<string, string>? tabChanged;
    private readonly TextBox filter = new() { PlaceholderText = "筛选名称" };
    private readonly StackPanel body = new() { Spacing = 8 };
    private readonly ScrollViewer scroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    private readonly NativePanelViewState viewState = new();
    private NativePanelViewState.ReadingState? readingState;
    private bool syncingFilter;
    private bool restoreScroll;
    private readonly Dictionary<string, ToggleButton> tabButtons = [];
    private readonly Dictionary<string, FrameworkElement> previewImages = [];
    private readonly HashSet<string> previewLoading = [];
    private NativeRichPreview? pdfPreview;
    private NativePanelFileContent? pdfContent;
    private readonly CancellationTokenSource lifetime = new();
    private bool disposed;
    public async Task ShowFilesAsync() => await ShowTabAsync("files");
    public Task ShowRunAsync(string runId) => PresentRunAsync(runId, true);
    private async Task PresentRunAsync(string runId, bool openHosts)
    {
        if (disposed) return;
        runReview?.Close();
        var read = openHosts ? model.OpenRunAsync(runId, lifetime.Token) : model.ReadRunAsync(runId, lifetime.Token);
        viewState.For("hosts").SetOffset(0);
        if (openHosts) tabChanged?.Invoke(model.Tabs.Selected, model.Tabs.Saved);
        Render();
        await read;
        Render();
    }
    public async Task ShowTabAsync(string tab)
    {
        runReview?.Close();
        try
        {
            var refresh = model.RefreshAsync(tab, cancellationToken: lifetime.Token);
            tabChanged?.Invoke(model.Tabs.Selected, model.Tabs.Saved);
            Render();
            await refresh;
            Render();
        }
        catch (OperationCanceledException) { }
    }

    public NativeWorkspacePanel(WorkspacePanelModel model, Func<ConversationItem[]> transcript, WispDesign design, Action close,
        WorkspaceSideChatModel? sideChat = null, Action<string>? openTerminal = null, Action<string, string>? tabChanged = null,
        WorkspaceRunReviewModel? runReview = null)
    {
        this.model = model; this.transcript = transcript; this.design = design; this.close = close;
        this.tabChanged = tabChanged;
        this.runReview = runReview;
        if (runReview != null) runReview.Changed += Render;
        design.BindTypography(this);
        this.sideChat = sideChat; this.openTerminal = openTerminal;
        var root = new Grid { Padding = new Thickness(12), Background = design.Brush("bg-sunken") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var header = new Grid();
        header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var tabStrip = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        var tabs = new ScrollViewer
        {
            HorizontalScrollBarVisibility = ScrollBarVisibility.Hidden,
            HorizontalScrollMode = ScrollMode.Enabled,
            VerticalScrollMode = ScrollMode.Disabled,
            Content = tabStrip
        };
        foreach (var id in model.Tabs.Available)
        {
            var captured = id;
            var tab = new ToggleButton { Content = Label(id), Padding = new Thickness(8, 4, 8, 4), IsChecked = model.Tabs.Selected == id };
            AutomationProperties.SetName(tab, Label(id));
            tabButtons[id] = tab;
            tab.Click += async (_, _) => await ShowTabAsync(captured);
            tabStrip.Children.Add(tab);
        }
        header.Children.Add(tabs);
        var dismiss = new Button { Content = design.Icon("close", 14), Padding = new Thickness(6) };
        dismiss.Click += (_, _) => close();
        ToolTipService.SetToolTip(dismiss, "关闭面板");
        Grid.SetColumn(dismiss, 1); header.Children.Add(dismiss);
        root.Children.Add(header);
        filter.TextChanged += (_, _) =>
        {
            if (syncingFilter || disposed) return;
            readingState?.SetFilter(filter.Text);
            Render();
        };
        Grid.SetRow(filter, 1); root.Children.Add(filter);
        scroll.Content = body;
        scroll.ViewChanged += (_, _) =>
        {
            if (!disposed && !restoreScroll && !model.Loading) readingState?.SetOffset(scroll.VerticalOffset);
        };
        scroll.LayoutUpdated += RestoreScroll;
        Grid.SetRow(scroll, 2); root.Children.Add(scroll);
        Content = root;
        _ = StartAsync();
    }

    private async Task StartAsync()
    {
        try
        {
            await model.RefreshAsync(model.Tabs.Selected, cancellationToken: lifetime.Token);
            if (sideChat is not null) await sideChat.LoadOptionsAsync(lifetime.Token);
            Render();
        }
        catch (OperationCanceledException) { }
    }

    public void Refresh() { if (!disposed) Render(); }

    private void RestoreScroll(object? sender, object e)
    {
        if (disposed || !restoreScroll || model.Loading || readingState is null) return;
        scroll.ChangeView(null, Math.Min(readingState.Offset, scroll.ScrollableHeight), null, disableAnimation: true);
        restoreScroll = false;
    }

    private void Render()
    {
        if (disposed) return;
        restoreScroll = true;
        readingState = viewState.For(model.Tabs.Selected, model.Path);
        syncingFilter = true;
        filter.Text = readingState.Filter;
        syncingFilter = false;
        foreach (var (id, button) in tabButtons)
        {
            button.IsChecked = id == model.Tabs.Selected;
            button.Background = design.Brush(id == model.Tabs.Selected ? "surface-hover" : "bg-sunken");
        }
        body.Children.Clear();
        filter.Visibility = runReview?.Visible == true && model.Tabs.Selected == "hosts" ? Visibility.Collapsed : Visibility.Visible;
        if (runReview?.Visible == true && model.Tabs.Selected == "hosts")
        { RenderRunReview(); design.ApplyTypography(this); return; }
        if (model.Loading) body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        if (model.Error is { } error) body.Children.Add(new TextBlock { Text = error, TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong"), FontSize = 12 });
        var query = filter.Text.Trim();
        if (model.Tabs.Selected == "artifacts") RenderArtifacts(query);
        else if (model.Tabs.Selected == "files") RenderFiles(query);
        else if (model.Tabs.Selected == "hosts") RenderHosts();
        else if (model.Tabs.Selected == "agents") RenderAgents(query);
        else if (model.Tabs.Selected == "notebook") RenderNotebook(query);
        else if (model.Tabs.Selected == "highlights") RenderHighlights(query);
        else if (model.Tabs.Selected == "provenance")
        {
            foreach (var row in NativeProvenanceRow.Collect(transcript()).Where(item => item.Matches(query)))
                body.Children.Add(Row(row.Name, row.Output.Length == 0 ? row.Input : row.Output, "list"));
        }
        else if (model.Tabs.Selected == "sidechat") RenderSideChat();
        RenderPreview();
        design.ApplyTypography(this);
    }

    private void RenderArtifacts(string query)
    {
        var groups = NativeArtifactGroups.Collect(model.Artifacts, query);
        foreach (var group in groups)
        {
            var header = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, Margin = new Thickness(0, 6, 0, 0) };
            header.Children.Add(new TextBlock { Text = group.Label, FontSize = 12, Foreground = design.Brush("text-muted"), FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
            header.Children.Add(new TextBlock { Text = group.Items.Count.ToString(), FontSize = 11, Foreground = design.Brush("text-faint"), VerticalAlignment = VerticalAlignment.Bottom });
            body.Children.Add(header);
            foreach (var artifact in group.Items)
            {
                var captured = artifact;
                var button = Row(captured.Name, (captured.LogicalPath ?? captured.Path), "doc");
                button.Click += async (_, _) => { await model.ReadArtifactAsync(captured.Id, lifetime.Token); Render(); };
                body.Children.Add(button);
            }
        }
        if (model.Artifacts.Length == 0 && !model.Loading) body.Children.Add(Mute("这个会话暂无产物"));
        else if (groups.Count == 0 && !model.Loading) body.Children.Add(Mute("没有匹配的产物"));
    }

    private void RenderFiles(string query)
    {
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        actions.Children.Add(FileButton("新建文件", NativePanelFileAction.CreateFile));
        actions.Children.Add(FileButton("新建文件夹", NativePanelFileAction.CreateDirectory));
        body.Children.Add(actions);
        var up = new Button { Content = "上级", IsEnabled = model.Path != "." };
        up.Click += async (_, _) => { await model.RefreshAsync("files", model.Parent, lifetime.Token); Render(); };
        body.Children.Add(up);
        body.Children.Add(Mute(model.Path));
        foreach (var file in model.Files.Where(item => query.Length == 0 || item.Name.Contains(query, StringComparison.CurrentCultureIgnoreCase)))
        {
            var captured = file;
            var row = new Grid();
            row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var button = Row(captured.Name, captured.IsDir ? "文件夹" : $"{captured.Size} bytes", captured.IsDir ? "folder" : "doc");
            button.Click += async (_, _) =>
            {
                if (captured.IsDir) await model.RefreshAsync("files", WorkspacePanelModel.Child(model.Path, captured.Name), lifetime.Token);
                else await model.ReadFileAsync(WorkspacePanelModel.Child(model.Path, captured.Name), lifetime.Token);
                Render();
            };
            row.Children.Add(button);
            var menu = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
            var rename = new Button { Content = "重命名" };
            rename.Click += async (_, _) => await PromptFileAsync(NativePanelFileAction.Rename, captured.Name);
            var delete = new Button { Content = "删除" };
            delete.Click += async (_, _) => await PromptFileAsync(NativePanelFileAction.Delete, captured.Name);
            menu.Children.Add(rename); menu.Children.Add(delete); Grid.SetColumn(menu, 1); row.Children.Add(menu);
            body.Children.Add(row);
        }
    }

    private Button FileButton(string label, NativePanelFileAction action)
    {
        var button = new Button { Content = label };
        button.Click += async (_, _) => await PromptFileAsync(action);
        return button;
    }

    private async Task PromptFileAsync(NativePanelFileAction action, string currentName = "")
    {
        var name = new TextBox { Text = currentName, PlaceholderText = "名称" };
        var dialog = new ContentDialog
        {
            Title = action switch { NativePanelFileAction.CreateFile => "新建文件", NativePanelFileAction.CreateDirectory => "新建文件夹", NativePanelFileAction.Rename => "重命名", _ => "删除" },
            Content = action == NativePanelFileAction.Delete ? new TextBlock { Text = $"永久删除“{currentName}”？文件夹内的所有内容也会被删除。此操作无法撤销。", TextWrapping = TextWrapping.Wrap } : name,
            PrimaryButtonText = action == NativePanelFileAction.Delete ? "删除" : "确定",
            CloseButtonText = "取消",
            XamlRoot = XamlRoot
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        try
        {
            var target = NativePanelPaths.Destination(model.Path, action is NativePanelFileAction.Rename or NativePanelFileAction.Delete ? currentName : name.Text);
            var destination = action == NativePanelFileAction.Rename ? NativePanelPaths.Destination(model.Path, name.Text) : null;
            await model.PerformFileActionAsync(action, target, destination, lifetime.Token);
            Render();
        }
        catch (Exception ex) { await ShowErrorAsync("操作未确认成功，请刷新核对后再操作。\n" + ex.Message); Render(); }
    }

    private async Task ShowErrorAsync(string message)
    {
        var dialog = new ContentDialog { Title = "文件操作", Content = message, CloseButtonText = "关闭", XamlRoot = XamlRoot };
        await dialog.ShowAsync();
    }

    private void RenderHosts()
    {
        RenderRunDetail();
        if (model.Contexts?.DefaultContext is { } currentDefault)
        {
            var id = currentDefault.ContextId;
            var label = id is null ? "继承全局设置" : id == "local" ? "本机" :
                model.Contexts.Contexts.FirstOrDefault(context => context.Id == id)?.Label ?? "环境不可用：" + id;
            body.Children.Add(Mute("本会话默认执行环境：" + label));
        }
        if (model.ContextUncertain)
        {
            var refresh = new Button { Content = "重新读取环境设置", IsEnabled = !model.Loading && !model.ContextBusy };
            refresh.Click += async (_, _) => await ShowTabAsync("hosts");
            body.Children.Add(refresh);
        }
        foreach (var context in model.Contexts?.Contexts ?? [])
        {
            var captured = context;
            var attached = model.Contexts!.EnabledIds.Contains(captured.Id) || captured.Kind == "local";
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(Row(captured.Label, captured.Kind + (attached ? " · 已连接" : ""), "terminal"));
            if (captured.Kind != "local" && model.Contexts?.ReadOnly != true)
            {
                var toggle = new Button { Content = attached ? "断开" : "连接", IsEnabled = model.CanChangeContext };
                toggle.Click += async (_, _) => { var change = model.SetContextEnabledAsync(captured.Id, !attached, lifetime.Token); Render(); await change; Render(); };
                row.Children.Add(toggle);
            }
            if (model.Contexts is { ReadOnly: false, DefaultContext: not null })
            {
                var target = captured.Kind == "local" ? "local" : captured.Id;
                var selected = model.Contexts.DefaultContext.ContextId == target;
                var choose = new Button { Content = selected ? "本会话默认" : "设为本会话默认", IsEnabled = model.CanChangeContext && !selected };
                choose.Click += async (_, _) => { var change = model.SetDefaultContextAsync(target, lifetime.Token); Render(); await change; Render(); };
                row.Children.Add(choose);
            }
            if (openTerminal is not null && attached)
            {
                var terminal = new Button { Content = "打开终端" };
                terminal.Click += (_, _) => openTerminal(captured.Id);
                row.Children.Add(terminal);
            }
            body.Children.Add(row);
        }
        if ((model.Contexts?.Contexts.Length ?? 0) == 0) body.Children.Add(Mute("没有已连接的运行环境"));
        RenderRuntimes();
        RenderExecutePanel();
        RenderRuns();
    }

    private void RenderRuntimes()
    {
        var activity = model.Activity;
        if (activity is null) return;
        body.Children.Add(TextHeading("运行时"));
        if (activity.Runtimes.Length == 0) body.Children.Add(Mute("没有已启动的解释器；执行代码会自动按环境启动。"));
        foreach (var runtime in activity.Runtimes)
        {
            var captured = runtime;
            var card = new StackPanel { Spacing = 4 };
            var detail = $"{runtime.Status}" +
                (string.IsNullOrEmpty(runtime.Interpreter) ? "" : " · " + runtime.Interpreter) +
                (runtime.ResidentMemoryBytes is { } memory ? $" · {memory / 1024 / 1024} MB" : "");
            if (!string.IsNullOrEmpty(runtime.LastError)) detail += "\n" + runtime.LastError;
            card.Children.Add(Row($"{runtime.Key.Language} · {runtime.Key.ContextId}", detail, "gauge"));
            if (!activity.ReadOnly)
            {
                var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
                var stop = new Button { Content = "停止", IsEnabled = !model.ActivityBusy };
                stop.Click += async (_, _) => { await model.StopRuntimeAsync(captured.RuntimeId, captured.Generation, lifetime.Token); Render(); };
                var restart = new Button { Content = "重启", IsEnabled = !model.ActivityBusy };
                restart.Click += async (_, _) => { await model.RestartRuntimeAsync(captured.RuntimeId, captured.Generation, lifetime.Token); Render(); };
                var dismiss = new Button { Content = "丢弃", IsEnabled = !model.ActivityBusy };
                dismiss.Click += async (_, _) => { await model.DismissRuntimeAsync(captured.RuntimeId, captured.Generation, lifetime.Token); Render(); };
                row.Children.Add(stop); row.Children.Add(restart); row.Children.Add(dismiss);
                card.Children.Add(row);
            }
            body.Children.Add(card);
        }
    }

    private void RenderExecutePanel()
    {
        var activity = model.Activity;
        if (activity is null) return;
        body.Children.Add(TextHeading("执行代码"));
        var attached = (model.Contexts?.Attached ?? []).Where(c => activity.Runtimes.Any(r => r.Key.ContextId == c.Id) || c.Kind == "local").ToArray();
        if (model.Contexts is { ReadOnly: true } || attached.Length == 0)
        {
            body.Children.Add(Mute("当前环境只读或未连接，执行面板不可用。"));
            return;
        }
        var contextPicker = new ComboBox { MaxWidth = 280, HorizontalAlignment = HorizontalAlignment.Stretch };
        foreach (var context in attached) contextPicker.Items.Add(new ComboBoxItem { Content = context.Label, Tag = context.Id });
        contextPicker.SelectedIndex = 0;
        var languagePicker = new ComboBox { MaxWidth = 120 };
        foreach (var language in new[] { "python", "r" }) languagePicker.Items.Add(new ComboBoxItem { Content = language, Tag = language });
        languagePicker.SelectedIndex = 0;
        var code = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 80,
            PlaceholderText = "输入要执行的代码…", FontFamily = design.Font(true), FontSize = design.FontSize(12, true) };
        var run = new Button { Content = "执行", IsEnabled = !model.ActivityBusy };
        run.Click += async (_, _) =>
        {
            if (contextPicker.SelectedItem is ComboBoxItem contextItem && languagePicker.SelectedItem is ComboBoxItem languageItem
                && code.Text.Trim().Length > 0)
                await model.ExecuteAsync((string)contextItem.Tag, (string)languageItem.Tag, code.Text, lifetime.Token);
            Render();
        };
        body.Children.Add(contextPicker); body.Children.Add(languagePicker); body.Children.Add(code); body.Children.Add(run);
        if (model.Execution is { } execution)
        {
            if (execution.Text.Length > 0)
                body.Children.Add(new TextBox { Text = execution.Text, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
                    FontFamily = design.Font(true), FontSize = design.FontSize(11, true), MaxHeight = 220 });
            foreach (var plot in execution.Plots.Where(File.Exists))
            {
                try
                {
                    body.Children.Add(new Border
                    {
                        Child = new Image { Source = new BitmapImage(new Uri(Path.GetFullPath(plot))), MaxHeight = 260, Stretch = Stretch.Uniform, HorizontalAlignment = HorizontalAlignment.Left },
                        CornerRadius = new CornerRadius(8), Padding = new Thickness(4), Background = design.Brush("bg-app")
                    });
                }
                catch { }
            }
        }
    }

    private void RenderRuns()
    {
        var activity = model.Activity;
        if (activity is null) return;
        body.Children.Add(TextHeading("运行记录"));
        if (activity.Runs.Length == 0) { body.Children.Add(Mute("暂无运行记录。")); return; }
        foreach (var run in activity.Runs)
        {
            var captured = run;
            var card = new StackPanel { Spacing = 4 };
            var detail = $"{NativeToolPresentation.RunState(run.Status)} · {run.Kind}" + (run.ExitCode is { } exit ? $" · 退出码 {exit}" : "");
            var rowButton = Row(run.Title, detail, "list");
            rowButton.Click += async (_, _) => await PresentRunAsync(captured.Id, false);
            card.Children.Add(rowButton);
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
            if (!activity.ReadOnly && run.Status is "running" or "paused" or "submitted")
            {
                var cancel = new Button { Content = "取消", IsEnabled = !model.ActivityBusy && !(runReview?.Mutating == true && runReview.RunId == run.Id) };
                cancel.Click += async (_, _) => { var change = model.CancelRunAsync(captured.Id, lifetime.Token); Render(); await change; Render(); };
                row.Children.Add(cancel);
            }
            if (!activity.ReadOnly && run.HarvestedAt is null && run.CleanedAt is null && run.Status is "succeeded" or "failed" or "cancelled" or "timed_out" or "lost")
            {
                var harvest = new Button { Content = "收取", IsEnabled = !model.ActivityBusy && !(runReview?.Mutating == true && runReview.RunId == run.Id) };
                harvest.Click += async (_, _) => { var change = model.HarvestRunAsync(captured.Id, lifetime.Token); Render(); await change; Render(); };
                row.Children.Add(harvest);
            }
            if (row.Children.Count > 0) card.Children.Add(row);
            body.Children.Add(card);
        }
    }

    private void RenderRunDetail()
    {
        if (model.SelectedRunId is not { } selected) return;
        body.Children.Add(TextHeading("运行详情 · " + selected));
        if (runReview?.Mutating == true) body.Children.Add(Mute("审阅操作仍在处理，详情可能尚未更新。"));
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        var refresh = new Button { Content = "刷新详情", IsEnabled = !model.RunLoading };
        refresh.Click += async (_, _) => await PresentRunAsync(selected, false);
        var dismiss = new Button { Content = "关闭详情" };
        dismiss.Click += (_, _) => { model.DismissRun(); Render(); };
        actions.Children.Add(refresh); actions.Children.Add(dismiss); body.Children.Add(actions);
        if (model.RunLoading) body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        if (model.RunError is { } error)
            body.Children.Add(new TextBlock { Text = error + (model.RunDetail != null ? "\n以下为上次读取的详情。" : ""),
                TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") });
        if (model.RunDetail is { } runDetail)
        {
            var card = new StackPanel { Spacing = 4 };
            card.Children.Add(TextHeading(runDetail.Title));
            card.Children.Add(Mute($"{NativeToolPresentation.RunState(runDetail.Status)} · {runDetail.Kind} · {runDetail.ContextId}"));
            if (runReview != null && model.Activity?.RunReviewSupported == true && runDetail.Kind == "ssh_direct"
                && runDetail.Status is "succeeded" or "failed" or "cancelled" or "timed_out" or "lost")
            {
                var review = new Button { Content = "结果与清理", IsEnabled = !model.ActivityBusy && !runReview.Mutating };
                review.Click += async (_, _) => { viewState.For("hosts").SetOffset(0); await runReview.OpenAsync(runDetail.Id, lifetime.Token); };
                card.Children.Add(review);
            }
            if (runDetail.ExitCode is { } exit) card.Children.Add(Mute($"退出码：{exit}"));
            if (!string.IsNullOrEmpty(runDetail.RemoteWorkdir)) card.Children.Add(Mute("工作目录：" + runDetail.RemoteWorkdir));
            if (!string.IsNullOrEmpty(runDetail.Command)) card.Children.Add(new TextBox { Text = runDetail.Command, IsReadOnly = true, TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(true), FontSize = design.FontSize(11, true) });
            if (!string.IsNullOrEmpty(runDetail.StdoutTail))
            {
                card.Children.Add(Mute("标准输出（末尾片段）"));
                card.Children.Add(new TextBox { Text = runDetail.StdoutTail, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(true), FontSize = design.FontSize(11, true), MaxHeight = 180 });
            }
            if (!string.IsNullOrEmpty(runDetail.StderrTail))
            {
                card.Children.Add(Mute("标准错误（末尾片段）"));
                card.Children.Add(new TextBox { Text = runDetail.StderrTail, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(true), FontSize = design.FontSize(11, true), MaxHeight = 120 });
            }
            if (string.IsNullOrEmpty(runDetail.StdoutTail) && string.IsNullOrEmpty(runDetail.StderrTail))
                card.Children.Add(Mute("本次读取暂无输出。"));
            if (!string.IsNullOrEmpty(runDetail.LastPollError)) card.Children.Add(Mute("上次状态轮询失败：" + runDetail.LastPollError));
            if (runDetail.CleanupError is { } cleanupError) card.Children.Add(Mute(cleanupError));
            body.Children.Add(card);
        }
    }

    private TextBlock TextHeading(string value) => new()
    {
        Text = value, FontSize = 12, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold,
        Foreground = design.Brush("text-muted"), Margin = new Thickness(0, 10, 0, 0)
    };

    private void RenderAgents(string query)
    {
        // Explanation card mirrors the macOS alignment: delegation is opt-in and
        // never implies auto-approval; approval actions below stay version-checked.
        var intro = new StackPanel { Spacing = 4, Padding = new Thickness(10), Background = design.Brush("bg-elev"), CornerRadius = new CornerRadius(8) };
        intro.Children.Add(new TextBlock { Text = "Agent 工作流", FontSize = 13, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold });
        intro.Children.Add(new TextBlock
        {
            Text = "工作流由会话中的 Agent 创建。需要确认的工作流必须先批准；开启委派不会自动批准任何操作。",
            FontSize = 11, Foreground = design.Brush("text-muted"), TextWrapping = TextWrapping.Wrap
        });
        body.Children.Add(intro);
        if (model.DelegationEnabled is { } delegation)
        {
            var delegationCard = new StackPanel { Spacing = 4, Padding = new Thickness(10), Background = design.Brush(delegation ? "bg-elev" : "bg-sunken"), CornerRadius = new CornerRadius(8) };
            delegationCard.Children.Add(new TextBlock
            {
                Text = delegation ? "委派已开启：会话可把子任务委派给工作流。" : "委派已关闭：会话不会创建新的委派任务。",
                FontSize = 12, TextWrapping = TextWrapping.Wrap
            });
            var toggle = new Button
            {
                Content = delegation ? "关闭委派" : "开启委派",
                IsEnabled = !model.DelegationBusy && model.Tabs.Selected == "agents"
            };
            toggle.Click += async (_, _) => { await model.SetDelegationAsync(!delegation, lifetime.Token); Render(); };
            delegationCard.Children.Add(toggle);
            body.Children.Add(delegationCard);
        }
        foreach (var agent in model.Agents.Where(item => query.Length == 0 || item.Workflow.Name.Contains(query, StringComparison.CurrentCultureIgnoreCase)))
        {
            var captured = agent;
            var card = new StackPanel { Spacing = 4 };
            card.Children.Add(Row(captured.Workflow.Name, captured.Workflow.Status + (captured.Workflow.Depth > 0 ? " · 子工作流" : ""), "sparkles"));
            if (captured.Workflow.Depth == 0)
            {
                var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
                foreach (var (label, action) in new (string, NativeAgentAction)[] { ("批准", NativeAgentAction.Approve), ("运行", NativeAgentAction.Run), ("取消", NativeAgentAction.Cancel) })
                {
                    var button = new Button { Content = label };
                    button.Click += async (_, _) => { await model.ActAgentAsync(captured, action, lifetime.Token); Render(); };
                    row.Children.Add(button);
                }
                card.Children.Add(row);
            }
            body.Children.Add(card);
        }
        if (model.Agents.Length == 0) body.Children.Add(Mute("这个会话暂无工作流"));
    }

    private void RenderNotebook(string query)
    {
        foreach (var cell in NativeNotebookCell.Collect(transcript()).Where(cell => query.Length == 0 || cell.Source.Contains(query, StringComparison.CurrentCultureIgnoreCase)))
        {
            var captured = cell;
            var card = new StackPanel { Spacing = 4 };
            card.Children.Add(Row(captured.Language, StatusLabel(cell), "book"));
            card.Children.Add(new TextBox
            {
                Text = captured.Source, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
                FontFamily = design.Font(true), FontSize = design.FontSize(11, true), MaxHeight = 160
            });
            if (captured.Output.Length > 0)
                card.Children.Add(new Expander
                {
                    Header = new TextBlock { Text = "输出", FontSize = 11, Foreground = design.Brush("text-muted") },
                    Content = new TextBox
                    {
                        Text = captured.Output, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap,
                        FontFamily = design.Font(true), FontSize = design.FontSize(11, true), MaxHeight = 220
                    },
                    IsExpanded = cell.OutputInitiallyExpanded
                });
            var star = new Button { Content = model.NotebookStars.Any(item => item.Matches(captured)) ? "取消收藏" : "收藏代码" };
            star.Click += async (_, _) => { await model.ToggleNotebookStarAsync(captured, lifetime.Token); Render(); };
            card.Children.Add(star); body.Children.Add(card);
        }
        if (!NativeNotebookCell.Collect(transcript()).Any()) body.Children.Add(Mute("暂无代码单元"));
    }

    private static string StatusLabel(NativeNotebookCell cell) => cell.Status switch
    {
        "ok" => "执行完成", "error" => "执行出错", "source" => "草稿", _ => "运行中"
    };

    private void RenderHighlights(string query)
    {
        foreach (var row in model.Highlights.Where(item => query.Length == 0 || item.Code.Contains(query, StringComparison.CurrentCultureIgnoreCase)))
        {
            var captured = row;
            var card = new StackPanel { Spacing = 4 };
            card.Children.Add(Row(captured.Title, captured.Code, "book"));
            var remove = new Button { Content = "移除" };
            remove.Click += async (_, _) => { await model.RemoveHighlightAsync(captured.Id, lifetime.Token); Render(); };
            card.Children.Add(remove); body.Children.Add(card);
        }
    }

    private void RenderSideChat()
    {
        if (sideChat is null) { body.Children.Add(Mute("侧聊尚未连接")); return; }
        foreach (var option in sideChat.Options)
        {
            var captured = option;
            var button = new Button { Content = option.Label + (sideChat.Selected?.Key == option.Key ? " · 当前" : ""), HorizontalAlignment = HorizontalAlignment.Stretch };
            button.Click += async (_, _) => { await sideChat.SelectAsync(captured, lifetime.Token); Render(); };
            body.Children.Add(button);
        }
        foreach (var row in sideChat.Rows)
        {
            body.Children.Add(new TextBlock { Text = row.Question, TextWrapping = TextWrapping.Wrap });
            body.Children.Add(new TextBlock { Text = row.Error ?? row.Answer?.Answer ?? "…", TextWrapping = TextWrapping.Wrap, Foreground = design.Brush(row.Error == null ? "text" : "clay-strong") });
        }
        var draft = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, Text = sideChat.Draft, PlaceholderText = "侧聊问题" };
        draft.TextChanged += (_, _) => sideChat.Draft = draft.Text;
        draft.KeyDown += async (_, e) =>
        {
            if (e.Key != Windows.System.VirtualKey.Enter) return;
            var shift = Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Shift).HasFlag(Microsoft.UI.Input.VirtualKeyStates.Down);
            if (NativeSideChatKeyboard.ResolveReturn(shift, false) != NativeSideChatReturnAction.Send) return;
            e.Handled = true; await sideChat.SendAsync(lifetime.Token); Render();
        };
        var send = new Button { Content = "发送", IsEnabled = sideChat.CanSend };
        send.Click += async (_, _) => { await sideChat.SendAsync(lifetime.Token); Render(); };
        body.Children.Add(draft); body.Children.Add(send);
        if (sideChat.Error is { } error) body.Children.Add(new TextBlock { Text = error, TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") });
    }

    private void RenderPreview()
    {
        if (pdfContent != model.Preview) { pdfPreview?.Dispose(); pdfPreview = null; pdfContent = null; }
        if (model.Preview is { Mime: "application/pdf" } pdf)
        {
            body.Children.Add(Mute(pdf.Path));
            if (pdf.Truncated || string.IsNullOrEmpty(pdf.Base64)) body.Children.Add(Mute("PDF 内容不完整，无法预览。请在项目文件中打开原文件。"));
            else
            {
                pdfContent = pdf;
                pdfPreview ??= new NativeRichPreview(design, "pdf", pdf.Base64, pdf.Path);
                body.Children.Add(pdfPreview);
            }
            var dismissPdf = new Button { Content = "关闭预览" };
            dismissPdf.Click += (_, _) => { model.DismissPreview(); Render(); };
            body.Children.Add(dismissPdf);
            return;
        }
        if (model.Preview?.Text is { } text)
        {
            body.Children.Add(new TextBlock { Text = model.Preview.Path, FontSize = 12, Foreground = design.Brush("text-muted") });
            var editor = new TextBox { Text = text, IsReadOnly = !model.PreviewEditable, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 120 };
            body.Children.Add(editor);
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            if (!model.PreviewEditable)
            {
                var edit = new Button { Content = "编辑" }; edit.Click += (_, _) => { model.EditPreview(); Render(); };
                row.Children.Add(edit);
            }
            else
            {
                var save = new Button { Content = "保存" };
                save.Click += async (_, _) =>
                {
                    try { await model.SaveFileAsync(text, editor.Text, lifetime.Token); Render(); }
                    catch (Exception ex) { await ShowErrorAsync("保存未确认成功，不会自动重试。\n" + ex.Message); }
                };
                row.Children.Add(save);
            }
            var dismiss = new Button { Content = "关闭预览" }; dismiss.Click += (_, _) => { model.DismissPreview(); Render(); };
            row.Children.Add(dismiss); body.Children.Add(row);
            return;
        }
        // Read-only image preview for artifacts/files the host returned as base64 bytes.
        var content = model.Preview;
        if (content?.Base64 is not { Length: > 0 } || content.Mime?.StartsWith("image/", StringComparison.Ordinal) != true) return;
        var key = content.Path + "#" + content.TotalBytes;
        if (previewImages.TryGetValue(key, out var loaded)) body.Children.Add(loaded);
        else if (previewLoading.Add(key)) _ = LoadPreviewImageAsync(key, content);
    }

    private async Task LoadPreviewImageAsync(string key, NativePanelFileContent content)
    {
        try
        {
            var bytes = Convert.FromBase64String(content.Base64 ?? "");
            if (bytes.Length > 0)
            {
                var source = new BitmapImage();
                using (var stream = new Windows.Storage.Streams.InMemoryRandomAccessStream())
                using (var writer = new Windows.Storage.Streams.DataWriter(stream.GetOutputStreamAt(0)))
                {
                    writer.WriteBytes(bytes);
                    await writer.StoreAsync();
                    await writer.FlushAsync();
                    stream.Seek(0);
                    await source.SetSourceAsync(stream);
                }
                var stack = new StackPanel { Spacing = 4 };
                stack.Children.Add(new TextBlock { Text = content.Path, FontSize = 12, Foreground = design.Brush("text-muted"), TextWrapping = TextWrapping.Wrap });
                stack.Children.Add(new Border
                {
                    Child = new Image { Source = source, MaxHeight = 300, Stretch = Stretch.Uniform, HorizontalAlignment = HorizontalAlignment.Left },
                    CornerRadius = new CornerRadius(8), Padding = new Thickness(4), Background = design.Brush("bg-app")
                });
                if (content.Truncated) stack.Children.Add(Mute("内容被截断；预览可能不完整。"));
                var dismiss = new Button { Content = "关闭预览" }; dismiss.Click += (_, _) => { model.DismissPreview(); Render(); };
                stack.Children.Add(dismiss);
                previewImages[key] = stack;
            }
        }
        catch { } // Broken or non-image bytes fall back to no preview; rows stay selectable.
        finally { previewLoading.Remove(key); if (!disposed) Render(); }
    }

    private Button Row(string title, string detail, string icon)
    {
        var content = new StackPanel { Spacing = 2 };
        var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        heading.Children.Add(design.Icon(icon, 14));
        heading.Children.Add(new TextBlock { Text = title, TextWrapping = TextWrapping.Wrap });
        content.Children.Add(heading);
        content.Children.Add(new TextBlock { Text = detail, FontSize = 11, Foreground = design.Brush("text-muted"), TextWrapping = TextWrapping.Wrap });
        return new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left, Padding = new Thickness(8) };
    }
    private TextBlock Mute(string text) => new() { Text = text, Foreground = design.Brush("text-muted"), FontSize = 12, TextWrapping = TextWrapping.Wrap };
    private static string Label(string id) => id switch
    {
        "artifacts" => "产物", "files" => "文件", "hosts" => "环境", "agents" => "工作流",
        "notebook" => "笔记本", "highlights" => "摘录", "provenance" => "溯源", "sidechat" => "侧聊", _ => id
    };
    public void Dispose()
    {
        if (disposed) return;
        scroll.LayoutUpdated -= RestoreScroll;
        if (runReview != null) { runReview.Changed -= Render; runReview.Close(); }
        disposed = true; lifetime.Cancel(); pdfPreview?.Dispose(); model.Close(); lifetime.Dispose();
    }
}
