using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel
{
    private readonly Action? manageEnvironments;
    private readonly Func<string, Button, bool, Task>? openContextRuntimes;
    private string attachQuery = "", selectedMachine = "";
    private Popup? contextDialog;
    private ComboBox? runtimeLanguage;
    private enum ContextView { Machine, Runs }

    private void RenderHosts()
    {
        var attached = model.Contexts?.Attached ?? [];
        body.Children.Add(ContextHeading(design.T("contexts.execution"), attached.Length));
        if (model.ContextUncertain || model.HostListError != null || model.Error != null)
        {
            if (model.HostListError is { } error) body.Children.Add(Mute(error));
            var retry = new Button { Content = design.L("重新读取环境", "Reload environments"), IsEnabled = !model.Loading && !model.ContextBusy };
            design.ActionButton(retry); retry.Click += async (_, _) => await ShowTabAsync("hosts"); body.Children.Add(retry);
        }
        if (model.Contexts != null && attached.Length == 0) body.Children.Add(ContextEmpty("contexts.empty"));
        foreach (var ctx in attached) body.Children.Add(ContextCard(ctx));
        body.Children.Add(new Border { Height = 1, Margin = new(0, 4, 0, 2), Background = design.Brush("border") });
        body.Children.Add(ContextHeading(design.T("contexts.attach")));
        var search = new TextBox { Text = attachQuery, PlaceholderText = design.T("compute.search"), Padding = new(32, 5, 8, 5),
            CornerRadius = new(6), HorizontalAlignment = HorizontalAlignment.Stretch };
        AutomationProperties.SetName(search, search.PlaceholderText);
        var field = new Grid(); field.Children.Add(search); var icon = design.Icon("search", 15);
        icon.Margin = new(10, 0, 0, 0); icon.HorizontalAlignment = HorizontalAlignment.Left; icon.IsHitTestVisible = false;
        field.Children.Add(icon); body.Children.Add(field);
        var rows = new StackPanel { Spacing = 4 };
        body.Children.Add(new ScrollViewer { MaxHeight = 220, Content = rows, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled });
        void UpdateChoices()
        {
            rows.Children.Clear();
            var choices = NativeComputeMenu.AttachCandidates(model.Contexts, model.HostAliases, attachQuery);
            if (choices.Length == 0 && model.Contexts != null && model.HostListError == null) rows.Children.Add(ContextEmpty("contexts.attach_empty"));
            foreach (var option in choices)
            {
                var row = new Grid { ColumnSpacing = 10 }; row.ColumnDefinitions.Add(new() { Width = new(22) });
                row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                row.Children.Add(design.Icon(option.Icon, 15));
                var name = design.Text(option.Name, 12.5); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis;
                Grid.SetColumn(name, 1); row.Children.Add(name);
                var action = design.Text(design.T("contexts.attach_action"), 11); action.Foreground = design.Brush("clay-strong");
                action.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(action, 2); row.Children.Add(action);
                var button = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                    IsEnabled = model.CanChangeContext, MinHeight = 36, Padding = new(9, 6, 9, 6), CornerRadius = new(6), Background = design.Brush("bg-elev") };
                AutomationProperties.SetName(button, design.T("contexts.attach_action") + ": " + option.Name);
                button.Click += async (_, _) => await ChangeHostAsync(model.SetContextEnabledAsync(option.Id, true, lifetime.Token));
                rows.Children.Add(button);
            }
        }
        search.TextChanged += (_, _) => { attachQuery = search.Text; UpdateChoices(); };
        UpdateChoices();
        if (manageEnvironments != null)
        {
            var manage = new Button { Content = design.T("compute.manage"), HorizontalAlignment = HorizontalAlignment.Stretch };
            design.ActionButton(manage); manage.Foreground = design.Brush("clay-strong"); manage.Click += (_, _) => manageEnvironments(); body.Children.Add(manage);
        }
    }

    private Grid ContextHeading(string text, int? count = null)
    {
        var grid = new Grid { ColumnSpacing = 8 }; grid.ColumnDefinitions.Add(new()); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        grid.Children.Add(RunGroupLabel(text));
        if (count.HasValue) { var value = design.Text(count.Value.ToString(), 12); value.Foreground = design.Brush("text-faint"); Grid.SetColumn(value, 1); grid.Children.Add(value); }
        return grid;
    }
    private Grid ContextEmpty(string key, bool elevated = false)
    {
        var box = new Grid { Background = design.Brush(elevated ? "bg-elev" : "bg-sunken"), CornerRadius = new(8) };
        var text = design.Text(design.T(key), 12.5); text.Foreground = design.Brush("text-faint"); text.Margin = new(12); box.Children.Add(text);
        box.Children.Add(new Microsoft.UI.Xaml.Shapes.Rectangle { RadiusX = 8, RadiusY = 8, Stroke = design.Brush("border-strong"),
            StrokeThickness = 1, StrokeDashArray = new DoubleCollection { 3, 2 }, IsHitTestVisible = false });
        return box;
    }
    private Grid ContextIdStatus(NativePanelContext ctx)
    {
        var grid = new Grid { ColumnSpacing = 8 }; grid.ColumnDefinitions.Add(new()); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var id = design.Text(ctx.Id, 13); id.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; id.TextWrapping = TextWrapping.NoWrap;
        id.TextTrimming = TextTrimming.CharacterEllipsis; grid.Children.Add(id);
        var status = design.Text((ctx.LastProbeStatus ?? "unknown").ToUpperInvariant(), 10); status.FontWeight = Microsoft.UI.Text.FontWeights.Bold;
        status.Foreground = design.Brush(ctx.LastProbeStatus switch {
            "ok" or "success" => "ok", "error" or "failed" => "err", _ => "text-muted" });
        var badge = new Border { Child = status, Background = design.Brush("bg-sunken"), Padding = new(7, 2, 7, 2), CornerRadius = new(20) };
        Grid.SetColumn(badge, 1); grid.Children.Add(badge); return grid;
    }
    private Border ContextCard(NativePanelContext ctx)
    {
        var stack = new StackPanel { Spacing = 8 }; var info = new StackPanel { Spacing = 3 };
        info.Children.Add(ContextIdStatus(ctx)); var name = design.Text(NativeComputeMenu.Label(ctx), 11.5);
        name.Foreground = design.Brush("text-muted"); info.Children.Add(name);
        var meta = design.Text(ctx.Kind + " · " + NativeComputeMenu.CapabilitySummary(ctx), 11.5); meta.FontFamily = design.Font(true);
        meta.Foreground = design.Brush("text-muted"); info.Children.Add(meta);
        if (!string.IsNullOrEmpty(ctx.LastProbeError)) { var error = design.Text(ctx.LastProbeError, 12); error.Foreground = design.Brush("err"); info.Children.Add(error); }
        var machine = new Button { Content = info, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        design.QuietButton(machine); machine.Padding = new(0); AutomationProperties.SetName(machine, design.T("contexts.machine_info") + ": " + NativeComputeMenu.Label(ctx));
        machine.Click += async (_, _) => await ShowContextDetailAsync(ctx, ContextView.Machine); stack.Children.Add(machine);
        stack.Children.Add(new Border { Height = 1, Background = design.Brush("border") });
        var actions = new NativeActionWrap { Gap = 6, SpaceBetween = true };
        var tools = new NativeActionWrap { Gap = 6 }; actions.Children.Add(tools);
        void Tool(string key, string glyph, Func<Button, Task> act, bool enabled = true)
        {
            var b = design.ToolButton(design.T(key), glyph); b.Content = design.Icon(glyph, 14); b.IsEnabled = enabled;
            b.Width = b.Height = b.MinWidth = b.MinHeight = 26; b.Padding = new(4); b.CornerRadius = new(5);
            b.Background = design.Brush("bg-sunken"); b.BorderBrush = design.Brush("border"); b.BorderThickness = new(1);
            b.Click += async (_, _) => await act(b); tools.Children.Add(b);
        }
        if (openContextRuntimes != null) Tool("contexts.view_runtimes", "runtime-panel", b => openContextRuntimes(ctx.Id, b, false));
        Tool("contexts.view_runs", "list", _ => ShowContextDetailAsync(ctx, ContextView.Runs));
        tools = new NativeActionWrap { Gap = 6 }; actions.Children.Add(tools);
        if (openContextRuntimes != null) Tool("contexts.configure_interpreters", "edit", b => openContextRuntimes(ctx.Id, b, true), model.CanManageHosts);
        Tool("contexts.probe", "sync", _ => ChangeHostAsync(model.ProbeAttachedAsync(ctx.Id, lifetime.Token)), model.CanManageHosts);
        Tool("contexts.open_terminal", "terminal", _ => { openTerminal?.Invoke(ctx.Id); return Task.CompletedTask; }, openTerminal != null);
        if (ctx.Kind != "local") Tool("contexts.detach", "minus", _ => ChangeHostAsync(model.SetContextEnabledAsync(ctx.Id, false, lifetime.Token)), model.CanChangeContext);
        stack.Children.Add(actions);
        return new Border { Child = stack, Tag = ctx, Padding = new(12, 10, 12, 10), CornerRadius = new(8), Background = design.Brush("bg-elev"),
            BorderThickness = new(1), BorderBrush = design.Brush(selectedMachine == ctx.Id ? "clay" : "border-strong") };
    }
    private async Task ChangeHostAsync(Task change)
    {
        Render(); await change; if (!disposed && model.Tabs.Selected == "hosts") Render();
    }

    private async Task ShowContextDetailAsync(NativePanelContext ctx, ContextView view)
    {
        if (disposed || XamlRoot is not { } root || contextDialog != null) return;
        selectedMachine = ctx.Id;
        foreach (var b in body.Children.OfType<Border>()) if (b.Tag is NativePanelContext c) b.BorderBrush = design.Brush(c.Id == ctx.Id ? "clay" : "border-strong");
        var title = design.T(view == ContextView.Machine ? "contexts.machine_info" : "contexts.runs");
        var content = new StackPanel { Spacing = 10 };
        var header = new Grid(); header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var titles = new StackPanel { Spacing = 3 }; var heading = design.Text(title, 18); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var subtitle = design.Text(NativeComputeMenu.Label(ctx), 11); subtitle.FontFamily = design.Font(true); subtitle.Foreground = design.Brush("text-faint");
        titles.Children.Add(heading); titles.Children.Add(subtitle); header.Children.Add(titles);
        var close = design.ToolButton(design.T("contexts.close_details"), "close"); Grid.SetColumn(close, 1); header.Children.Add(close);
        var layout = new Grid { RowSpacing = 16, HorizontalAlignment = HorizontalAlignment.Stretch };
        layout.RowDefinitions.Add(new() { Height = GridLength.Auto }); layout.RowDefinitions.Add(new() { Height = GridLength.Auto });
        layout.Children.Add(header);
        var scroller = new ScrollViewer { Content = content, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        Grid.SetRow(scroller, 1); layout.Children.Add(scroller);
        var overlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
        NativeDialogVisuals.Backdrop(overlay, design); AutomationProperties.SetName(overlay, title);
        var card = new Border { Child = layout, Padding = new(22), CornerRadius = new(16), Background = design.Brush("bg-elev"),
            BorderBrush = design.Brush("border"), BorderThickness = new(1), TabFocusNavigation = KeyboardNavigationMode.Cycle,
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        var runs = view == ContextView.Runs;
        if (runs)
        {
            card.Padding = new(0); layout.RowSpacing = 0;
            layout.RowDefinitions[1].Height = new(1, GridUnitType.Star);
            header.Padding = new(18, 16, 18, 12); header.BorderBrush = design.Brush("border"); header.BorderThickness = new(0, 0, 0, 1);
            content.Margin = new(14, 12, 14, 16); scroller.Background = design.Brush("bg-sunken");
        }
        var trigger = FocusManager.GetFocusedElement(root) as Control;
        overlay.Children.Add(card); var dialog = contextDialog = new Popup { XamlRoot = root, Child = overlay, IsLightDismissEnabled = false };
        var dismissed = new TaskCompletionSource(); dialog.Closed += (_, _) => dismissed.TrySetResult();
        close.Click += (_, _) => dialog.IsOpen = false;
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, e) => { if (contextDialog == dialog) e.Handled = HandleEscape(); };
        overlay.KeyboardAccelerators.Add(escape); overlay.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        void Resize(XamlRoot sender, XamlRootChangedEventArgs? e)
        {
            var width = Math.Clamp(sender.Size.Width - (runs ? 48 : 40), 1, runs ? 1040 : 480);
            overlay.Width = sender.Size.Width; overlay.Height = sender.Size.Height; card.Width = width;
            if (runs)
            {
                var available = Math.Max(1, sender.Size.Height - 40);
                var wanted = header.ActualHeight + content.ActualHeight + content.Margin.Top + content.Margin.Bottom + 2;
                card.Height = Math.Clamp(wanted, Math.Min(620, available), available);
            }
            else scroller.MaxHeight = Math.Max(80, sender.Size.Height - 180);
        }
        if (runs) content.SizeChanged += (_, _) => Resize(root, null);
        Resize(root, null); root.Changed += Resize;
        try
        {
            if (view == ContextView.Machine)
            {
                content.Children.Add(new Border { Child = ContextIdStatus(ctx), Padding = new(12, 10, 12, 10), CornerRadius = new(8),
                    Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1) });
                var fields = new StackPanel();
                void Field(string key, string value)
                {
                    var row = new Grid { ColumnSpacing = 16, Padding = new(12, 10, 12, 10) };
                    row.ColumnDefinitions.Add(new() { Width = new(.35, GridUnitType.Star), MinWidth = 80 }); row.ColumnDefinitions.Add(new());
                    var caption = design.Text(design.T(key), 11.5); caption.Foreground = design.Brush("text-faint"); row.Children.Add(caption);
                    var data = design.Text(value, 12); data.FontFamily = design.Font(true); Grid.SetColumn(data, 1); row.Children.Add(data); fields.Children.Add(row);
                }
                Field("contexts.kind", ctx.Kind); fields.Children.Add(new Border { Height = 1, Background = design.Brush("border") });
                Field("contexts.capabilities", NativeComputeMenu.CapabilitySummary(ctx));
                content.Children.Add(new Border { Child = fields, BorderThickness = new(1), BorderBrush = design.Brush("border"), CornerRadius = new(8) });
                if (ctx.LastProbeError is { Length: > 0 } error) { var message = design.Text(error, 12); message.Foreground = design.Brush("err"); content.Children.Add(message); }
                if (model.Contexts is { ReadOnly: false, DefaultContext: { } def })
                {
                    var id = ctx.Kind == "local" ? "local" : ctx.Id;
                    var choose = new Button { Content = def.ContextId == id ? design.L("本会话默认环境", "Session default environment") :
                        design.L("设为本会话默认环境", "Set as session default"), IsEnabled = model.CanChangeContext && def.ContextId != id };
                    design.ActionButton(choose); choose.Click += async (_, _) =>
                    {
                        choose.IsEnabled = false; await ChangeHostAsync(model.SetDefaultContextAsync(id, lifetime.Token));
                        if (contextDialog != dialog) return;
                        if (model.Error is { } message) content.Children.Add(Mute(message));
                        else if (model.Contexts?.DefaultContext?.ContextId == id) choose.Content = design.L("本会话默认环境", "Session default environment");
                    }; content.Children.Add(choose);
                }
                if (model.Contexts?.ReadOnly == false && model.Activity is { ReadOnly: false } activity &&
                    (ctx.Kind == "local" || activity.Runtimes.Any(r => r.Key.ContextId == ctx.Id)))
                    content.Children.Add(ContextConsole(ctx, dialog));
            }
            else
            {
                MountContextRuns(content, ctx.Id, dialog);
            }
            dialog.IsOpen = true; close.Focus(FocusState.Programmatic); await dismissed.Task;
        }
        finally
        {
            root.Changed -= Resize; dialog.Child = null;
            if (contextDialog == dialog) { contextDialog = null; runtimeLanguage = null; }
            if (!disposed && trigger?.IsLoaded == true) trigger.Focus(FocusState.Programmatic);
        }
    }

    private Expander ContextConsole(NativePanelContext ctx, Popup dialog)
    {
        var stack = new StackPanel { Spacing = 8 };
        var picker = runtimeLanguage = new() { ItemsSource = new[] { "Python", "R" }, SelectedIndex = 0 };
        stack.Children.Add(picker);
        var code = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 80,
            PlaceholderText = design.T("runtime.console_input"), FontFamily = design.Font(true) }; stack.Children.Add(code);
        var execute = new Button { Content = design.T("runtime.console_run"), IsEnabled = false };
        design.ActionButton(execute); stack.Children.Add(execute);
        var output = new StackPanel { Spacing = 4 }; stack.Children.Add(output);
        code.TextChanged += (_, _) => execute.IsEnabled = !model.ActivityBusy && !string.IsNullOrWhiteSpace(code.Text);
        execute.Click += async (_, _) =>
        {
            if (model.ActivityBusy || string.IsNullOrWhiteSpace(code.Text)) return;
            execute.IsEnabled = false; var previous = model.Execution;
            await model.ExecuteAsync(ctx.Id, picker.SelectedIndex == 0 ? "python" : "r", code.Text, lifetime.Token);
            if (disposed || contextDialog != dialog) return;
            output.Children.Clear();
            if (model.Error is { } error) output.Children.Add(Mute(error));
            if (!ReferenceEquals(previous, model.Execution) && model.Execution is { } result)
            {
                if (result.Text.Length > 0) { var text = design.Text(result.Text, 12); text.FontFamily = design.Font(true); text.IsTextSelectionEnabled = true; output.Children.Add(text); }
                foreach (var plot in result.Plots.Where(File.Exists)) output.Children.Add(new Image { Source = new BitmapImage(new Uri(Path.GetFullPath(plot))),
                    MaxHeight = 220, Stretch = Stretch.Uniform, HorizontalAlignment = HorizontalAlignment.Left });
            }
            execute.IsEnabled = !string.IsNullOrWhiteSpace(code.Text);
        };
        return new Expander { Header = design.T("runtime.console"), Content = stack, HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch };
    }
}
