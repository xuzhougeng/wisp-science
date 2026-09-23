using System.Text.Json.Nodes;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.System;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeConversationPage : UserControl, IDisposable
{
    private readonly WorkspaceConversationModel model;
    private readonly WispDesign design;
    private readonly Action<string> quote;
    private readonly Func<Task> create;
    private readonly StackPanel transcript = new() { Spacing = 18, MaxWidth = 850, Margin = new Thickness(24) };
    private readonly StackPanel approvals = new() { Spacing = 12, MaxWidth = 850, Margin = new Thickness(24, 0, 24, 12) };
    private readonly TextBox composer = new() { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 58, PlaceholderText = "向 Wisp Science 提问…" };
    private readonly ComboBox models = new() { MaxWidth = 230 };
    private readonly Button send = new() { Content = "发送" };
    private readonly Button stop = new() { Content = "停止" };
    private readonly Button createSession = new() { Content = "新建会话" };
    private readonly Button attach = new() { Content = "对话附件" };
    private readonly Button queue = new() { Content = "排队后续" };
    private readonly StackPanel attachments = new() { Spacing = 4 };
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap, FontSize = 12 };
    private readonly TextBlock hint = new() { FontSize = 11 };
    private readonly Grid composerBar = new();
    private readonly ScrollViewer scroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    private readonly CancellationTokenSource lifetime = new();
    private bool disposed, followLatest = true;

    public NativeConversationPage(WorkspaceConversationModel model, WispDesign design, Action<string> quote, Func<Task> create, Func<Task<string?>>? pickAttachment = null)
    {
        this.model = model; this.design = design; this.quote = quote; this.create = create;
        model.Changed += Refresh;
        var root = new Grid();
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        status.Margin = new Thickness(24, 12, 24, 0);
        var retry = new Button { Content = "重新读取" };
        retry.Click += async (_, _) => await model.RefreshAsync(lifetime.Token);
        var ack = new Button { Content = "已检查，允许再次发送或排队…" };
        ack.Click += (_, _) => model.AcknowledgeUncertainSend();
        var banner = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Margin = new Thickness(24, 8, 24, 0) };
        banner.Children.Add(retry); banner.Children.Add(ack);
        var header = new StackPanel(); header.Children.Add(status); header.Children.Add(banner);
        root.Children.Add(header);
        scroll.Content = transcript; Grid.SetRow(scroll, 1); root.Children.Add(scroll);
        Grid.SetRow(approvals, 2); root.Children.Add(approvals);
        composer.TextChanged += (_, _) => { model.Draft = composer.Text; send.IsEnabled = model.CanSend; queue.IsEnabled = model.CanQueue; };
        composer.KeyDown += (_, e) =>
        {
            if (e.Key != VirtualKey.Enter) return;
            var shift = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift).HasFlag(VirtualKeyStates.Down);
            var control = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control).HasFlag(VirtualKeyStates.Down);
            if (control && !shift) { e.Handled = true; _ = model.SendAsync(lifetime.Token); }
        };
        send.Click += async (_, _) => await model.SendAsync(lifetime.Token);
        stop.Click += async (_, _) => await model.StopAsync(lifetime.Token);
        models.SelectionChanged += async (_, _) =>
        {
            if (models.SelectedItem is ComboBoxItem item && item.Tag is string id && id != model.Snapshot?.ModelId)
                await model.SelectModelAsync(id, lifetime.Token);
        };
        createSession.Click += async (_, _) => await create();
        attach.Click += async (_, _) =>
        {
            if (pickAttachment != null && await pickAttachment() is { } path) await model.AttachAsync(path, lifetime.Token);
        };
        attach.Visibility = pickAttachment == null ? Visibility.Collapsed : Visibility.Visible;
        queue.Click += async (_, _) => await model.QueueAsync(lifetime.Token);
        var actions = new Grid();
        actions.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        actions.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        actions.Children.Add(models);
        send.HorizontalAlignment = HorizontalAlignment.Right; stop.HorizontalAlignment = HorizontalAlignment.Right;
        Grid.SetColumn(send, 1); Grid.SetColumn(stop, 1); actions.Children.Add(send); actions.Children.Add(stop);
        var card = new StackPanel { Spacing = 12, Padding = new Thickness(16) };
        var extras = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        extras.Children.Add(attach); extras.Children.Add(queue);
        card.Children.Add(extras); card.Children.Add(attachments); card.Children.Add(composer); card.Children.Add(actions);
        var follow = new CheckBox { Content = "跟随最新回复", IsChecked = true, FontSize = 11 };
        follow.Checked += (_, _) => followLatest = true; follow.Unchecked += (_, _) => followLatest = false;
        var footer = new Grid { Margin = new Thickness(24, 0, 24, 16), MaxWidth = 850 };
        footer.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        footer.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        hint.Text = "Ctrl+Enter 发送 · Enter 换行";
        footer.Children.Add(hint); Grid.SetColumn(follow, 1); footer.Children.Add(follow);
        composerBar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        composerBar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        composerBar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var empty = new StackPanel { Spacing = 12, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(24) };
        empty.Children.Add(new TextBlock { Text = "开始新的研究对话", FontSize = 22, HorizontalAlignment = HorizontalAlignment.Center });
        empty.Children.Add(createSession);
        composerBar.Children.Add(empty);
        var border = new Border { Child = card, CornerRadius = new CornerRadius(14), BorderThickness = new Thickness(1), Margin = new Thickness(24, 0, 24, 8), MaxWidth = 850 };
        Grid.SetRow(border, 1); composerBar.Children.Add(border);
        Grid.SetRow(footer, 2); composerBar.Children.Add(footer);
        Grid.SetRow(composerBar, 3); root.Children.Add(composerBar);
        Content = root;
        Refresh();
        _ = PollAsync();
    }

    private async Task PollAsync()
    {
        try
        {
            while (!lifetime.IsCancellationRequested)
            {
                var delay = model.ConnectionError != null ? 2000 : model.Snapshot?.Running == true ? 350 : 1500;
                await Task.Delay(delay, lifetime.Token);
                await model.RefreshAsync(lifetime.Token);
            }
        }
        catch (OperationCanceledException) { }
    }

    public void Refresh()
    {
        if (disposed) return;
        var error = model.ConnectionError ?? model.OperationError ?? model.Snapshot?.Error;
        status.Text = error ?? "";
        status.Foreground = design.Brush(error == null ? "text-muted" : "clay-strong");
        status.Visibility = error == null && !model.UncertainSend ? Visibility.Collapsed : Visibility.Visible;
        var hasSession = model.Snapshot != null || model.Loading;
        composerBar.Children[0].Visibility = hasSession ? Visibility.Collapsed : Visibility.Visible;
        composerBar.Children[1].Visibility = hasSession ? Visibility.Visible : Visibility.Collapsed;
        composerBar.Children[2].Visibility = hasSession ? Visibility.Visible : Visibility.Collapsed;
        if (composer.Text != model.Draft) composer.Text = model.Draft;
        composer.IsEnabled = model.Snapshot is { ReadOnly: false } && !model.ShowingHistory;
        send.Visibility = model.Snapshot?.Running == true ? Visibility.Collapsed : Visibility.Visible;
        stop.Visibility = model.Snapshot?.Running == true ? Visibility.Visible : Visibility.Collapsed;
        send.IsEnabled = model.CanSend; stop.IsEnabled = !model.Busy;
        attach.IsEnabled = model.CanAttach; queue.IsEnabled = model.CanQueue;
        queue.Content = model.QueuedFollowUp == null ? "排队后续" : "后续已排队";
        attachments.Children.Clear();
        foreach (var file in model.Attachments)
        {
            var remove = new Button { Content = file.Name + " · 移除", IsEnabled = !model.Busy && !model.UncertainSend };
            remove.Click += (_, _) => model.RemoveAttachment(file.Path); attachments.Children.Add(remove);
        }
        stop.Content = model.Snapshot?.Stopping == true ? "正在停止…" : "停止";
        if (models.Items.Count != model.Models.Length)
        {
            models.Items.Clear();
            foreach (var option in model.Models)
                models.Items.Add(new ComboBoxItem { Content = option.Label, Tag = option.Id, IsSelected = option.Id == model.Snapshot?.ModelId });
        }
        models.IsEnabled = !model.Busy && model.Snapshot is { Running: false, ReadOnly: false };
        RenderTranscript();
        RenderApprovals();
        if (followLatest && !model.ShowingHistory) scroll.ChangeView(null, scroll.ScrollableHeight, null);
    }

    private void RenderTranscript()
    {
        transcript.Children.Clear();
        var older = (model.ShowingHistory ? model.History : model.Snapshot)?.NextBeforeSeq != null;
        if (older || model.ShowingHistory)
        {
            var nav = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            if (older)
            {
                var button = new Button { Content = "更早的消息" };
                button.Click += async (_, _) => await model.OlderAsync(lifetime.Token);
                nav.Children.Add(button);
            }
            if (model.ShowingHistory)
            {
                var latest = new Button { Content = "返回最新消息" };
                latest.Click += (_, _) => model.Latest();
                nav.Children.Add(latest);
            }
            transcript.Children.Add(nav);
        }
        var index = 0;
        foreach (var item in model.VisibleItems)
        {
            var captured = index;
            var card = new StackPanel { Spacing = 8, Padding = new Thickness(16) };
            var role = item.Role == "user" ? "你" : item.Role == "tool" ? item.ToolName ?? "工具" : item.Role == "reasoning" ? "思考" : "Wisp Science";
            card.Children.Add(new TextBlock { Text = role, FontSize = 12, Foreground = design.Brush("text-muted") });
            foreach (var path in item.Attachments ?? []) card.Children.Add(new TextBlock { Text = "附件 · " + path, TextWrapping = TextWrapping.Wrap });
            if (item.Role == "question" && JsonNode.Parse(item.Text) is JsonObject question)
            {
                card.Children.Add(new TextBlock { Text = question["question"]?.GetValue<string>() ?? item.Text, TextWrapping = TextWrapping.Wrap });
                foreach (var option in question["options"]?.AsArray() ?? [])
                {
                    var label = option?["label"]?.GetValue<string>() ?? "";
                    var choice = new Button { Content = label, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
                    choice.IsEnabled = !model.ShowingHistory && model.Snapshot?.ReadOnly != true;
                    choice.Click += (_, _) => { model.Draft = label; Refresh(); };
                    card.Children.Add(choice);
                }
            }
            else if (item.Role == "tool")
            {
                var body = new StackPanel { Spacing = 8 };
                if (!string.IsNullOrEmpty(item.Input)) body.Children.Add(new TextBox { Text = item.Input, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap });
                body.Children.Add(new TextBox { Text = item.Text, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap });
                card.Children.Add(new Expander { Header = item.Text.Length == 0 ? "执行中…" : item.Text[..Math.Min(180, item.Text.Length)], Content = body, IsExpanded = item.Ok == false });
            }
            else
            {
                card.Children.Add(TranscriptView.Create(new BrowserMessage(captured, item.Role, item.Text, item.ToolName), design));
                var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                var quoteButton = new Button { Content = "引用" };
                quoteButton.Click += (_, _) => quote(item.Text);
                var star = new Button { Content = "收藏" };
                star.Click += async (_, _) => await model.SaveSelectionAsync(item.Text, lifetime.Token);
                actions.Children.Add(quoteButton); actions.Children.Add(star); card.Children.Add(actions);
            }
            transcript.Children.Add(new Border
            {
                Child = card, CornerRadius = new CornerRadius(12), Padding = new Thickness(4),
                Background = design.Brush(item.Role == "user" ? "bg-sunken" : "bg-app")
            });
            index++;
        }
        if (model.Loading) transcript.Children.Add(new ProgressBar { IsIndeterminate = true, Width = 180 });
        else if (model.VisibleItems.Length == 0 && hasSession())
            transcript.Children.Add(new TextBlock { Text = "向 Wisp Science 提问，开始这个会话。", FontSize = 14, Foreground = design.Brush("text-faint") });
        if (model.Snapshot?.Running == true && !model.ShowingHistory)
            transcript.Children.Add(new TextBlock { Text = model.Snapshot.Stopping ? "正在停止…" : "正在处理…", FontSize = 12, Foreground = design.Brush("text-muted") });
        bool hasSession() => model.Snapshot != null || model.Loading;
    }

    private void RenderApprovals()
    {
        approvals.Children.Clear();
        if (model.ShowingHistory) return;
        foreach (var approval in model.Snapshot?.Approvals ?? [])
        {
            var captured = approval;
            var card = new StackPanel { Spacing = 10, Padding = new Thickness(16) };
            card.Children.Add(new TextBlock { Text = "需要确认 · " + approval.Tool, FontSize = 13 });
            card.Children.Add(new TextBlock { Text = approval.Message, TextWrapping = TextWrapping.Wrap });
            if (approval.Preview.Length > 0)
                card.Children.Add(new TextBox { Text = approval.Preview, IsReadOnly = true, AcceptsReturn = true, FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Consolas"), MaxHeight = 130 });
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
            var deny = new Button { Content = "拒绝", IsEnabled = !model.Busy && model.ConnectionError == null };
            deny.Click += async (_, _) => await model.ApproveAsync(captured, false, lifetime.Token);
            var allow = new Button { Content = "允许这一次", IsEnabled = !model.Busy && model.ConnectionError == null };
            allow.Click += async (_, _) => await model.ApproveAsync(captured, true, lifetime.Token);
            row.Children.Add(deny); row.Children.Add(allow); card.Children.Add(row);
            approvals.Children.Add(new Border { Child = card, CornerRadius = new CornerRadius(12), BorderThickness = new Thickness(1), BorderBrush = design.Brush("clay"), Background = design.Brush("bg-elev") });
        }
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true; lifetime.Cancel(); model.Changed -= Refresh; model.Pause(); lifetime.Dispose();
    }
}
