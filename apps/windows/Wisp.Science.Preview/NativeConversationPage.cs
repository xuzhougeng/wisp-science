using System.Text.Json.Nodes;
using System.Text.Json;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Automation;
using Windows.ApplicationModel.DataTransfer;
using Windows.System;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeConversationPage : UserControl, IDisposable
{
    // Keep native Button focus/pressed states and accessible text.
    // Secondary actions remain present for touch and keyboard users at all times.
    private Button MessageAction(string label)
    {
        var button = new Button
        {
            Content = label, Padding = new Thickness(8, 4, 8, 4),
            MinWidth = 40, MinHeight = 32, BorderThickness = new Thickness(0),
            Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            Foreground = design.Brush("text-muted")
        };
        AutomationProperties.SetName(button, label);
        var pointerInside = false;
        void UpdateEmphasis() => button.Foreground = design.Brush(
            pointerInside || button.FocusState != FocusState.Unfocused ? "text" : "text-muted");
        button.PointerEntered += (_, _) => { pointerInside = true; UpdateEmphasis(); };
        button.PointerExited += (_, _) => { pointerInside = false; UpdateEmphasis(); };
        button.GotFocus += (_, _) => UpdateEmphasis();
        button.LostFocus += (_, _) => UpdateEmphasis();
        return button;
    }
    private readonly WorkspaceConversationModel model;
    private readonly WispDesign design;
    private readonly Action<string> quote;
    private readonly Func<Task> create;
    private readonly StackPanel transcript = new() { Spacing = 18, MaxWidth = PreviewLayout.ConversationMaxWidth, Margin = new Thickness(16) };
    private readonly StackPanel approvals = new() { Spacing = 12, MaxWidth = PreviewLayout.ConversationMaxWidth, Margin = new Thickness(16, 0, 16, 12) };
    private readonly TextBox composer = new() { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 44, PlaceholderText = "向 Wisp Science 提问…" };
    private readonly ComboBox models = new() { MaxWidth = 230 };
    private readonly ComboBox efforts = new() { MaxWidth = 100, MinWidth = 72, Visibility = Visibility.Collapsed };
    private bool updatingEfforts;
    private readonly Microsoft.UI.Xaml.Controls.Primitives.ToggleButton planMode = new() { Content = "Plan", Visibility = Visibility.Collapsed };
    private readonly Microsoft.UI.Xaml.Controls.Primitives.ToggleButton fastMode = new() { Content = "Fast", Visibility = Visibility.Collapsed };
    private readonly Button send = new() { Content = "发送" };
    private readonly Button stop = new() { Content = "停止" };
    private readonly Button createSession = new() { Content = "新建会话" };
    private readonly Button attach = new() { Content = "对话附件" };
    private readonly Flyout composerOptions = new();
    private bool composerOptionsOpen;
    private readonly Button queue = new() { Content = "排队后续" };
    private readonly Button retry = new() { Content = "重新读取" };
    private readonly Button acknowledge = new() { Content = "已检查，允许再次发送或排队…" };
    private readonly StackPanel attachments = new() { Spacing = 4 };
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap, FontSize = 12 };
    private readonly TextBlock hint = new() { FontSize = 11 };
    private readonly Grid composerBar = new();
    private readonly ScrollViewer scroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    private readonly CancellationTokenSource lifetime = new();
    private readonly TextBlock slashHint = new() { FontSize = 11, TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
    private readonly ListView commandChoices = new() { MaxHeight = 170, SelectionMode = ListViewSelectionMode.Single,
        IsItemClickEnabled = true, Visibility = Visibility.Collapsed };
    private bool composing, compositionJustEnded, commandsDismissed, updatingModels;
    private string? attachmentFingerprint;
    private bool disposed, followLatest = true;
    private bool userScrollPending;
    private readonly Button follow = new() { Content = "回到最新", Visibility = Visibility.Collapsed };
    private readonly Dictionary<string, (string Fingerprint, FrameworkElement Element)> renderedRows = [];
    private sealed class ActivityDisclosure
    {
        public Microsoft.UI.Xaml.Controls.Primitives.ToggleButton Button { get; } = new();
        public List<FrameworkElement> Rows { get; } = [];
        public void Update()
        {
            var expanded = Button.IsChecked == true;
            Button.Content = $"{(expanded ? "收起" : "显示")}已完成过程 · {Rows.Count} 条";
            foreach (var row in Rows) row.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed;
        }
    }
    private readonly Dictionary<string, ActivityDisclosure> activityDisclosures = [];
    private string? approvalFingerprint;
    private string? readingSession;
    private readonly Func<string, bool>? slashCommand;
    private readonly Action? openHosts;
    private readonly Action<string>? openRun;
    private readonly Func<Task<string?>>? pickAttachment;

    public NativeConversationPage(WorkspaceConversationModel model, WispDesign design, Action<string> quote, Func<Task> create,
        Func<Task<string?>>? pickAttachment = null, Func<string, bool>? slashCommand = null, Action? openHosts = null,
        Action<string>? openRun = null)
    {
        this.model = model; this.design = design; this.quote = quote; this.create = create;
        this.slashCommand = slashCommand; this.openHosts = openHosts; this.pickAttachment = pickAttachment;
        this.openRun = openRun;
        design.BindTypography(status, 12); design.BindTypography(hint, 11);
        var root = new Grid();
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        status.Margin = new Thickness(24, 12, 24, 0);
        retry.Click += async (_, _) =>
        {
            if (model.Effort.Error != null) await model.Effort.ReloadAsync(lifetime.Token);
            await model.RefreshAsync(lifetime.Token);
        };
        acknowledge.Click += (_, _) => model.AcknowledgeUncertainSend();
        var banner = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Margin = new Thickness(24, 8, 24, 0) };
        banner.Children.Add(retry); banner.Children.Add(acknowledge);
        var header = new StackPanel(); header.Children.Add(status); header.Children.Add(banner);
        root.Children.Add(header);
        scroll.Content = transcript; Grid.SetRow(scroll, 1); root.Children.Add(scroll);
        scroll.AddHandler(UIElement.PointerWheelChangedEvent, new PointerEventHandler((_, _) => PauseFollowing()), true);
        scroll.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler((_, _) => PauseFollowing()), true);
        scroll.AddHandler(UIElement.KeyDownEvent, new KeyEventHandler((_, e) =>
        {
            if (e.Key is VirtualKey.Up or VirtualKey.Down or VirtualKey.PageUp or VirtualKey.PageDown or VirtualKey.Home or VirtualKey.End)
                PauseFollowing();
        }), true);
        scroll.ViewChanged += (_, e) =>
        {
            if (!userScrollPending) return;
            followLatest = scroll.ScrollableHeight - scroll.VerticalOffset <= 48;
            UpdateFollowButton();
            if (!e.IsIntermediate) userScrollPending = false;
        };
        transcript.SizeChanged += (_, _) => FollowAfterLayout();
        Grid.SetRow(approvals, 2); root.Children.Add(approvals);
        composer.TextChanged += (_, _) =>
        {
            model.Draft = composer.Text; send.IsEnabled = model.CanSend; queue.IsEnabled = model.CanQueue;
            commandsDismissed = false;
            UpdateSlashHint();
        };
        composer.TextCompositionStarted += (_, _) => composing = true;
        composer.TextCompositionEnded += (_, _) =>
        {
            composing = false; compositionJustEnded = true;
            DispatcherQueue.TryEnqueue(() => compositionJustEnded = false);
        };
        composer.KeyDown += (_, e) =>
        {
            if (composing || compositionJustEnded) return;
            var shift = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift).HasFlag(VirtualKeyStates.Down);
            var control = InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control).HasFlag(VirtualKeyStates.Down);
            if (commandChoices.Visibility == Visibility.Visible && !control && !shift)
            {
                if (e.Key is VirtualKey.Up or VirtualKey.Down)
                {
                    var direction = e.Key == VirtualKey.Down ? 1 : -1;
                    commandChoices.SelectedIndex = (commandChoices.SelectedIndex + direction + commandChoices.Items.Count) % commandChoices.Items.Count;
                    commandChoices.ScrollIntoView(commandChoices.SelectedItem); e.Handled = true; return;
                }
                if (e.Key is VirtualKey.Enter or VirtualKey.Tab && commandChoices.SelectedItem is NativeComposerCommand selected)
                { ChooseCommand(selected); e.Handled = true; return; }
            }
            if (e.Key == VirtualKey.Enter && NativeComposerCommands.ShouldSubmit(control, shift, composing || compositionJustEnded))
            { e.Handled = true; Submit(); }
        };
        commandChoices.ItemClick += (_, e) => { if (e.ClickedItem is NativeComposerCommand command) ChooseCommand(command); };
        AutomationProperties.SetName(commandChoices, "命令候选，方向键选择，Enter 填入，Ctrl+Enter 执行");
        send.Click += (_, _) => Submit();
        stop.Click += async (_, _) => await model.StopAsync(lifetime.Token);
        models.SelectionChanged += async (_, _) =>
        {
            if (!updatingModels && models.SelectedItem is ComboBoxItem item && item.Tag is string id && id != model.Snapshot?.ModelId)
                await model.SelectModelAsync(id, lifetime.Token);
        };
        createSession.Click += async (_, _) => await create();
        attach.Click += async (_, _) => await DoAttachAsync();
        attach.Visibility = pickAttachment == null ? Visibility.Collapsed : Visibility.Visible;
        queue.Click += async (_, _) => await model.QueueAsync(lifetime.Token);
        var hosts = design.ToolButton("执行环境", "server", showLabel: true);
        hosts.Click += (_, _) => openHosts?.Invoke();
        hosts.Visibility = openHosts == null ? Visibility.Collapsed : Visibility.Visible;
        hosts.HorizontalAlignment = HorizontalAlignment.Left;
        attach.Content = design.Icon("plus", 18);
        AutomationProperties.SetName(attach, "添加附件"); ToolTipService.SetToolTip(attach, "添加附件");
        var options = new Button { Content = design.Icon("adjustments", 18) };
        AutomationProperties.SetName(options, "对话选项"); ToolTipService.SetToolTip(options, "对话选项");
        var optionContent = new StackPanel { Spacing = 10, MinWidth = 220 };
        optionContent.Children.Add(design.Text("对话选项", 15));
        planMode.Content = "计划模式";
        optionContent.Children.Add(planMode);
        var optionHint = design.Text("先调查并提交计划，再决定是否执行。", 12);
        optionHint.Foreground = design.Brush("text-muted"); optionContent.Children.Add(optionHint);
        composerOptions.Content = optionContent;
        composerOptions.Opened += (_, _) => composerOptionsOpen = true;
        composerOptions.Closed += (_, _) => composerOptionsOpen = false;
        options.Flyout = composerOptions;
        AutomationProperties.SetName(planMode, "计划模式");
        planMode.Click += async (_, _) => await model.SetPlanModeAsync(planMode.IsChecked == true, lifetime.Token);
        fastMode.Content = design.Icon("bolt", 18);
        AutomationProperties.SetName(fastMode, "Fast 优先服务");
        fastMode.Click += async (_, _) => await model.SetFastModeAsync(fastMode.IsChecked == true, lifetime.Token);
        send.Content = design.Icon("arrow-up", 18);
        send.Opacity = send.IsEnabled ? 1 : 0.35;
        send.RegisterPropertyChangedCallback(Control.IsEnabledProperty, (sender, _) =>
        {
            var button = (Button)sender;
            button.Opacity = button.IsEnabled ? 1 : 0.35;
        });
        AutomationProperties.SetName(send, "发送"); ToolTipService.SetToolTip(send, "发送");
        foreach (var button in new[] { attach, options, send })
        {
            design.QuietButton(button);
            button.Width = button.Height = button.MinWidth = button.MinHeight = 32;
            button.Padding = new Thickness(6); button.CornerRadius = new CornerRadius(16);
            button.BorderThickness = new Thickness(1); button.BorderBrush = design.Brush("border");
            button.VerticalAlignment = VerticalAlignment.Center;
        }
        send.Background = design.Brush("bg-sunken");
        fastMode.Width = fastMode.Height = fastMode.MinWidth = fastMode.MinHeight = 32;
        fastMode.Padding = new Thickness(6); fastMode.CornerRadius = new CornerRadius(16);
        fastMode.VerticalAlignment = VerticalAlignment.Center;
        foreach (var button in new[] { queue, stop }) { design.QuietButton(button); button.VerticalAlignment = VerticalAlignment.Center; }
        models.HorizontalAlignment = HorizontalAlignment.Right; models.MinWidth = 144;
        foreach (var picker in new[] { models, efforts })
        {
            picker.Height = picker.MinHeight = 32; picker.CornerRadius = new CornerRadius(16);
            picker.VerticalAlignment = VerticalAlignment.Center;
        }
        AutomationProperties.SetName(models, "对话模型");
        AutomationProperties.SetName(efforts, "模型默认思考强度");
        ToolTipService.SetToolTip(efforts, "更改此模型的默认思考强度；没有会话覆盖时，在下一轮使用。");
        efforts.SelectionChanged += async (_, _) =>
        {
            if (!updatingEfforts && efforts.SelectedItem is ComboBoxItem item && item.Tag is string value && value != model.Effort.Value)
                await model.SelectEffortAsync(value, lifetime.Token);
        };
        var actions = new Grid { ColumnSpacing = 12, RowSpacing = 8 };
        actions.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        actions.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        actions.RowDefinitions.Add(new() { Height = GridLength.Auto });
        actions.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var leading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        leading.Children.Add(attach); leading.Children.Add(options); actions.Children.Add(leading);
        var tools = new Grid { ColumnSpacing = 6, VerticalAlignment = VerticalAlignment.Center };
        tools.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        for (var i = 0; i < 3; i++) tools.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        tools.Children.Add(models);
        Grid.SetColumn(efforts, 1); tools.Children.Add(efforts);
        Grid.SetColumn(fastMode, 2); tools.Children.Add(fastMode);
        var primary = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        primary.Children.Add(queue); primary.Children.Add(send); primary.Children.Add(stop);
        Grid.SetColumn(primary, 3); tools.Children.Add(primary);
        Grid.SetColumn(tools, 1); actions.Children.Add(tools);
        actions.SizeChanged += (_, e) =>
        {
            var narrow = PreviewLayout.StackComposerSend(e.NewSize.Width);
            Grid.SetColumn(tools, narrow ? 0 : 1); Grid.SetColumnSpan(tools, narrow ? 2 : 1);
            Grid.SetRow(tools, narrow ? 1 : 0);
        };
        var card = new StackPanel { Spacing = 8, Padding = new Thickness(14) };
        composer.MinHeight = 56;
        composer.BorderThickness = new Thickness(0);
        composer.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        composer.Padding = new Thickness(0, 4, 0, 6);
        composer.Resources["TextControlBackgroundFocused"] = design.Brush("bg-elev");
        composer.Resources["TextControlBorderBrushFocused"] = design.Brush("clay");
        composer.MaxHeight = 240;
        card.Children.Add(commandChoices); card.Children.Add(attachments); card.Children.Add(composer);
        design.BindTypography(slashHint, 11);
        slashHint.Foreground = design.Brush("text-faint");
        slashHint.Text = "方向键选择 · Enter 填入 · Ctrl+Enter 执行";
        slashHint.Visibility = Visibility.Collapsed;
        follow.Click += (_, _) => { followLatest = true; userScrollPending = false; FollowAfterLayout(); UpdateFollowButton(); };
        var footer = new Grid { Margin = new Thickness(24, 0, 24, 10), MaxWidth = PreviewLayout.ConversationMaxWidth };
        footer.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        footer.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var footerHints = new StackPanel { Spacing = 2 };
        footerHints.Children.Add(slashHint);
        hint.Text = "Ctrl+Enter 发送 · Enter 换行";
        hint.Foreground = design.Brush("text-faint");
        hint.HorizontalAlignment = HorizontalAlignment.Center; hint.TextAlignment = TextAlignment.Center;
        card.Children.Add(hint); card.Children.Add(actions);
        footer.Children.Add(footerHints); Grid.SetColumn(follow, 1); footer.Children.Add(follow);
        composerBar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        composerBar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        composerBar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var empty = new StackPanel { Spacing = 12, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(24) };
        var emptyHeading = design.Text("开始新的研究对话", 22); emptyHeading.HorizontalAlignment = HorizontalAlignment.Center; empty.Children.Add(emptyHeading);
        empty.Children.Add(createSession);
        composerBar.Children.Add(empty);
        var border = new Border { Child = card, CornerRadius = new CornerRadius(16), BorderThickness = new Thickness(1),
            BorderBrush = design.Brush("border-strong"), Background = design.Brush("bg-elev"),
            Margin = new Thickness(20, 0, 20, 8), MaxWidth = PreviewLayout.ConversationMaxWidth };
        var inputArea = new StackPanel { Spacing = 4, MaxWidth = PreviewLayout.ConversationMaxWidth + 40 };
        hosts.Margin = new Thickness(20, 0, 20, 0); inputArea.Children.Add(hosts); inputArea.Children.Add(border);
        Grid.SetRow(inputArea, 1); composerBar.Children.Add(inputArea);
        Grid.SetRow(footer, 2); composerBar.Children.Add(footer);
        Grid.SetRow(composerBar, 3); root.Children.Add(composerBar);
        Content = root;
        Refresh();
        // Subscribe only after the controls have been constructed successfully.
        // A failed constructor must not leave a half-built page observing resets.
        model.Changed += Refresh;
        model.Effort.Changed += Refresh;
        design.TypographyChanged += Refresh;
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

    private async Task DoAttachAsync(string? commandDraft = null)
    {
        if (pickAttachment == null || !model.CanAttach) return;
        var session = model.Snapshot;
        if (await pickAttachment() is not { } path || disposed || model.Snapshot?.SessionId != session?.SessionId
            || model.Snapshot?.ProjectId != session?.ProjectId || !model.CanAttach) return;
        var attached = await model.AttachAsync(path, lifetime.Token);
        if (commandDraft != null && attached && model.Draft == commandDraft
            && model.Snapshot?.SessionId == session?.SessionId && model.Snapshot?.ProjectId == session?.ProjectId)
        { model.Draft = ""; Refresh(); }
    }

    private void UpdateSlashHint()
    {
        var matches = commandsDismissed || !composer.IsEnabled ? []
            : NativeComposerCommands.Match(composer.Text, pickAttachment != null, slashCommand != null);
        var previous = (commandChoices.SelectedItem as NativeComposerCommand)?.Command;
        commandChoices.ItemsSource = matches;
        commandChoices.SelectedIndex = matches.Count == 0 ? -1 : Math.Max(0, matches.ToList().FindIndex(item => item.Command == previous));
        commandChoices.Visibility = matches.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        slashHint.Visibility = !commandsDismissed && composer.Text.TrimStart().StartsWith('/') ? Visibility.Visible : Visibility.Collapsed;
        slashHint.Text = matches.Count > 0 ? "方向键选择 · Enter 填入 · Ctrl+Enter 执行"
            : "命令不存在或包含额外文字；草稿已保留。输入 / 查看可用命令。";
    }

    private void ChooseCommand(NativeComposerCommand command)
    {
        composer.Text = command.Command;
        composer.SelectionStart = composer.Text.Length;
        commandsDismissed = true; commandChoices.Visibility = Visibility.Collapsed;
        slashHint.Text = "Ctrl+Enter 执行命令";
        composer.Focus(FocusState.Programmatic);
    }

    public bool HandleEscape()
    {
        if (composerOptionsOpen) { composerOptions.Hide(); return true; }
        if (efforts.IsDropDownOpen) { efforts.IsDropDownOpen = false; return true; }
        if (composing || compositionJustEnded) return false;
        if (models.IsDropDownOpen) { models.IsDropDownOpen = false; return true; }
        if (commandChoices.Visibility != Visibility.Visible) return false;
        commandsDismissed = true; commandChoices.Visibility = slashHint.Visibility = Visibility.Collapsed;
        return true;
    }

    /// <summary>Send attempt. A leading slash routes to a client-side command
    /// mapping instead of the send API; unknown commands keep the draft and
    /// surface the available set. Real sends are untouched.</summary>
    private void Submit()
    {
        var draft = model.Draft.Trim();
        if (draft.StartsWith('/'))
        {
            var command = NativeComposerCommands.Exact(draft)?.Command;
            if (command == null) { commandsDismissed = false; UpdateSlashHint(); return; }
            if (command == "/upload" && pickAttachment != null)
            {
                _ = DoAttachAsync(model.Draft); return;
            }
            if (slashCommand != null && slashCommand(command))
            {
                if (model.Draft == composer.Text && model.Draft.Length > 0) { model.Draft = ""; Refresh(); }
                return;
            }
            UpdateSlashHint();
            return;
        }
        _ = model.SendAsync(lifetime.Token);
    }

    public void Refresh()
    {
        if (disposed) return;
        var session = model.Snapshot is { } snapshot ? snapshot.ProjectId + "/" + snapshot.SessionId : null;
        if (session != readingSession) { readingSession = session; followLatest = true; userScrollPending = false; }
        FontFamily = design.Font(); FontSize = design.FontSize(14);
        composer.FontFamily = design.Font(); composer.FontSize = design.FontSize(14);
        var error = model.ConnectionError ?? model.OperationError ?? model.Effort.Error ?? model.Snapshot?.Error;
        retry.Visibility = error != null ? Visibility.Visible : Visibility.Collapsed;
        acknowledge.Visibility = model.UncertainSend ? Visibility.Visible : Visibility.Collapsed;
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
        queue.Visibility = model.Snapshot?.Running == true || model.QueuedFollowUp != null ? Visibility.Visible : Visibility.Collapsed;
        var nextAttachments = JsonSerializer.Serialize(model.Attachments) + $"/{model.Busy}/{model.UncertainSend}";
        if (nextAttachments != attachmentFingerprint)
        {
            attachmentFingerprint = nextAttachments;
            attachments.Children.Clear();
            foreach (var file in model.Attachments)
            {
                var remove = new Button { Content = file.Name + " · 移除", IsEnabled = !model.Busy && !model.UncertainSend };
                AutomationProperties.SetName(remove, "移除附件 " + file.Name);
                remove.Click += (_, _) => model.RemoveAttachment(file.Path); attachments.Children.Add(remove);
            }
        }
        attachments.Visibility = model.Attachments.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        stop.Content = model.Snapshot?.Stopping == true ? "正在停止…" : "停止";
        updatingModels = true;
        try
        {
            if (!models.Items.Cast<ComboBoxItem>().Select(item => ((string)item.Tag, (string)item.Content))
                .SequenceEqual(model.Models.Select(option => (option.Id, option.Label))))
            {
                models.Items.Clear();
                foreach (var option in model.Models) models.Items.Add(new ComboBoxItem { Content = option.Label, Tag = option.Id });
            }
            models.SelectedItem = models.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == model.Snapshot?.ModelId);
        }
        finally { updatingModels = false; }
        models.IsEnabled = !model.Busy && !model.Effort.Busy && model.Snapshot is { Running: false, ReadOnly: false };
        updatingEfforts = true;
        try
        {
            var values = new[] { "" }.Concat(model.Effort.Options).ToArray();
            if (!efforts.Items.Cast<ComboBoxItem>().Select(item => (string)item.Tag).SequenceEqual(values))
            {
                efforts.Items.Clear();
                foreach (var value in values) efforts.Items.Add(new ComboBoxItem { Content = value.Length == 0 ? "默认强度" : value, Tag = value });
            }
            efforts.SelectedItem = efforts.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == model.Effort.Value);
            efforts.PlaceholderText = model.Effort.Value;
        }
        finally { updatingEfforts = false; }
        efforts.Visibility = model.Effort.Options.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        efforts.IsEnabled = models.IsEnabled && model.CanAttach;
        planMode.Visibility = model.Snapshot?.PlanMode is null ? Visibility.Collapsed : Visibility.Visible;
        planMode.IsChecked = model.Snapshot?.PlanMode == true;
        planMode.IsEnabled = model.CanChangePlanMode;
        fastMode.Visibility = model.Snapshot?.FastMode is null ? Visibility.Collapsed : Visibility.Visible;
        fastMode.IsChecked = model.Snapshot?.FastMode?.Enabled == true;
        fastMode.IsEnabled = model.CanChangeFastMode;
        ToolTipService.SetToolTip(fastMode, "请求优先服务等级，费用以提供方为准。" +
            (model.Snapshot?.FastMode?.Inherited == true ? "当前继承模型默认值。" : "当前使用此会话的覆盖值。"));
        if (!composer.IsEnabled) { commandChoices.Visibility = Visibility.Collapsed; slashHint.Visibility = Visibility.Collapsed; }
        RenderTranscript();
        RenderApprovals();
        design.ApplyTypography(this);
        FollowAfterLayout();
        UpdateFollowButton();
    }

    private void FollowAfterLayout()
    {
        if (disposed || !followLatest || userScrollPending || model.ShowingHistory) return;
        DispatcherQueue.TryEnqueue(() =>
        {
            if (!disposed && followLatest && !userScrollPending && !model.ShowingHistory)
                scroll.ChangeView(null, scroll.ScrollableHeight, null, true);
        });
    }

    private void PauseFollowing()
    {
        userScrollPending = true; followLatest = false; UpdateFollowButton();
    }

    private void UpdateFollowButton() => follow.Visibility = !followLatest && !model.ShowingHistory ? Visibility.Visible : Visibility.Collapsed;

    private void RenderTranscript()
    {
        var desired = new List<UIElement>();
        var retained = new HashSet<string>();
        var snapshot = model.ShowingHistory ? model.History : model.Snapshot;
        var keys = snapshot == null ? [] : NativeTranscriptRows.Keys(snapshot);
        var style = $"{design.Dark}/{design.LightPalette}/{design.DarkPalette}/{design.Typography}";
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
            desired.Add(nav);
        }
        var index = 0;
        foreach (var item in model.VisibleItems)
        {
            var captured = index;
            var key = keys[index];
            retained.Add(key);
            var fingerprint = JsonSerializer.Serialize(item) + style
                + (item.Role == "question" ? $"/{model.ShowingHistory}/{model.Snapshot?.ReadOnly}" : "");
            if (renderedRows.TryGetValue(key, out var previous) && previous.Fingerprint == fingerprint)
            {
                desired.Add(previous.Element); index++; continue;
            }
            void Add(FrameworkElement element)
            {
                if (previous.Element != null && !NativeToolPresentation.RequiresAttention(item))
                {
                    var states = Expanders(previous.Element).Select(expander => expander.IsExpanded).ToArray();
                    var expanders = Expanders(element).ToArray();
                    for (var n = 0; n < Math.Min(states.Length, expanders.Length); n++) expanders[n].IsExpanded = states[n];
                }
                renderedRows[key] = (fingerprint, element); desired.Add(element);
            }
            if (TranscriptPresentation.UsageSummary(item.Role, item.Text) is { } usage)
            {
                // Usage rows stay bare like the WebView's metadata line: no card, no actions.
                Add(new TextBlock { Text = usage, FontSize = design.FontSize(12),
                    FontFamily = design.Font(), Foreground = design.Brush("text-faint"), Margin = new Thickness(16, 0, 16, 0) });
                index++;
                continue;
            }
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
                if (!string.IsNullOrEmpty(item.Input)) body.Children.Add(new TextBox { Text = item.Input, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(true), FontSize = design.FontSize(12, true) });
                if (TranscriptPresentation.ToolImagePath(item.Text) is { } imagePath && File.Exists(imagePath))
                {
                    try
                    {
                        body.Children.Add(new Border
                        {
                            Child = new Image { Source = new BitmapImage(new Uri(imagePath)), MaxHeight = 340, Stretch = Stretch.Uniform, HorizontalAlignment = HorizontalAlignment.Left },
                            CornerRadius = new CornerRadius(8), Padding = new Thickness(4), Background = design.Brush("bg-sunken")
                        });
                    }
                    catch { }
                }
                body.Children.Add(new TextBox { Text = item.Text, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(true), FontSize = design.FontSize(12, true) });
                var heading = new StackPanel { Spacing = 4 };
                heading.Children.Add(new TextBlock { Text = NativeToolPresentation.Heading(item),
                    Foreground = design.Brush(NativeToolPresentation.IsFailure(item) ? "clay-strong" : "text-muted") });
                if (item.Run is { } run) heading.Children.Add(new TextBlock { Text = "Run · " + run.Id,
                    TextWrapping = TextWrapping.Wrap, Foreground = design.Brush("text-muted"), FontSize = design.FontSize(12) });
                if (!string.IsNullOrWhiteSpace(item.Text)) heading.Children.Add(new TextBlock {
                    Text = item.Text, MaxLines = 2, TextWrapping = TextWrapping.Wrap, TextTrimming = TextTrimming.CharacterEllipsis,
                    Foreground = design.Brush("text-muted"), FontSize = design.FontSize(12) });
                card.Children.Add(new Expander { Header = heading, HorizontalAlignment = HorizontalAlignment.Stretch,
                    HorizontalContentAlignment = HorizontalAlignment.Stretch, Content = body,
                    IsExpanded = NativeToolPresentation.RequiresAttention(item) });
                if (item.Run is { } linkedRun && openRun is not null)
                {
                    var detail = MessageAction("查看运行详情");
                    detail.Click += (_, _) => openRun(linkedRun.Id);
                    card.Children.Add(detail);
                }
            }
            else
            {
                card.Children.Add(TranscriptView.Create(new BrowserMessage(captured, item.Role, item.Text, item.ToolName), design));
                var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 };
                var copy = MessageAction("复制");
                copy.Click += (_, _) =>
                {
                    try { var package = new DataPackage { RequestedOperation = DataPackageOperation.Copy }; package.SetText(item.Text); Clipboard.SetContent(package); }
                    catch { }
                };
                var quoteButton = MessageAction("引用");
                quoteButton.Click += (_, _) => quote(item.Text);
                var star = MessageAction("收藏");
                star.Click += async (_, _) => await model.SaveSelectionAsync(item.Text, lifetime.Token);
                actions.Children.Add(copy); actions.Children.Add(quoteButton); actions.Children.Add(star); card.Children.Add(actions);
            }
            Add(new Border
            {
                Child = card, CornerRadius = new CornerRadius(12), Padding = new Thickness(4),
                Background = design.Brush(item.Role == "user" ? "bg-sunken" : "bg-app")
            });
            index++;
        }
        // Keep row controls mounted even when hidden. Toggling a process must
        // not rebuild Markdown, clear selection on other rows or reset tools.
        foreach (var key in retained) renderedRows[key].Element.Visibility = Visibility.Visible;
        var retainedGroups = new HashSet<string>();
        if (snapshot != null)
        {
            foreach (var group in NativeTranscriptActivity.Groups(snapshot.Items, snapshot.Running || snapshot.Stopping))
            {
                var groupKey = keys[group.Start];
                retainedGroups.Add(groupKey);
                if (!activityDisclosures.TryGetValue(groupKey, out var disclosure))
                {
                    disclosure = new ActivityDisclosure();
                    var capturedDisclosure = disclosure;
                    disclosure.Button.Click += (_, _) => capturedDisclosure.Update();
                    activityDisclosures[groupKey] = disclosure;
                }
                disclosure.Rows.Clear();
                for (var row = group.Start; row < group.End; row++) disclosure.Rows.Add(renderedRows[keys[row]].Element);
                disclosure.Update();
                desired.Insert(desired.IndexOf(disclosure.Rows[0]), disclosure.Button);
            }
        }
        foreach (var key in activityDisclosures.Keys.Where(key => !retainedGroups.Contains(key)).ToArray()) activityDisclosures.Remove(key);
        if (model.Loading) desired.Add(new ProgressBar { IsIndeterminate = true, Width = 180 });
        else if (model.VisibleItems.Length == 0 && hasSession())
            desired.Add(new TextBlock { Text = "向 Wisp Science 提问，开始这个会话。", FontSize = 14, Foreground = design.Brush("text-faint") });
        if (model.Snapshot?.Running == true && !model.ShowingHistory)
            desired.Add(new TextBlock { Text = model.Snapshot.Stopping ? "正在停止…" : "正在处理…", FontSize = 12, Foreground = design.Brush("text-muted") });
        foreach (var key in renderedRows.Keys.Where(key => !retained.Contains(key)).ToArray()) renderedRows.Remove(key);
        // Keep unchanged controls attached: rebuilding their parents would still
        // discard text selection, keyboard focus and nested disclosure state.
        var wanted = desired.ToHashSet();
        var anchor = !followLatest ? transcript.Children.OfType<FrameworkElement>()
            .Where(element => wanted.Contains(element) && element.Visibility == Visibility.Visible && element.ActualHeight > 0)
            .Select(element => (Element: element, Y: element.TransformToVisual(scroll).TransformPoint(new Windows.Foundation.Point()).Y))
            .FirstOrDefault(item => item.Y + item.Element.ActualHeight > 0) : default;
        for (var n = transcript.Children.Count - 1; n >= 0; n--)
            if (!wanted.Contains(transcript.Children[n])) transcript.Children.RemoveAt(n);
        for (var n = 0; n < desired.Count; n++)
        {
            if (n < transcript.Children.Count && ReferenceEquals(transcript.Children[n], desired[n])) continue;
            var existing = transcript.Children.IndexOf(desired[n]);
            if (existing >= 0) transcript.Children.RemoveAt(existing);
            transcript.Children.Insert(n, desired[n]);
        }
        if (anchor.Element != null)
        {
            var session = readingSession;
            EventHandler<object>? restore = null;
            restore = (_, _) =>
            {
                transcript.LayoutUpdated -= restore;
                if (disposed || followLatest || userScrollPending || session != readingSession || !transcript.Children.Contains(anchor.Element)) return;
                var current = anchor.Element.TransformToVisual(scroll).TransformPoint(new Windows.Foundation.Point()).Y;
                if (Math.Abs(current - anchor.Y) > 1) scroll.ChangeView(null, scroll.VerticalOffset + current - anchor.Y, null, true);
            };
            transcript.LayoutUpdated += restore;
        }
        bool hasSession() => model.Snapshot != null || model.Loading;
    }

    private static IEnumerable<Expander> Expanders(DependencyObject element)
    {
        if (element is Expander expander) yield return expander;
        IEnumerable<DependencyObject> children = element switch
        {
            Panel panel => panel.Children.Cast<DependencyObject>(),
            Border border when border.Child != null => [border.Child],
            ContentControl content when content.Content is DependencyObject child => [child],
            _ => []
        };
        foreach (var child in children)
            foreach (var nested in Expanders(child)) yield return nested;
    }

    private void RenderApprovals()
    {
        var fingerprint = JsonSerializer.Serialize(model.Snapshot?.Approvals) + $"/{model.ShowingHistory}/{model.Busy}/{model.ConnectionError}/{design.Typography}";
        if (fingerprint == approvalFingerprint) return;
        approvalFingerprint = fingerprint;
        approvals.Children.Clear();
        if (model.ShowingHistory) return;
        foreach (var approval in model.Snapshot?.Approvals ?? [])
        {
            var captured = approval;
            var card = new StackPanel { Spacing = 10, Padding = new Thickness(16) };
            card.Children.Add(new TextBlock { Text = "需要确认 · " + approval.Tool, FontSize = 13 });
            card.Children.Add(new TextBlock { Text = approval.Message, TextWrapping = TextWrapping.Wrap });
            if (approval.Preview.Length > 0)
                card.Children.Add(new TextBox { Text = approval.Preview, IsReadOnly = true, AcceptsReturn = true, FontFamily = design.Font(true), FontSize = design.FontSize(12, true), MaxHeight = 130 });
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
        composerOptions.Hide();
        disposed = true; lifetime.Cancel(); model.Changed -= Refresh; model.Effort.Changed -= Refresh; design.TypographyChanged -= Refresh; model.Pause(); lifetime.Dispose();
    }
}
