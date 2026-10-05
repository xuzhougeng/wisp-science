using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly StackPanel queueRows = new() { Spacing = 6 };
    private readonly ScrollViewer queueScroll = new() { MaxHeight = 80, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    private readonly StackPanel queueCard = new() { Spacing = 4, Visibility = Visibility.Collapsed };
    private readonly Button queueHeader = new() { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private bool queueExpanded = true;
    private MenuFlyout? queueMenu;
    private ContentDialog? queueDialog;
    private NativeQueueTarget? queueOwner;
    private string? queueFingerprint;
    public bool HandleQueueEscape()
    {
        if (queueMenu != null) { queueMenu.Hide(); return true; }
        if (queueDialog != null) { queueDialog.Hide(); return true; }
        return false;
    }
    private void InitializeQueue()
    {
        queueScroll.Content = queueRows;
        design.QuietButton(queueHeader);
        queueHeader.MinHeight = 28; queueHeader.Padding = new Thickness(4, 2, 4, 2);
        queueHeader.Click += (_, _) => { queueExpanded = !queueExpanded; UpdateQueueHeader(); };
        queueCard.Children.Add(queueHeader); queueCard.Children.Add(queueScroll);
        SizeChanged += (_, _) => queueScroll.MaxHeight = ActualHeight < 650 ? 80 : 160;
    }
    private void UpdateQueueHeader()
    {
        var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        content.Children.Add(design.Icon(queueExpanded ? "chevron-down" : "chevron-right", 14));
        content.Children.Add(new TextBlock { Text = $"排队消息 · {model.QueuedTurns.Length}", FontSize = design.FontSize(12) });
        queueHeader.Content = content;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(queueHeader, $"{(queueExpanded ? "收起" : "展开")}排队消息 · {model.QueuedTurns.Length}");
        queueScroll.Visibility = queueExpanded ? Visibility.Visible : Visibility.Collapsed;
    }
    private void RenderQueue()
    {
        if (queueOwner != null && (model.Snapshot?.SessionId != queueOwner.SessionId || model.Snapshot?.ProjectId != queueOwner.ProjectId))
        { queueMenu?.Hide(); queueDialog?.Hide(); }
        var snapshot = model.Snapshot;
        var recoveries = model.QueueRecoveries;
        var failed = snapshot?.Queue?.Outcomes.Where(outcome => outcome.State == "failed" && !recoveries.Any(row => row.Id == outcome.Id)).TakeLast(3).ToArray() ?? [];
        var fingerprint = JsonSerializer.Serialize(snapshot?.Queue) + JsonSerializer.Serialize(recoveries)
            + $"/{snapshot?.SessionId}/{snapshot?.Running}/{snapshot?.ReadOnly}/{model.Busy}/{model.QueueUncertain}/{model.ConnectionError}/{model.ShowingHistory}/{model.CanRecoverQueue}";
        if (queueFingerprint == fingerprint) return;
        queueFingerprint = fingerprint;
        UpdateQueueHeader();
        queueCard.Visibility = model.QueuedTurns.Length > 0 || recoveries.Length > 0 || failed.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        queueRows.Children.Clear();
        foreach (var item in model.QueuedTurns)
        {
            if (model.QueueTarget(item) is not { } target) continue;
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var body = new StackPanel { Spacing = 3 };
            body.Children.Add(new TextBlock { Text = item.State == "cutin_pending" ? "已提交 · 等待当前步骤结束" : "等待执行", FontSize = design.FontSize(11), Foreground = design.Brush("text-muted") });
            body.Children.Add(new TextBlock { Text = item.EditText, MaxLines = 2, TextWrapping = TextWrapping.Wrap, TextTrimming = TextTrimming.CharacterEllipsis });
            if (item.Attachments.Length + item.References.Length > 0) body.Children.Add(new TextBlock {
                Text = $"附件 {item.Attachments.Length} · 引用 {item.References.Length}", FontSize = design.FontSize(11), Foreground = design.Brush("text-muted") });
            row.Children.Add(body);
            var actions = MessageAction("队列操作"); actions.IsEnabled = !model.Busy && !model.QueueUncertain && item.State == "queued";
            actions.Click += (_, _) => ShowQueueMenu(actions, target);
            Grid.SetColumn(actions, 1); row.Children.Add(actions);
            queueRows.Children.Add(row);
        }
        foreach (var recovery in recoveries)
        {
            var text = new TextBlock { Text = recovery.Reason, TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") };
            var restore = new Button { Content = "恢复排队草稿", IsEnabled = model.CanRecoverQueue };
            restore.Click += (_, _) => model.RestoreQueueDraft(recovery.Id);
            queueRows.Children.Add(text); queueRows.Children.Add(restore);
        }
        foreach (var outcome in failed) queueRows.Children.Add(new TextBlock {
            Text = $"排队消息 {outcome.Id} 执行失败，请检查会话。", TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") });
    }
    private void ShowQueueMenu(Button owner, NativeQueueTarget target)
    {
        queueMenu?.Hide();
        var menu = new MenuFlyout(); queueMenu = menu; queueOwner = target;
        foreach (var (label, kind) in new[] { ("编辑…", "edit"), ("取消排队", "cancel"), ("插入当前轮", "cut_in"),
            ("中断当前轮并优先执行", "replace"), ("上移", "move_up"), ("下移", "move_down") })
        {
            var entry = new MenuFlyoutItem { Text = label, IsEnabled = model.CanQueueAction(target, kind) };
            entry.Click += async (_, _) =>
            {
                if (kind == "edit") await EditQueuedTurn(target);
                else await model.QueueActionAsync(target, new() { ["kind"] = kind }, lifetime.Token);
            };
            menu.Items.Add(entry);
        }
        menu.Closed += (_, _) => { if (queueMenu == menu) queueMenu = null; };
        menu.ShowAt(owner);
    }
    private async Task EditQueuedTurn(NativeQueueTarget target)
    {
        if (!model.CanQueueAction(target, "edit") || disposed || queueDialog != null || historyDialog != null || openingRunReview) return;
        var editor = new TextBox { Text = target.Item.EditText, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 140, MaxHeight = 250 };
        var error = new TextBlock { TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("clay-strong") };
        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(new TextBlock { Text = "保存后按新内容执行。如果它已开始，修改会被拒绝。附件和引用保留。", TextWrapping = TextWrapping.Wrap });
        body.Children.Add(editor); body.Children.Add(error);
        var dialog = new ContentDialog { Title = "编辑排队消息", XamlRoot = XamlRoot, PrimaryButtonText = "保存", CloseButtonText = "取消",
            DefaultButton = ContentDialogButton.Close, RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light,
            Content = new ScrollViewer { Content = body, MaxHeight = 380 } };
        queueDialog = dialog; queueOwner = target;
        dialog.PrimaryButtonClick += async (_, args) =>
        {
            var deferral = args.GetDeferral();
            try
            {
                if (string.IsNullOrWhiteSpace(editor.Text) && target.Item.Attachments.Length == 0 && target.Item.References.Length == 0)
                { args.Cancel = true; error.Text = "请填写消息内容。"; return; }
                var saved = await model.QueueActionAsync(target, new() { ["kind"] = "edit", ["message"] = editor.Text }, lifetime.Token);
                args.Cancel = !saved;
                if (!saved) { error.Text = model.OperationError ?? "这条消息已改变或开始执行，请关闭并检查队列。"; dialog.IsPrimaryButtonEnabled = false; }
            }
            finally { deferral.Complete(); }
        };
        try { await dialog.ShowAsync(); }
        finally { if (queueDialog == dialog) queueDialog = null; }
    }
}
