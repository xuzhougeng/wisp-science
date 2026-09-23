using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeWorkspacePanel : UserControl, IDisposable
{
    private readonly WorkspacePanelModel model;
    private readonly WispDesign design;
    private readonly Func<ConversationItem[]> transcript;
    private readonly WorkspaceSideChatModel? sideChat;
    private readonly Action<string>? openTerminal;
    private readonly Action close;
    private readonly TextBox filter = new() { PlaceholderText = "筛选名称" };
    private readonly StackPanel body = new() { Spacing = 8 };
    private readonly CancellationTokenSource lifetime = new();
    private bool disposed;
    public async Task ShowFilesAsync()
    {
        try { await model.RefreshAsync("files", cancellationToken: lifetime.Token); Render(); }
        catch (OperationCanceledException) { }
    }

    public NativeWorkspacePanel(WorkspacePanelModel model, Func<ConversationItem[]> transcript, WispDesign design, Action close,
        WorkspaceSideChatModel? sideChat = null, Action<string>? openTerminal = null)
    {
        this.model = model; this.transcript = transcript; this.design = design; this.close = close;
        this.sideChat = sideChat; this.openTerminal = openTerminal;
        var root = new Grid { Width = 320, Padding = new Thickness(12), Background = design.Brush("bg-sunken") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var header = new Grid();
        header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var tabs = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        foreach (var id in model.Tabs.Available)
        {
            var captured = id;
            var tab = new Button { Content = Label(id), Padding = new Thickness(8, 4, 8, 4) };
            tab.Click += async (_, _) => { await model.RefreshAsync(captured, cancellationToken: lifetime.Token); Render(); };
            tabs.Children.Add(tab);
        }
        header.Children.Add(tabs);
        var dismiss = new Button { Content = design.Icon("close", 14), Padding = new Thickness(6) };
        dismiss.Click += (_, _) => close();
        ToolTipService.SetToolTip(dismiss, "关闭面板");
        Grid.SetColumn(dismiss, 1); header.Children.Add(dismiss);
        root.Children.Add(header);
        filter.TextChanged += (_, _) => Render();
        Grid.SetRow(filter, 1); root.Children.Add(filter);
        var scroll = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
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

    private void Render()
    {
        if (disposed) return;
        body.Children.Clear();
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
    }

    private void RenderArtifacts(string query)
    {
        foreach (var artifact in model.Artifacts.Where(item => query.Length == 0 || item.Name.Contains(query, StringComparison.CurrentCultureIgnoreCase)))
        {
            var captured = artifact;
            var button = Row(captured.Name, captured.Kind + " · " + (captured.LogicalPath ?? captured.Path), "doc");
            button.Click += async (_, _) => { await model.ReadArtifactAsync(captured.Id, lifetime.Token); Render(); };
            body.Children.Add(button);
        }
        if (model.Artifacts.Length == 0 && !model.Loading) body.Children.Add(Mute("这个会话暂无产物"));
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
        foreach (var context in model.Contexts?.Contexts ?? [])
        {
            var captured = context;
            var attached = model.Contexts!.EnabledIds.Contains(captured.Id) || captured.Kind == "local";
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(Row(captured.Label, captured.Kind + (attached ? " · 已连接" : ""), "terminal"));
            if (captured.Kind != "local" && model.Contexts?.ReadOnly != true)
            {
                var toggle = new Button { Content = attached ? "断开" : "连接", IsEnabled = !model.ContextBusy };
                toggle.Click += async (_, _) => { await model.SetContextEnabledAsync(captured.Id, !attached, lifetime.Token); Render(); };
                row.Children.Add(toggle);
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
    }

    private void RenderAgents(string query)
    {
        foreach (var agent in model.Agents.Where(item => query.Length == 0 || item.Workflow.Name.Contains(query, StringComparison.CurrentCultureIgnoreCase)))
        {
            var captured = agent;
            var card = new StackPanel { Spacing = 4 };
            card.Children.Add(Row(captured.Workflow.Name, captured.Workflow.Status, "sparkles"));
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
            var row = new StackPanel { Spacing = 4 };
            row.Children.Add(Row(captured.Language, captured.Source, "book"));
            var star = new Button { Content = model.NotebookStars.Any(item => item.Matches(captured)) ? "取消收藏" : "收藏代码" };
            star.Click += async (_, _) => { await model.ToggleNotebookStarAsync(captured, lifetime.Token); Render(); };
            row.Children.Add(star); body.Children.Add(row);
        }
    }

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
        if (model.Preview?.Text is not { } text) return;
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
        disposed = true; lifetime.Cancel(); model.Close(); lifetime.Dispose();
    }
}
