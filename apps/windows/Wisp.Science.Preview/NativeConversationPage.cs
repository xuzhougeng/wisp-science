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

internal sealed partial class NativeConversationPage : UserControl, IDisposable
{
    public void FocusComposer()
    {
        var project = model.CurrentProjectId; var session = model.CurrentSessionId;
        // Closing a narrow panel reparents the page; its SizeChanged handler
        // re-enables the conversation after layout. Focus only that same session.
        DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
        {
            if (!disposed && IsLoaded && project == model.CurrentProjectId
                && session == model.CurrentSessionId) composer.Focus(FocusState.Keyboard);
        });
    }
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
        design.BindTypography(button, size: 14);
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
    private readonly Action<string>? openBranch;
    private readonly Action<string, string>? openBranchResult;
    private readonly Dictionary<string, bool> branchDisclosures = new();
    private string? explorationBannerKey;
    private NativeExplorationBanner? explorationBanner;
    private readonly Action<NativeExplorationSummary>? reviewExploration;
    private readonly Action<NativeHistoryTarget>? startExploration;
    private readonly Func<Task> create;
    private readonly StackPanel transcript = new() { Spacing = 20, MaxWidth = PreviewLayout.ConversationMaxWidth,
        HorizontalAlignment = HorizontalAlignment.Center, Padding = new Thickness(16, 26, 16, 40) };
    private readonly StackPanel approvals = new() { Spacing = 12, MaxWidth = PreviewLayout.ConversationMaxWidth, Margin = new Thickness(16, 0, 16, 12) };
    private readonly TextBox composer = new() { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 44, PlaceholderText = "请输入问题 — @ 附加产物，# 附加会话，/ 选择技能…" };
    private readonly Button models = new() { MaxWidth = 220 };
    private readonly Button efforts = new() { MaxWidth = 180, MinWidth = 0, Visibility = Visibility.Collapsed };
    private readonly ToggleSwitch planMode = new();
    private readonly Microsoft.UI.Xaml.Controls.Primitives.ToggleButton fastMode = new() { Content = "Fast", Visibility = Visibility.Collapsed };
    private readonly Button send = new() { Content = "发送" };
    private readonly Button stop = new() { Content = "停止" };
    private readonly Button createSession = new() { Content = "新建会话" };
    private readonly Button attach = new() { Content = "对话附件" };
    private readonly Flyout composerOptions = new() { Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.TopEdgeAlignedLeft };
    private readonly NativeComposerResizeGrip composerResize;
    private bool composerOptionsOpen;
    private Action? refreshComposerFocus;
    private readonly Border stoppingNotice = new();
    private readonly SolidColorBrush stopFill = new() { Opacity = .78 };
    private readonly ProgressRing stoppingSpinner = new() { Width = 16, Height = 16, IsActive = false, VerticalAlignment = VerticalAlignment.Top };
    private readonly TextBlock stoppingTitle, stoppingDetail;
    private readonly Button retry = new() { Content = "重新读取" };
    private readonly StackPanel retryActions;
    private readonly Button acknowledge = new() { Content = "已检查，允许再次发送或排队…" };
    private readonly NativeActionWrap attachments = new() { Gap = 7, Margin = new Thickness(0, 2, 0, 9) };
    private readonly Button assistantContext = new() { HorizontalAlignment = HorizontalAlignment.Left, Padding = new Thickness(8, 4, 8, 4), Margin = new Thickness(0, 0, 0, 8) };
    private readonly TextBlock assistantContextLabel = new() { TextTrimming = TextTrimming.CharacterEllipsis };
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap, FontSize = 12 };
    private readonly TextBlock hint = new() { FontSize = 11 };
    private readonly Grid composerBar = new() { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
    private readonly ScrollViewer welcomeScroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly Image welcomeMark;
    private readonly TextBlock emptyHeading;
    private readonly TextBlock emptyDescription;
    private readonly TextBlock tipLabel;
    private readonly TextBlock tipText;
    private readonly Button hideTip;
    private readonly Border tipCard;
    private readonly ScrollViewer scroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly CancellationTokenSource lifetime = new();
    private readonly TextBlock slashHint = new() { FontSize = 11, TextWrapping = TextWrapping.Wrap, Visibility = Visibility.Collapsed };
    private readonly ListView commandChoices = new() { MaxHeight = 170, SelectionMode = ListViewSelectionMode.Single,
        IsItemClickEnabled = true, Visibility = Visibility.Collapsed };
    private bool composing, compositionJustEnded, commandsDismissed;
    private string? attachmentFingerprint;
    private string? pickerSession;
    private bool disposed, followLatest = true;
    private bool userScrollPending;
    private readonly Button follow = new() { Content = "回到最新", Visibility = Visibility.Collapsed };
    private readonly Dictionary<string, (string Fingerprint, FrameworkElement Element, bool TerminalRun, string? ProcessKey)> renderedRows = [];
    private readonly NativeGeneratedArtifacts generatedOutputs = new();
    private readonly Dictionary<string, bool> generatedChoices = [];
    private readonly Action<NativeGeneratedOutput>? openGenerated;
    private readonly Func<string>? generatedProjectRoot;
    private readonly Dictionary<string, ActivityDisclosure> activityDisclosures = [];
    private string? approvalFingerprint;
    private string? readingSession;
    private readonly Func<string, bool>? slashCommand;
    private readonly Action? openHosts;
    private readonly Action<string>? openRun;
    private readonly Func<Task<string?>>? pickAttachment;
    private readonly Func<Task<IReadOnlyList<string>>>? pickAttachments;
    private bool CanPickAttachments => pickAttachment != null || pickAttachments != null;
    private readonly Func<string, Task>? createAcp;
    private readonly Func<string, string, string, Task>? openHistoryBranch;
    private readonly Func<Task<bool>> ensureSession;
    private readonly Button composerSettingsButton = new();
    private bool preparingSession;
    private void RefreshFastButton()
    {
        var state = model.ComposerFastMode;
        fastMode.Visibility = state == null ? Visibility.Collapsed : Visibility.Visible;
        if (state == null) return;
        var on = state.Enabled;
        var tip = design.T(model.Snapshot?.Running == true ? "composer.fast.running"
            : model.SavingFastMode ? "composer.fast.saving"
            : (on, state.Inherited) switch { (true, true) => "composer.fast.on_profile", (true, false) => "composer.fast.on_session",
                (false, true) => "composer.fast.off_profile", _ => "composer.fast.off_session" });
        AutomationProperties.SetName(fastMode, tip); ToolTipService.SetToolTip(fastMode, tip);
        fastMode.IsChecked = on; fastMode.IsEnabled = model.CanChangeFastMode && !preparingSession;
        fastMode.Opacity = model.SavingFastMode ? .62 : 1;
        var fill = design.Brush(on ? "warn-soft" : "bg-elev");
        var edge = on ? MixBrush("warn", "border-strong", .52) : design.Brush("border-strong");
        fastMode.BorderThickness = new(1); fastMode.Background = fill; fastMode.BorderBrush = edge;
        foreach (var suffix in new[] { "Checked", "CheckedPointerOver", "CheckedPressed", "CheckedDisabled", "Disabled" })
        {
            fastMode.Resources["ToggleButtonBackground" + suffix] = fill;
            fastMode.Resources["ToggleButtonBorderBrush" + suffix] = edge;
        }
        fastMode.Resources["ToggleButtonBackgroundPointerOver"] = design.Brush("surface-hover");
        fastMode.Content = design.Icon("bolt", fastMode.Width == 28 ? 15 : 17, design.ColorText(on ? "warn" : "text-faint"), filled: on);
    }
    private void RefreshSendAppearance()
    {
        var hasContent = model.HasComposerContent;
        var showStop = model.ShowStopButton;
        stop.Visibility = showStop ? Visibility.Visible : Visibility.Collapsed;
        send.Visibility = showStop ? Visibility.Collapsed : Visibility.Visible;
        stop.IsEnabled = model.CanStop;
        send.IsEnabled = !preparingSession && !clipboardBusy && (model.CanSubmit
            || !hasContent && model.CanCompose && (model.CanPrepareProjectDraft || model.CanAttach));
        var sendLabel = design.T(model.Snapshot?.Running == true ? "composer.queue_button" : "composer.send");
        AutomationProperties.SetName(send, sendLabel); ToolTipService.SetToolTip(send, sendLabel);
        var stopLabel = design.T(model.IsStopping ? "composer.stopping" : "composer.stop");
        AutomationProperties.SetName(stop, stopLabel); ToolTipService.SetToolTip(stop, stopLabel);
        stopFill.Color = design.Brush("text").Color;
        stop.Content = design.Icon("stop", stop.Width == 28 ? 16 : 18, design.ColorText("bg-app"));
        stop.Opacity = stop.IsEnabled ? 1 : .45;
        var fill = hasContent ? design.Brush("clay") : MixBrush("text", "bg-app", .14);
        var hover = hasContent ? design.Brush("clay-strong") : fill;
        send.Background = fill; send.Resources["ButtonBackgroundDisabled"] = fill;
        send.Resources["ButtonBackgroundPointerOver"] = hover; send.Resources["ButtonBackgroundPressed"] = hover;
        send.Opacity = send.IsEnabled ? 1 : .45;
        send.Content = design.Icon("arrow-up", send.Width == 28 ? 16 : 18, design.ColorText(hasContent ? "on-clay" : "text-faint"));
        send.BorderThickness = new Thickness(0);
    }
    private async Task<bool> EnsureSessionForActionAsync()
    {
        if (disposed || preparingSession) return false;
        preparingSession = true; Refresh();
        try { return await ensureSession(); }
        finally { preparingSession = false; Refresh(); }
    }

    public NativeConversationPage(WorkspaceConversationModel model, WispDesign design, Action<string> quote, Func<Task> create,
        Func<Task<string?>>? pickAttachment = null, Func<string, bool>? slashCommand = null, Action? openHosts = null,
        Action<string>? openRun = null, Func<string, Task>? createAcp = null,
        Func<string, string, string, Task>? openHistoryBranch = null, Func<string, Task>? reviewRun = null, Func<bool>? canShowRunReview = null,
        Func<Task<bool>>? ensureSession = null, Func<Task<IReadOnlyList<string>>>? pickAttachments = null, Action<string>? composeNavigate = null,
        Func<NativeSessionReferenceScope?>? sessionReferenceScope = null,
        Action<string>? openBranch = null, Action<string, string>? openBranchResult = null, Action<NativeExplorationSummary>? reviewExploration = null,
        Action<NativeHistoryTarget>? startExploration = null, Func<string, string, string, Task>? openExcerpt = null,
        Func<NativeQueueTarget, string, bool>? sendQueuedSideChat = null, Func<bool>? canSendQueuedSideChat = null, Action? openAgentSettings = null, Action? openComputeSettings = null, Action? addComputeHost = null, Action? openModelSettings = null,
        Func<bool>? modelSwitchWarningDisabled = null, Action? disableModelSwitchWarning = null,
        Func<NativeHandoffLaunch, Task>? startHandoff = null, Func<NativeContextRecoveryLaunch, Task>? startContextRecovery = null,
        Func<bool>? canShowContextRecovery = null, Action<NativeGeneratedOutput>? openGenerated = null, Func<string>? generatedProjectRoot = null)
    {
        this.openGenerated = openGenerated; this.generatedProjectRoot = generatedProjectRoot;
        this.model = model; this.design = design; this.quote = quote; this.create = create;
        this.startHandoff = startHandoff;
        this.startContextRecovery = startContextRecovery;
        this.canShowContextRecovery = canShowContextRecovery;
        model.Handoff.Changed += RefreshHandoffDialog;
        this.ensureSession = ensureSession ?? (() => Task.FromResult(model.Snapshot is { ReadOnly: false }));
        this.slashCommand = slashCommand; this.openHosts = openHosts; this.pickAttachment = pickAttachment;
        this.pickAttachments = pickAttachments;
        this.openAgentSettings = openAgentSettings;
        this.openComputeSettings = openComputeSettings;
        this.addComputeHost = addComputeHost;
        this.openModelSettings = openModelSettings;
        this.modelSwitchWarningDisabled = modelSwitchWarningDisabled; this.disableModelSwitchWarning = disableModelSwitchWarning;
        this.composeNavigate = composeNavigate;
        this.sessionReferenceScope = sessionReferenceScope;
        this.openBranch = openBranch; this.openBranchResult = openBranchResult;
        this.reviewExploration = reviewExploration;
        this.startExploration = startExploration;
        this.openExcerpt = openExcerpt;
        this.sendQueuedSideChat = sendQueuedSideChat; this.canSendQueuedSideChat = canSendQueuedSideChat;
        this.openRun = openRun;
        this.reviewRun = reviewRun;
        this.canShowRunReview = canShowRunReview;
        this.createAcp = createAcp;
        this.openHistoryBranch = openHistoryBranch;
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        VerticalContentAlignment = VerticalAlignment.Stretch;
        design.BindTypography(status, 12); design.BindTypography(hint, 11);
        var root = new Grid();
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        status.Margin = new Thickness(24, 12, 24, 0);
        retry.Click += async (_, _) =>
        {
            model.ShowCompaction();
            if (model.HasProjectDraft) await model.ReloadComposerCatalogAsync(lifetime.Token);
            if (model.Effort.Error != null) await model.Effort.ReloadAsync(lifetime.Token);
            await model.RefreshAsync(lifetime.Token);
        };
        acknowledge.Click += (_, _) => { if (model.DraftCreationUncertain) model.AcknowledgeDraftCreation(); else model.AcknowledgeUncertainSend(); };
        var banner = retryActions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, Margin = new Thickness(24, 8, 24, 0) };
        banner.Children.Add(retry); banner.Children.Add(acknowledge);
        acknowledgeHistory.Click += (_, _) => model.AcknowledgeHistoryResult();
        banner.Children.Add(acknowledgeHistory);
        var header = new StackPanel(); header.Children.Add(status); header.Children.Add(banner);
        root.Children.Add(header);
        scroll.Content = transcript; Grid.SetRow(scroll, 1); root.Children.Add(scroll);
        scroll.SizeChanged += (_, e) => transcript.Width = PreviewLayout.ConversationThreadWidth(e.NewSize.Width);
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
            model.Draft = composer.Text;
            RefreshSendAppearance();
            commandsDismissed = false;
            UpdateSlashHint();
        };
        composer.SelectionChanged += (_, _) => { if (!composing && !commandsDismissed) UpdateSlashHint(); };
        composer.TextCompositionStarted += (_, _) => composing = true;
        composer.TextCompositionEnded += (_, _) =>
        {
            composing = false; compositionJustEnded = true;
            DispatcherQueue.TryEnqueue(() => compositionJustEnded = false);
        };
        composer.Paste += (_, e) => { if (TryPasteComposerContent()) e.Handled = true; };
        ConfigureComposerClipboardMenu();
        composer.PreviewKeyDown += (_, e) =>
        {
            if (composing || compositionJustEnded) return;
            var shift = (InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Shift) & Windows.UI.Core.CoreVirtualKeyStates.Down) != 0;
            var control = (InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control) & Windows.UI.Core.CoreVirtualKeyStates.Down) != 0;
            if ((control && !shift && e.Key == VirtualKey.V || shift && !control && e.Key == VirtualKey.Insert) && TryPasteComposerContent())
            { e.Handled = true; return; }
            if (commandChoices.Visibility == Visibility.Visible && !control && !shift)
            {
                if (e.Key is VirtualKey.Up or VirtualKey.Down)
                {
                    var direction = e.Key == VirtualKey.Down ? 1 : -1;
                    commandChoices.SelectedIndex = (commandChoices.SelectedIndex + direction + commandChoices.Items.Count) % commandChoices.Items.Count;
                    commandChoices.ScrollIntoView(commandChoices.SelectedItem); e.Handled = true; return;
                }
                if (e.Key is VirtualKey.Enter or VirtualKey.Tab && commandChoices.SelectedItem is { } selected)
                { ChooseComposerItem(selected); e.Handled = true; return; }
            }
            if (e.Key == VirtualKey.Enter && NativeComposerCommands.ShouldSubmit(control, shift, composing || compositionJustEnded, model.InputPreferences.SendWithModifier))
            { e.Handled = true; Submit(); }
        };
        commandChoices.ItemClick += (_, e) => ChooseComposerItem(e.ClickedItem);
        AutomationProperties.SetName(commandChoices, "命令与引用候选，方向键选择，Enter 填入");
        send.Click += (_, _) => Submit();
        stop.Click += async (_, _) => await model.StopAsync(lifetime.Token);
        BuildModelPicker();
        createSession.Click += async (_, _) => await create();
        BuildComposeMenu();
        attach.Content = design.Icon("plus", 18);
        AutomationProperties.SetName(attach, design.T("composer.add")); ToolTipService.SetToolTip(attach, design.T("composer.add"));
        AutomationProperties.SetAutomationId(attach, "composer-add");
        var options = composerSettingsButton; options.Content = design.Icon("controls", 18);
        AutomationProperties.SetName(options, design.L("对话选项", "Agent options")); ToolTipService.SetToolTip(options, design.L("对话选项", "Agent options"));
        BuildComposerOptions();
        composerOptions.Opened += async (_, _) => { composerOptionsOpen = true; refreshComposerFocus?.Invoke(); await model.Options.LoadAsync(lifetime.Token); };
        composerOptions.Closed += (_, _) =>
        {
            composerOptionsOpen = false; refreshComposerFocus?.Invoke(); optionSubmenu.Hide();
            if (!restoreComposerOptionsTrigger) return;
            restoreComposerOptionsTrigger = false;
            DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
            {
                if (!disposed && !composerOptionsOpen && composerSettingsButton.IsLoaded)
                    composerSettingsButton.Focus(FocusState.Keyboard);
            });
        };
        options.Click += (_, _) =>
        {
            modelMenu.Hide(); effortMenu.Hide();
            var ready = model.Snapshot != null && model.CurrentSessionId != null || model.HasProjectDraft && !model.Loading && !model.Busy;
            // Both menus start at the left edge of the same toolbar in WebView.
            if (ready && !disposed && attach.IsLoaded) composerOptions.ShowAt(attach);
        };
        AutomationProperties.SetAutomationId(fastMode, "composer-fast-toggle");
        fastMode.Click += async (_, _) =>
        {
            if (model.ComposerFastMode is { } fast) await model.SetFastModeAsync(!fast.Enabled, lifetime.Token);
        };
        send.Content = design.Icon("arrow-up", 18);
        AutomationProperties.SetAutomationId(send, "composer-send"); AutomationProperties.SetAutomationId(stop, "composer-stop");
        foreach (var button in new[] { attach, options, send, stop })
        {
            design.QuietButton(button);
            button.Width = button.Height = button.MinWidth = button.MinHeight = 32;
            button.Padding = new Thickness(6); button.CornerRadius = new CornerRadius(16);
            button.BorderThickness = new Thickness(1); button.BorderBrush = design.Brush("border");
            button.VerticalAlignment = VerticalAlignment.Center;
        }
        fastMode.Width = fastMode.Height = fastMode.MinWidth = fastMode.MinHeight = 32;
        fastMode.Padding = new Thickness(6); fastMode.CornerRadius = new CornerRadius(16);
        fastMode.VerticalAlignment = VerticalAlignment.Center;
        stop.BorderThickness = new Thickness(0);
        stop.Background = stopFill; stop.Resources["ButtonBackgroundDisabled"] = stopFill;
        stop.Resources["ButtonBackgroundPointerOver"] = stop.Resources["ButtonBackgroundPressed"] = design.Brush("text");
        models.HorizontalAlignment = HorizontalAlignment.Stretch;
        foreach (var picker in new Control[] { models, efforts })
        {
            picker.MinWidth = 0;
            picker.Height = picker.MinHeight = 32; picker.CornerRadius = new CornerRadius(16);
            picker.VerticalAlignment = VerticalAlignment.Center;
        }
        BuildEffortPicker();
        var leading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        leading.Children.Add(attach); leading.Children.Add(options);
        SetupReading();
        var primary = new Grid { VerticalAlignment = VerticalAlignment.Center };
        primary.Children.Add(send); primary.Children.Add(stop);
        var actions = new NativeComposerToolbar(leading, models, efforts) { Margin = new Thickness(0, 8, 0, 0) };
        foreach (var c in new UIElement[] { leading, contextUsage, models, efforts, fastMode, primary }) actions.Children.Add(c);
        root.SizeChanged += (_, e) =>
        {
            var compact = PreviewLayout.CompactComposer(e.NewSize.Width);
            var viewportWidth = XamlRoot?.Size.Width ?? e.NewSize.Width;
            actions.Compact = compact; leading.Spacing = compact ? 4 : 6;
            foreach (var b in new ContentControl[] { attach, options, fastMode, send, stop })
            {
                b.Width = b.Height = b.MinWidth = b.MinHeight = compact ? 28 : 32;
                if (b.Content is Image glyph) glyph.Width = glyph.Height = compact ? (b == send || b == stop ? 16 : 15) : b == fastMode ? 17 : 18;
            }
            models.Height = models.MinHeight = efforts.Height = efforts.MinHeight = contextUsage.Height = contextUsage.MinHeight = compact ? 28 : 32;
            models.MaxWidth = compact ? 104 : Math.Min(viewportWidth <= 640 ? 126 : 220, viewportWidth * .38);
            efforts.MaxWidth = compact ? 104 : Math.Min(180, viewportWidth * .3);
            models.Padding = new(compact ? 8 : 11, 0, compact ? 8 : 11, 0);
            efforts.Padding = new(compact ? 8 : 10, 0, compact ? 8 : 10, 0);
        };
        var card = new StackPanel { Spacing = 0, Padding = new Thickness(14, 0, 12, 10) };
        composerResize = new(() => design.ComposerTextHeight, ApplyComposerHeight, height =>
        {
            design.ComposerTextHeight = height;
            ApplyComposerHeight(height); design.SaveComposerTextHeight?.Invoke(height); FollowAfterLayout();
        });
        AutomationProperties.SetAutomationId(composerResize, "composer-resizer");
        card.Children.Add(composerResize);
        InitializeQueue();
        composer.MinHeight = 50;
        composer.BorderThickness = new Thickness(0);
        composer.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        composer.Padding = new Thickness(0, 4, 0, 6);
        composer.Resources["TextControlBackgroundFocused"] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        composer.Resources["TextControlBackgroundPointerOver"] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        composer.Resources["TextControlBorderBrushFocused"] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        composer.Resources["TextControlBorderBrushPointerOver"] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        ApplyComposerHeight(design.ComposerTextHeight);
        design.QuietButton(assistantContext);
        var contextContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 7 };
        contextContent.Children.Add(design.Icon("folder", 14)); contextContent.Children.Add(assistantContextLabel); contextContent.Children.Add(design.Icon("close", 14));
        assistantContext.Content = contextContent;
        assistantContext.Click += (_, _) => model.SetAssistantContext(null, model.ContextReady);
        card.Children.Add(followUpQuestions); card.Children.Add(commandChoices); card.Children.Add(attachments); card.Children.Add(assistantContext); card.Children.Add(composer);
        design.BindTypography(slashHint, 11);
        slashHint.Foreground = design.Brush("text-faint");
        slashHint.Text = "方向键选择 · Enter 填入 · Ctrl+Enter 执行";
        slashHint.Visibility = Visibility.Collapsed;
        follow.Click += (_, _) => { followLatest = true; userScrollPending = false; FollowAfterLayout(); UpdateFollowButton(); };
        var footer = new Grid { Margin = new Thickness(20, 0, 20, 0), MaxWidth = PreviewLayout.ConversationMaxWidth };
        footer.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        footer.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var footerHints = new StackPanel { Spacing = 2 };
        footerHints.Children.Add(slashHint);
        hint.Text = model.InputPreferences.LocalizedHint(design.Localization);
        hint.Foreground = design.Brush("text-faint");
        hint.HorizontalAlignment = HorizontalAlignment.Stretch; hint.TextAlignment = TextAlignment.Center;
        design.BindLineHeight(hint, 11, 1.35); hint.Margin = new Thickness(54, 2, 2, 0); hint.MinHeight = 20;
        hint.TextTrimming = TextTrimming.CharacterEllipsis; hint.Opacity = 0;
        card.Children.Add(hint); card.Children.Add(actions);
        footer.Children.Add(footerHints); Grid.SetColumn(follow, 1); footer.Children.Add(follow);
        composerBar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        composerBar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        composerBar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        // Welcome belongs to the flexible reading region, not the bottom composer.
        var welcome = new Grid { Padding = new Thickness(16, 26, 16, 40) };
        var empty = new StackPanel { Spacing = 12, MaxWidth = PreviewLayout.ConversationMaxWidth, HorizontalAlignment = HorizontalAlignment.Stretch,
            VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(24, 40, 24, 40) };
        welcomeMark = design.Wordmark();
        welcomeMark.Width = 208; welcomeMark.Height = 208 * HomeLayout.WordmarkHeight / HomeLayout.WordmarkWidth;
        welcomeMark.HorizontalAlignment = HorizontalAlignment.Center;
        AutomationProperties.SetName(welcomeMark, "Wisp Science"); empty.Children.Add(welcomeMark);
        emptyHeading = design.Text("", PreviewLayout.WelcomeHeadingSize);
        emptyHeading.FontSize = PreviewLayout.WelcomeHeadingSize;
        emptyHeading.FontFamily = new FontFamily("ms-appx:///Assets/Fonts/wisp-serif.ttf#Wisp Serif, Noto Serif SC, SimSun, Georgia, Times New Roman");
        emptyHeading.Margin = new Thickness(0, 4, 0, 0);
        emptyHeading.TextAlignment = TextAlignment.Center; emptyHeading.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        emptyHeading.Foreground = design.Brush("text");
        empty.Children.Add(emptyHeading);
        emptyDescription = design.Text("", 14);
        // .empty p uses the fixed --text-md token, not the scalable body size.
        emptyDescription.FontSize = 14;
        emptyDescription.MaxWidth = 420;
        emptyDescription.TextAlignment = TextAlignment.Center; emptyDescription.Foreground = design.Brush("text-muted");
        emptyDescription.LineStackingStrategy = LineStackingStrategy.MaxHeight;
        emptyDescription.LineHeight = 14 * 1.55;
        empty.Children.Add(emptyDescription);
        var tip = new Grid { ColumnSpacing = 8 };
        tip.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        tip.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        tip.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        tip.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var tipIcon = design.Icon("lightbulb", 16); tipIcon.VerticalAlignment = VerticalAlignment.Top; tipIcon.Margin = new Thickness(0, 2, 0, 0);
        tip.Children.Add(tipIcon);
        tipLabel = design.Text("", 13); tipLabel.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        tipLabel.Foreground = design.Brush("text"); design.BindLineHeight(tipLabel, 13, 1.5);
        Grid.SetColumn(tipLabel, 1); tip.Children.Add(tipLabel);
        tipText = design.Text("", 13); tipText.Foreground = design.Brush("text-muted");
        design.BindLineHeight(tipText, 13, 1.5);
        Grid.SetColumn(tipText, 2); tip.Children.Add(tipText);
        hideTip = design.ToolButton("", "close"); hideTip.Padding = new Thickness(2, 0, 2, 0);
        hideTip.Width = hideTip.Height = 20; hideTip.MinWidth = hideTip.MinHeight = 0;
        hideTip.Content = design.Icon("close", 16);
        hideTip.VerticalAlignment = VerticalAlignment.Top;
        hideTip.Click += (_, _) => design.Welcome.DismissTips();
        Grid.SetColumn(hideTip, 3); tip.Children.Add(hideTip);
        tipCard = new Border { Child = tip, MaxWidth = 460, CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(1),
            BorderBrush = design.Brush("border"), Padding = new Thickness(12, 8, 8, 8), Margin = new Thickness(0, 8, 0, 0) };
        AutomationProperties.SetAutomationId(tipCard, "usage-tip"); empty.Children.Add(tipCard);
        design.ActionButton(createSession, primary: true);
        if (createSession.Content is TextBlock startLabel) startLabel.Foreground = new SolidColorBrush(Microsoft.UI.Colors.White);
        createSession.HorizontalAlignment = HorizontalAlignment.Center;
        createSession.Margin = new Thickness(0, 8, 0, 0); empty.Children.Add(createSession);
        welcome.Children.Add(new NativeWelcomeBackdrop(design, empty)); welcomeScroll.Content = welcome;
        welcomeScroll.SizeChanged += (_, e) => welcome.MinHeight = Math.Max(0, e.NewSize.Height);
        AutomationProperties.SetAutomationId(welcomeScroll, "conversation-welcome");
        Grid.SetRow(welcomeScroll, 1); root.Children.Add(welcomeScroll);
        var border = new Border { Child = card, CornerRadius = new CornerRadius(16), BorderThickness = new Thickness(1),
            BorderBrush = design.Brush("border"), Background = design.Brush("bg-input") };
        var frame = new Grid { Margin = model.AssistantMode ? new Thickness(24, 0, 24, 18) : new Thickness(16, 0, 16, 12),
            MaxWidth = PreviewLayout.ConversationMaxWidth };
        var ring = new Border { Margin = new(-3), BorderThickness = new(3), CornerRadius = new(19), IsHitTestVisible = false };
        frame.Children.Add(ring); frame.Children.Add(border);
        var pointerInsideComposer = false; var fileDragOverComposer = false;
        void UpdateComposerFocus()
        {
            if (disposed) return;
            var focused = XamlRoot == null ? null : FocusManager.GetFocusedElement(XamlRoot) as DependencyObject;
            while (focused != null && focused != border) focused = VisualTreeHelper.GetParent(focused);
            // WebView's toolbar menus are children of the composer. WinUI flyouts
            // own a separate visual tree, but still belong to the same input area.
            var focusWithin = focused != null || composeMenuOpen || composerOptionsOpen;
            hint.Opacity = focusWithin || pointerInsideComposer ? 1 : 0;
            var showRing = focusWithin || fileDragOverComposer;
            ring.BorderBrush = showRing ? design.Brush("focus-ring") : null;
            border.BorderBrush = showRing ? MixBrush("clay", "border-strong", .45) : design.Brush("border");
        }
        refreshComposerFocus = UpdateComposerFocus;
        border.PointerEntered += (_, _) => { pointerInsideComposer = true; UpdateComposerFocus(); };
        border.PointerExited += (_, _) => { pointerInsideComposer = false; UpdateComposerFocus(); };
        border.GotFocus += (_, _) => UpdateComposerFocus();
        border.LostFocus += (_, _) => DispatcherQueue.TryEnqueue(() => UpdateComposerFocus());
        var inputArea = new StackPanel { Spacing = 4, MaxWidth = PreviewLayout.ConversationMaxWidth + 32 };
        inputArea.Children.Add(excerptArea);
        inputArea.Children.Add(planProgress);
        inputArea.Children.Add(queueCard);
        var stopBody = new Grid { ColumnSpacing = 12 };
        stopBody.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); stopBody.ColumnDefinitions.Add(new());
        stoppingSpinner.Foreground = design.Brush("err"); stopBody.Children.Add(stoppingSpinner);
        var stopCopy = new StackPanel { Spacing = 3 };
        stoppingTitle = design.Text("", 13.5); stoppingTitle.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        stoppingDetail = design.Text("", 12); stoppingDetail.Foreground = design.Brush("text-muted");
        design.BindLineHeight(stoppingDetail, 12, 1.4);
        stopCopy.Children.Add(stoppingTitle); stopCopy.Children.Add(stoppingDetail); Grid.SetColumn(stopCopy, 1); stopBody.Children.Add(stopCopy);
        stoppingNotice.Child = stopBody; stoppingNotice.Margin = new Thickness(16, 0, 16, 6); stoppingNotice.Padding = new Thickness(16, 12, 16, 12);
        stoppingNotice.Background = design.Brush("bg-elev"); stoppingNotice.BorderBrush = design.Brush("border-strong");
        stoppingNotice.BorderThickness = new Thickness(1); stoppingNotice.CornerRadius = new CornerRadius(8);
        AutomationProperties.SetAutomationId(stoppingNotice, "stopping-toast");
        AutomationProperties.SetLiveSetting(stoppingTitle, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        inputArea.Children.Add(stoppingNotice);
        inputArea.Children.Add(runtimeStrip);
        inputArea.Children.Add(frame);
        Grid.SetRow(inputArea, 0); composerBar.Children.Add(inputArea);
        Grid.SetRow(footer, 1); composerBar.Children.Add(footer);
        var composerViewport = new ScrollViewer { Content = composerBar, HorizontalScrollMode = ScrollMode.Disabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        void FitComposer()
        {
            composerViewport.MaxHeight = PreviewLayout.ComposerViewportHeight(root.ActualHeight - header.ActualHeight - approvals.ActualHeight, composer.Height);
            FitComposerHint();
        }
        root.SizeChanged += (_, _) => FitComposer(); header.SizeChanged += (_, _) => FitComposer(); approvals.SizeChanged += (_, _) => FitComposer();
        composer.SizeChanged += (_, _) => FitComposer();
        Grid.SetRow(contextUsageSlot, 3); root.Children.Add(contextUsageSlot);
        Grid.SetRow(composerViewport, 4); root.Children.Add(composerViewport);
        // TextBox owns its drag/drop hit target; allow the parent handler to
        // receive workspace paths over the editable surface as well as its rim.
        composer.AllowDrop = true; border.AllowDrop = true;
        EnableFileReferenceDrop(border, active => { fileDragOverComposer = active; UpdateComposerFocus(); });
        Content = root;
        Refresh();
        // Subscribe only after the controls have been constructed successfully.
        // A failed constructor must not leave a half-built page observing resets.
        model.Changed += Refresh;
        model.ModelContext.Changed += ModelContextChanged;
        model.Effort.Changed += Refresh;
        model.Runtimes.Changed += RefreshRuntimeStrip;
        model.Runtimes.Inspector.Changed += RefreshRuntimeEnvironment;
        Unloaded += (_, _) => DismissRuntimeSurfaces();
        Unloaded += (_, _) => { CloseIdleCompactionDialog(); CloseContextRecoveryDialog(); };
        Loaded += (_, _) => { RefreshContextRecoveryDialog(); RefreshIdleCompactionDialog(); };
        model.Options.Changed += Refresh;
        model.ReferencePicker.Changed += RenderComposerChoices;
        design.TypographyChanged += Refresh;
        design.Welcome.Changed += Refresh;
        if (!model.ExampleMode) _ = PollAsync();
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
                await model.Runtimes.RefreshAsync(lifetime.Token);
            }
        }
        catch (OperationCanceledException) { }
    }

    private async Task DoAttachAsync(string? commandDraft = null)
    {
        if (clipboardBusy || !CanPickAttachments || !(model.CanAttach || model.CanPrepareProjectDraft)) return;
        var project = model.CurrentProjectId; var session = model.CurrentSessionId;
        IReadOnlyList<string> paths;
        try { paths = pickAttachments != null ? await pickAttachments() : await pickAttachment!() is { } path ? [path] : []; }
        catch (Exception ex)
        {
            if (!disposed && model.CurrentProjectId == project && model.CurrentSessionId == session)
                model.ReportAttachmentSelectionError(design.L("无法选择附件：", "Could not choose attachments: ") + ex.Message);
            return;
        }
        if (paths.Count == 0 || disposed || model.CurrentSessionId != session
            || model.CurrentProjectId != project || !await EnsureSessionForActionAsync() || disposed || !model.CanAttach) return;
        var attachedSession = model.CurrentSessionId;
        var attached = await model.AttachFilesAsync(paths, lifetime.Token);
        if (commandDraft != null && attached > 0 && model.OperationError == null && model.Draft == commandDraft
            && model.CurrentSessionId == attachedSession && model.CurrentProjectId == project)
        { model.Draft = ""; Refresh(); }
    }

    private void UpdateSlashHint()
    {
        var token = !commandsDismissed && !composing && model.CanAttach && model.Snapshot?.ComposerReferences == true
            ? NativeComposerToken.Find(composer.Text, composer.SelectionStart) : null;
        if (token != model.ReferencePicker.Token)
        {
            if (token == null) model.ReferencePicker.Dismiss();
            else _ = model.ReferencePicker.SearchAsync(token, lifetime.Token);
        }
        RenderComposerChoices();
    }

    private void RenderComposerChoices()
    {
        if (disposed) return;
        var choices = new List<object>();
        if (!commandsDismissed && composer.IsEnabled && !composing)
        {
            choices.AddRange(NativeComposerCommands.Match(composer.Text, CanPickAttachments, slashCommand != null, model.CanCompose)
                .Select(command => command.Command == "/compact" ? command with { Label = design.T("compact.title") } : command));
            if (model.ReferencePicker.Token == NativeComposerToken.Find(composer.Text, composer.SelectionStart))
                choices.AddRange(model.ReferencePicker.Options);
        }
        var selected = commandChoices.SelectedItem;
        commandChoices.ItemsSource = choices;
        commandChoices.SelectedIndex = choices.Count == 0 ? -1 : Math.Max(0, choices.IndexOf(selected));
        commandChoices.Visibility = choices.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        slashHint.Visibility = !commandsDismissed && (model.ReferencePicker.Token != null || composer.Text.TrimStart().StartsWith('/'))
            ? Visibility.Visible : Visibility.Collapsed;
        slashHint.Text = model.ReferencePicker.Error ?? (model.ReferencePicker.Loading ? design.L("正在查找引用…", "Looking up references…")
            : choices.Count > 0 ? design.L("方向键选择 · Enter/Tab 添加引用或填入命令 · Esc 关闭", "Arrow keys to select · Enter/Tab to insert · Esc to close")
            : design.L("没有匹配项；草稿已保留。输入 @、# 或 / 查找引用与命令。", "No matches; your draft is kept. Type @, # or / to find references and commands."));
    }

    private void ChooseComposerItem(object choice)
    {
        if (choice is NativeComposerCommand command) { ChooseCommand(command); return; }
        if (choice is not NativeReferenceOption reference || model.ReferencePicker.Token is not { } token
            || token != NativeComposerToken.Find(composer.Text, composer.SelectionStart)) return;
        var draft = composer.Text;
        if (!model.AddReference(reference)) return;
        composer.Text = token.RemoveFrom(draft);
        composer.SelectionStart = token.Start;
        commandsDismissed = true;
        model.ReferencePicker.Dismiss();
        RenderComposerChoices();
        composer.Focus(FocusState.Programmatic);
    }

    private void ChooseCommand(NativeComposerCommand command)
    {
        composer.Text = command.Command;
        composer.SelectionStart = composer.Text.Length;
        commandsDismissed = true; commandChoices.Visibility = Visibility.Collapsed;
        model.ReferencePicker.Dismiss();
        slashHint.Text = model.InputPreferences.SendWithModifier ? design.L("Ctrl+Enter 执行命令", "Ctrl+Enter to run the command") : design.L("Enter 执行命令", "Enter to run the command");
        composer.Focus(FocusState.Programmatic);
    }

    public bool HandleEscape()
    {
        if (HandleContextRecoveryEscape()) return true;
        if (HandleIdleCompactionEscape()) return true;
        if (HandleHandoffEscape()) return true;
        if (HandleHistoryEscape()) return true;
        if (HandleCompactionEscape()) return true;
        if (HandleModelSwitchEscape()) return true;
        if (HandleRuntimeEscape()) return true;
        if (HandleComposerTextEscape()) return true;
        if (HandleAttachmentEscape()) return true;
        if (HandleReadingEscape()) return true;
        if (HandleAcpEscape()) return true;
        if (HandleComposeMenuEscape()) return true;
        if (HandleComposerOptionsEscape()) return true;
        if (effortMenuOpen) { effortMenu.Hide(); return true; }
        if (composing || compositionJustEnded) return false;
        if (modelMenuOpen) { modelMenu.Hide(); models.Focus(FocusState.Programmatic); return true; }
        if (HandleContextUsageEscape()) return true;
        if (commandChoices.Visibility != Visibility.Visible && model.ReferencePicker.Token == null) return CancelExcerptSelection() || HandleCompactionRecordEscape();
        commandsDismissed = true; commandChoices.Visibility = slashHint.Visibility = Visibility.Collapsed;
        model.ReferencePicker.Dismiss();
        return true;
    }

    /// <summary>Send attempt. A leading slash routes to a client-side command
    /// mapping instead of the send API; unknown commands keep the draft and
    /// surface the available set. Real sends are untouched.</summary>
    private void Submit() => _ = SubmitAsync();
    private async Task SubmitAsync()
    {
        if (preparingSession || clipboardBusy || !model.HasComposerContent || !(model.CanSubmit || model.CanAttach)) return;
        var draft = model.Draft.Trim();
        if (draft.StartsWith('/'))
        {
            if (model.HandleCompactionCommand(model.Draft)) { commandsDismissed = true; commandChoices.Visibility = slashHint.Visibility = Visibility.Collapsed; Refresh(); return; }
            var command = NativeComposerCommands.Exact(draft)?.Command;
            if (command == null) { commandsDismissed = false; UpdateSlashHint(); return; }
            if (command == "/upload" && CanPickAttachments)
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
        if (model.Snapshot?.Running == true) { await model.QueueAsync(lifetime.Token); return; }
        if (await EnsureSessionForActionAsync() && !disposed && model.CanSend) await model.SendAsync(lifetime.Token);
    }

    private void ApplyComposerHeight(double? height)
    {
        composer.MaxHeight = height.HasValue ? PreviewLayout.ComposerMaxTextHeight : PreviewLayout.ComposerAutoHeightLimit;
        composer.Height = height.HasValue ? PreviewLayout.ResizeComposer(height, 0) : double.NaN;
    }
    private void FitComposerHint() => hint.Visibility = model.Snapshot?.ReadOnly == true || XamlRoot?.Size.Width <= 640
        ? Visibility.Collapsed : Visibility.Visible;

    public void Refresh()
    {
        if (disposed) return;
        RefreshHistoryActions();
        RefreshExcerpt();
        follow.Content = design.T("chat.jump_bottom");
        hint.Text = model.InputPreferences.LocalizedHint(design.Localization);
        FitComposerHint();
        if (!composerResize.Dragging) ApplyComposerHeight(design.ComposerTextHeight);
        var resizeHint = design.T("composer.resize_hint");
        if (AutomationProperties.GetName(composerResize) != resizeHint)
        {
            AutomationProperties.SetName(composerResize, resizeHint);
            ToolTipService.SetToolTip(composerResize, resizeHint);
        }
        composer.PlaceholderText = model.Snapshot?.Exploration?.SourceFrozen == true ? design.T("exploration.mainline_frozen_placeholder")
            : model.AssistantMode ? design.L("聊聊研究进展，或安排下一步…", "Ask about your research, or plan the next step…")
            : design.Localization.Format("composer.placeholder", ("modifier", "Ctrl"));
        assistantContext.Visibility = model.ContextReference == null ? Visibility.Collapsed : Visibility.Visible;
        assistantContextLabel.Text = model.ContextReference?.Label ?? "";
        AutomationProperties.SetName(assistantContext, design.L("清除项目上下文：", "Clear project context: ") + model.ContextReference?.Label);
        AutomationProperties.SetName(composer, composer.PlaceholderText);
        if (createSession.Content is TextBlock startLabel) startLabel.Text = design.T("sidebar.new_session");
        else createSession.Content = design.T("sidebar.new_session");
        AutomationProperties.SetName(createSession, design.T("sidebar.new_session"));
        var session = model.Snapshot is { } snapshot ? snapshot.ProjectId + "/" + snapshot.SessionId : null;
        if (session != readingSession)
        {
            optionSubmenu.Hide(); composerOptions.Hide();
            CloseSelectionActions(); CloseContextUsage();
            modelMenu.Hide();
            readingSession = session; followLatest = true; userScrollPending = false;
            design.Welcome.Rotate();
        }
        emptyHeading.Text = model.AssistantMode ? design.T("assistant.title") : design.Welcome.Title(design.Localization);
        emptyHeading.CharacterSpacing = PreviewLayout.WelcomeHeadingTracking(design.Localization.Locale);
        emptyDescription.Text = model.AssistantMode ? design.T("assistant.empty") : design.Welcome.Subtitle(design.Localization);
        tipLabel.Text = design.T("tips.title"); tipText.Text = design.Welcome.Tip(design.Localization);
        tipText.LineHeight = design.FontSize(13) * 1.5;
        AutomationProperties.SetName(hideTip, design.T("tips.close")); ToolTipService.SetToolTip(hideTip, design.T("tips.close"));
        tipCard.Visibility = !model.AssistantMode && design.Welcome.ShowTips ? Visibility.Visible : Visibility.Collapsed;
        FontFamily = design.Font(); FontSize = design.FontSize(14);
        composer.FontFamily = design.Font(); composer.FontSize = design.FontSize(14);
        var inlineTurnError = !model.ShowingHistory && !model.ModelContext.Active && model.Snapshot is { Running: false, Stopping: false };
        var error = model.ConnectionError ?? model.Compaction?.Error ?? (model.HasInlineAttachmentError ? null : ConversationOperationError) ?? model.Effort.Error
            ?? (inlineTurnError ? null : model.Snapshot?.Error);
        if (model.ConnectionError == null && model.QueueUncertain)
            error = design.L("队列操作结果未确认，请核对会话；不会自动重试。", "The queue result is unconfirmed. Check the conversation; it will not be retried automatically.")
                + (model.QueueErrorDetail is { Length: > 0 } detail ? "\n" + detail : "");
        retry.Visibility = error != null ? Visibility.Visible : Visibility.Collapsed;
        retry.Content = design.L("重新读取", "Reload");
        acknowledge.Visibility = model.UncertainSend || model.QueueUncertain || model.DraftCreationUncertain ? Visibility.Visible : Visibility.Collapsed;
        acknowledge.Content = model.DraftCreationUncertain ? design.L("已检查会话列表，允许重新创建", "I checked the session list; allow creation again")
            : design.L("已检查，允许再次发送或排队…", "I checked; allow sending or queuing again…");
        retryActions.Visibility = retryActions.Children.OfType<FrameworkElement>().Any(child => child.Visibility == Visibility.Visible)
            ? Visibility.Visible : Visibility.Collapsed;
        status.Text = error ?? "";
        status.Foreground = design.Brush(error == null ? "text-muted" : "clay-strong");
        status.Visibility = error == null && !model.UncertainSend ? Visibility.Collapsed : Visibility.Visible;
        var hasSession = model.Snapshot != null || model.Loading || model.HasProjectDraft;
        welcomeScroll.Visibility = model.ShowWelcome ? Visibility.Visible : Visibility.Collapsed;
        scroll.Visibility = model.ShowWelcome ? Visibility.Collapsed : Visibility.Visible;
        createSession.Visibility = hasSession || model.AssistantMode || model.ExampleMode ? Visibility.Collapsed : Visibility.Visible;
        createSession.IsEnabled = !model.Busy;
        composerBar.Visibility = hasSession && !model.ExampleMode ? Visibility.Visible : Visibility.Collapsed;
        if (composer.Text != model.Draft) composer.Text = model.Draft;
        composer.IsEnabled = model.CanCompose && !preparingSession;
        RefreshSendAppearance();
        attach.IsEnabled = !preparingSession && !clipboardBusy; RefreshComposeActions();
        composerSettingsButton.IsEnabled = (model.Snapshot != null || model.HasProjectDraft && !model.Loading && !model.Busy) && !preparingSession && !clipboardBusy;
        stoppingSpinner.IsActive = model.IsStopping; stoppingNotice.Visibility = model.IsStopping ? Visibility.Visible : Visibility.Collapsed;
        stoppingTitle.Text = design.T("composer.stopping"); stoppingDetail.Text = design.T("composer.stopping_hint");
        var nextAttachments = JsonSerializer.Serialize(model.Attachments) + JsonSerializer.Serialize(model.References)
            + JsonSerializer.Serialize(model.Quotes) + JsonSerializer.Serialize(model.AttachmentUploads) + $"/{model.CanAttach}/{model.CanRemoveReference}/{model.FileReferenceRevision}/{design.Localization.Locale}/{design.Typography}/{design.Dark}/{design.LightPalette}/{design.DarkPalette}";
        if (attachmentDialog != null && attachmentDialogRevision != model.FileReferenceRevision) attachmentDialog.Hide();
        if (nextAttachments != attachmentFingerprint)
        {
            attachmentFingerprint = nextAttachments;
            attachments.Children.Clear();
            foreach (var file in model.Attachments)
                attachments.Children.Add(AttachmentCard(file));
            foreach (var upload in model.AttachmentUploads)
                attachments.Children.Add(AttachmentCard(upload.Name, null, upload));
            foreach (var reference in model.References)
                attachments.Children.Add(ComposerReferenceCard(reference));
            foreach (var quote in model.Quotes)
                attachments.Children.Add(ComposerQuoteCard(quote));
        }
        attachments.Visibility = model.Attachments.Length > 0 || model.AttachmentUploads.Length > 0 || model.References.Length > 0 || model.Quotes.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        var referenceSession = model.Snapshot?.SessionId + "/" + model.Snapshot?.ComposerReferences + "/" + model.CanAttach;
        if (pickerSession != referenceSession) { pickerSession = referenceSession; UpdateSlashHint(); }
        RefreshModelPicker();
        RefreshEffortPicker();
        RefreshRuntimeStrip();
        RefreshComposerOptions();
        RefreshFastButton();
        if (!composer.IsEnabled) { commandChoices.Visibility = Visibility.Collapsed; slashHint.Visibility = Visibility.Collapsed; }
        RenderPlanProgress();
        RenderQueue();
        RenderTranscript();
        ApplyReadingTarget();
        UpdateRunClocks();
        RefreshContextRecoveryDialog();
        RefreshIdleCompactionDialog();
        _ = MaybeShowRunReviewPrompt();
        RenderApprovals();
        RefreshReading();
        RefreshAcpSettings();
        RefreshCompactionDialog();
        RefreshHandoffDialog();
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
        var modelView = model.ModelContext.Active && !model.ShowingHistory;
        var snapshot = modelView ? model.ModelContext.DisplaySnapshot : model.ShowingHistory ? model.History : model.Snapshot;
        if (!modelView && !model.ShowingHistory && snapshot != null) snapshot = NativeErrorPresentation.WithLiveError(snapshot);
        var keys = snapshot == null ? [] : NativeTranscriptRows.Keys(snapshot).Select(key => modelView ? "context:" + key : key).ToArray();
        generatedOutputs.Update(modelView ? null : snapshot, design.Localization);
        if (generatedChoices.Count > 0)
            foreach (var choice in generatedChoices.Keys.Where(choice => !keys.Any(key => choice == key || choice.StartsWith(key + "/", StringComparison.Ordinal))).ToArray()) generatedChoices.Remove(choice);
        // Model view exposes the working set in raw order, without transcript narrative folding.
        var processGroups = snapshot == null ? [] : NativeTranscriptActivity.DisplayGroups(snapshot.Items, snapshot.Running || snapshot.Stopping, openRun != null, modelView);
        string GroupKey(NativeActivityGroup group) => (group.CompletedTurn ? "activity:" : "steps:") + keys[group.Start];
        int[] Members(NativeActivityGroup group) => NativeTranscriptActivity.Members(snapshot!.Items, group, openRun != null);
        var processRows = processGroups.SelectMany(Members).ToHashSet();
        var style = $"{design.Dark}/{design.LightPalette}/{design.DarkPalette}/{design.Typography}/{design.Localization.Locale}/{model.InputPreferences.SelectionPopupEnabled}";
        if (!modelView && model.Snapshot?.Exploration is { } exploration && (exploration.Summary != null || exploration.SourceFrozen))
        {
            var roundTarget = model.CurrentExplorationRoundTarget;
            var canStartRound = roundTarget != null && model.CanStartExploration(roundTarget);
            var key = JsonSerializer.Serialize(exploration) + style + canStartRound;
            if (key != explorationBannerKey) { explorationBannerKey = key; explorationBanner = new(exploration, design,
                exploration.Summary is { } summary && reviewExploration != null ? () => reviewExploration(summary) : null,
                roundTarget != null && startExploration != null ? () => startExploration(roundTarget) : null, canStartRound); }
            desired.Add(explorationBanner!);
        }
        else { explorationBannerKey = null; explorationBanner = null; }
        var older = !modelView && (model.ShowingHistory ? model.History : model.Snapshot)?.NextBeforeSeq != null;
        if (!modelView && (older || model.ShowingHistory))
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
        if (modelView && (model.ModelContext.Busy || model.ModelContext.Error != null)) desired.Add(ModelContextStatus());
        foreach (var item in snapshot?.Items ?? [])
        {
            var captured = index;
            if (NativeTranscriptRows.RendersNothing(item, modelView)) { index++; continue; }
            var errorMessage = NativeErrorPresentation.Message(item);
            var errorTarget = errorMessage == null || snapshot == null ? null : model.ErrorTarget(snapshot, captured);
            if (!modelView && model.ErrorWasResumed(errorTarget)) { index++; continue; }
            var key = keys[index];
            var outputs = generatedOutputs.At(key);
            var processGroup = processGroups.FirstOrDefault(group => captured >= group.Start && captured < group.End);
            var processKey = processGroup == null ? null : GroupKey(processGroup);
            var stepLive = processGroup?.Live == true;
            var branches = snapshot == null ? [] : NativeBranchResults.At(snapshot, index);
            var compaction = NativeCompactionRecord.Parse(item);
            var compactionTarget = snapshot == null || compaction == null ? null : model.CompactionTarget(snapshot, compaction);
            var exploreTarget = modelView || snapshot == null ? null : model.ExplorationTarget(snapshot, captured);
            var messageActions = modelView || snapshot == null ? [] : model.MessageActions(snapshot, captured, startExploration != null)
                .Where(action => action.Kind != "excerpt" || openExcerpt != null).ToArray();
            var excerptSelected = snapshot != null && item.Role == "user" && model.ExcerptStart is { } excerpt
                && excerpt.Turn == model.HistoryTurn(snapshot, captured);
            var runRecord = item.Run is { } link ? snapshot?.RunCards?.FirstOrDefault(run => run.Id == link.Id && run.FrameId == snapshot.SessionId && run.Status == link.Status) : null;
            var terminalRun = item.Run is { } terminal && WorkspaceConversationModel.RunTerminal(terminal.Status);
            retained.Add(key);
            var fingerprint = JsonSerializer.Serialize(item) + JsonSerializer.Serialize(runRecord) + (runRecord == null ? "" : $"/{model.IsRunHidden(runRecord.Id)}/{model.RunBusy(runRecord.Id)}/{model.RunError(runRecord.Id)}/{model.RunReadError(runRecord.Id)}/{model.CanCancelRun(runRecord)}") + style + $"/{snapshot?.HistoryState != null}"
                + (snapshot == null ? "" : JsonSerializer.Serialize(model.HistoryTurn(snapshot, captured)))
                + JsonSerializer.Serialize(branches)
                + (exploreTarget == null ? "/no-explore" : "/explore/" + model.CanStartExploration(exploreTarget))
                + JsonSerializer.Serialize(messageActions)
                + string.Join("|", outputs.Select(output => output.Id + ":" + output.Name + ":" + output.Superseded))
                + (errorMessage == null ? "" : JsonSerializer.Serialize(errorTarget) + model.ShowsErrorResume(errorTarget) + model.CanResumeError(errorTarget))
                + excerptSelected
                + (compaction == null || snapshot == null ? "" : JsonSerializer.Serialize(model.CompactionUndoOutcome(compactionTarget))
                    + model.CanUndoCompaction(compactionTarget) + (model.CompactionRewindTarget(snapshot, CompactionRecord(snapshot, compaction)) != null))
                + $"/process:{processRows.Contains(captured)}/{processKey}/live:{stepLive}/{snapshot?.Stopping}"
                + (item.Role == "review" ? $"/{snapshot?.UserOffset}/{snapshot?.NextBeforeSeq}/" + JsonSerializer.Serialize(snapshot?.Items.Take(captured + 1)) : "")
                + (item.Role == "plan" ? $"/{model.ShowingHistory}/{model.Snapshot?.Running}/{model.Snapshot?.ReadOnly}/{model.Busy}/{model.ConnectionError}/{model.Snapshot?.PlanMode}/{model.Snapshot?.AcpState?.CurrentMode}/{model.LatestProposal?.Key}"
                    + (model.LatestProposal is { } proposal ? $"/{model.CanDecidePlan(proposal)}/{model.PlanDecisionUncertain(proposal)}" : "") : "")
                + (item.Role == "question" ? $"/{model.ShowingHistory}/{model.Snapshot?.ReadOnly}/{model.Busy}/{model.ConnectionError}/"
                    + JsonSerializer.Serialize(model.Snapshot?.Acp?.QuestionIds) : "");
            if (renderedRows.TryGetValue(key, out var previous) && previous.Fingerprint == fingerprint)
            {
                desired.Add(previous.Element); index++; continue;
            }
            void Add(FrameworkElement element)
            {
                if (previous.Element != null && previous.TerminalRun == terminalRun && NativeToolPresentation.PreserveDisclosureChoice(item))
                {
                    var states = Expanders(previous.Element).Select(expander => expander.IsExpanded).ToArray();
                    var expanders = Expanders(element).ToArray();
                    for (var n = 0; n < Math.Min(states.Length, expanders.Length); n++) expanders[n].IsExpanded = states[n];
                    if (previous.ProcessKey == processKey)
                    {
                        var steps = TranscriptDisclosures(previous.Element).Select(disclosure => disclosure.Choice).ToArray();
                        var nextSteps = TranscriptDisclosures(element).ToArray();
                        for (var n = 0; n < Math.Min(steps.Length, nextSteps.Length); n++) nextSteps[n].Choice = steps[n];
                    }
                }
                if (branches.Length > 0 && openBranch != null && openBranchResult != null)
                {
                    var wrap = new StackPanel(); wrap.Children.Add(element);
                    wrap.Children.Add(NativeBranchResultsView.Links(branches, design, branchDisclosures.GetValueOrDefault(key, true),
                        expanded => branchDisclosures[key] = expanded,
                        id => { if (CurrentBranchScope()) openBranch(id); },
                        (title, summary) => { if (CurrentBranchScope()) openBranchResult(title, summary); }));
                    element = wrap;
                }
                renderedRows[key] = (fingerprint, element, terminalRun, processKey); desired.Add(element);
            }
            bool CurrentBranchScope() => !disposed && snapshot != null && model.CurrentProjectId == snapshot.ProjectId && model.CurrentSessionId == snapshot.SessionId;
            if (errorMessage != null)
            {
                Add(ErrorCard(errorMessage, errorTarget, modelView)); index++; continue;
            }
            if (modelView && ContextSpecialRow(item) is { } contextRow)
            {
                Add(contextRow); index++; continue;
            }
            if (compaction != null && snapshot != null)
            {
                Add(CompactionRow(key, snapshot, compaction)); index++; continue;
            }
            if (NativeBranchResults.IsStandalone(item) && openBranchResult != null)
            {
                var title = NativeBranchResults.Title(item.ToolName, design.T("branch.merged_result"));
                Add(NativeBranchResultsView.Card(item.ToolName, design, () => { if (CurrentBranchScope()) openBranchResult(title, item.Text); }));
                index++; continue;
            }
            if (TranscriptPresentation.UsageSummary(item.Role, item.Text, design.Localization) is { } usage)
            {
                // Usage rows stay bare like the WebView's metadata line: no card, no actions.
                Add(new TextBlock { Text = usage, FontSize = design.FontSize(12),
                    FontFamily = design.Font(), Foreground = design.Brush("text-faint"), Margin = new Thickness(16, 0, 16, 0) });
                index++;
                continue;
            }
            if (snapshot != null && NativeReviewPresentation.Parse(item) is { } reviewReport)
            {
                Add(ReviewCard(snapshot, captured, reviewReport)); index++; continue;
            }
            if (snapshot != null && (item.Role == "reasoning" || item.Role == "assistant" && processRows.Contains(captured) && !string.IsNullOrWhiteSpace(item.Text)))
            {
                Add(ProseStep(item, captured, snapshot, modelView)); index++; continue;
            }
            var plainMessage = item.Role is "user" or "assistant";
            var toolStep = item.Role == "acp_tool" || item.Role == "tool" && (!NativeTranscriptActivity.Completion(item)
                || snapshot != null && NativeTranscriptActivity.RepeatedCompletion(snapshot.Items, captured));
            var card = new StackPanel { Spacing = 8, Padding = new Thickness(plainMessage || toolStep ? 0 : 16) };
            var question = item.Role == "question" ? JsonNode.Parse(item.Text) as JsonObject : null;
            var isAcpQuestion = question != null && NativeAcpQuestion.Parse(question) != null;
            var role = NativeTranscriptLabels.Role(item, design.Localization);
            var roleLabel = design.Text(role, plainMessage ? 11 : 12); roleLabel.Foreground = design.Brush("text-faint");
            if (item.Role != "user" && !toolStep && !isAcpQuestion) card.Children.Add(item.Role == "assistant" || NativeTranscriptActivity.Completion(item) ? AssistantHeader(item) : roleLabel);
            if (item.Role != "user") foreach (var path in item.Attachments ?? []) card.Children.Add(new TextBlock { Text = "附件 · " + path, TextWrapping = TextWrapping.Wrap });
            if (question != null)
            {
                if (!RenderAcpQuestion(card, question))
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
            }
            else if (RenderPlanProposal(card, item, captured) || RenderPlanTool(card, item) || RenderAcpTool(card, item, stepLive)) { }
            else if (item.Role == "tool" && (!NativeTranscriptActivity.Completion(item)
                || NativeTranscriptActivity.RepeatedCompletion(snapshot!.Items, captured)))
            {
                var body = new StackPanel { Spacing = 8 };
                if (runRecord != null && (item.Run?.OwnerIndex == null || item.Run.OwnerIndex == captured))
                {
                    var embedded = WorkspaceConversationModel.RunTerminal(runRecord.Status) && item.Run?.OwnerIndex != null;
                    if (embedded) body.Children.Add(InlineRunCard(runRecord, true));
                    else if (!model.IsRunHidden(runRecord.Id)) card.Children.Add(InlineRunCard(runRecord, false));
                }
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
                if (item.Run != null) body.Children.Add(RawToolDetails(item));
                else
                {
                    if (!string.IsNullOrEmpty(item.Input)) body.Children.Add(ToolText(item.Input));
                    if (!string.IsNullOrEmpty(item.Text)) body.Children.Add(ToolText(item.Text));
                }
                card.Children.Add(ToolStep(item, body, stepLive));
                if (item.Run is { } linkedRun && openRun is not null)
                {
                    var detail = MessageAction("查看运行详情");
                    detail.Click += (_, _) => openRun(linkedRun.Id);
                    card.Children.Add(detail);
                }
            }
            else
            {
                var message = item.Role == "user" ? UserMessageBody(item, modelView) : TranscriptView.Create(new BrowserMessage(captured, item.Role, item.Text, item.ToolName), design);
                if (!modelView && !model.ExampleMode && snapshot != null) AttachSelectionActions(message, snapshot, captured);
                card.Children.Add(message);
                if (!modelView && outputs.Length > 0 && (item.Role == "assistant" || NativeTranscriptActivity.Completion(item)) && openGenerated != null)
                    card.Children.Add(new NativeGeneratedDirectory(outputs, generatedProjectRoot?.Invoke() ?? "", design,
                        output => { if (generatedOutputs.Find(output.Id) is { Superseded: false } current) openGenerated(current); }, generatedChoices, key));
                var actions = new NativeActionWrap { Gap = 2, RowGap = 2, RightAligned = item.Role == "user",
                    Margin = new Thickness(item.Role == "assistant" ? -4 : 0, -4, 0, 0) };
                if (modelView) { }
                else if (plainMessage && snapshot != null)
                    AddMessageActions(actions, snapshot, captured, messageActions);
                else
                {
                    var labels = NativeTranscriptLabels.Actions(design.Localization);
                    var copy = MessageAction(labels.Copy);
                    copy.Click += (_, _) =>
                    {
                        try { var package = new DataPackage { RequestedOperation = DataPackageOperation.Copy }; package.SetText(item.Text); Clipboard.SetContent(package); }
                        catch { }
                    };
                    var quoteButton = MessageAction(labels.AddToChat);
                    quoteButton.Click += (_, _) => { if (snapshot != null) AddQuote(snapshot, captured, item.Text); };
                    var sideQuote = MessageAction(labels.SideChat);
                    sideQuote.Click += (_, _) => quote(snapshot != null && NativeComposerQuote.From(snapshot, captured, item.Text) is { } source ? source.Message : item.Text);
                    var star = MessageAction(labels.Highlight);
                    star.Click += async (_, _) => await model.SaveSelectionAsync(item.Text, lifetime.Token);
                    actions.Children.Add(copy);
                    if (!model.ExampleMode) { actions.Children.Add(quoteButton); actions.Children.Add(sideQuote); actions.Children.Add(star); }
                }
                if (actions.Children.Count > 0) card.Children.Add(actions);
            }
            if (item.Role == "user")
            {
                card.HorizontalAlignment = HorizontalAlignment.Right;
                var rowHost = new Grid(); rowHost.Children.Add(card);
                if (excerptSelected)
                {
                    var outline = new Border { BorderBrush = design.Brush("clay"), BorderThickness = new(2), CornerRadius = new(12),
                        IsHitTestVisible = false };
                    // An outline must not participate in measuring the bubble:
                    // a stretched Border would widen short messages to 760 DIP.
                    var overlay = new Canvas { IsHitTestVisible = false };
                    overlay.Children.Add(outline); rowHost.Children.Add(overlay);
                    void FitOutline()
                    {
                        var parts = card.Children.SelectMany(child => child is NativeActionWrap wrap
                            ? wrap.Children.OfType<FrameworkElement>() : new[] { (FrameworkElement)child })
                            .Where(child => child.IsLoaded && child.ActualWidth > 0).ToArray();
                        if (parts.Length == 0) return;
                        var left = parts.Min(child => child.TransformToVisual(rowHost).TransformPoint(new()).X);
                        var right = parts.Max(child => child.TransformToVisual(rowHost).TransformPoint(new()).X + child.ActualWidth);
                        outline.Width = right - left + 12; outline.Height = card.ActualHeight + 12;
                        Canvas.SetLeft(outline, left - 6); Canvas.SetTop(outline, -6);
                    }
                    rowHost.LayoutUpdated += (_, _) => FitOutline();
                }
                rowHost.SizeChanged += (_, e) => card.MaxWidth = PreviewLayout.UserMessageMaxWidth(e.NewSize.Width);
                Add(rowHost);
            }
            else Add(new Border
            {
                Child = card, CornerRadius = new CornerRadius(12), Padding = new Thickness(plainMessage || toolStep || isAcpQuestion ? 0 : 4),
                BorderBrush = design.Brush("border"), BorderThickness = new Thickness(isAcpQuestion ? 1 : 0),
                Background = design.Brush(isAcpQuestion ? "bg-elev" : "bg-app")
            });
            index++;
        }
        // Keep row controls mounted even when hidden. Toggling a process must
        // not rebuild Markdown, clear selection on other rows or reset tools.
        foreach (var key in retained) renderedRows[key].Element.Visibility = Visibility.Visible;
        var retainedGroups = new HashSet<string>();
        // Release only rows that changed ownership. Stable polls keep the same
        // body, child instances, text selection and keyboard focus mounted.
        foreach (var (key, disclosure) in activityDisclosures)
        {
            var group = processGroups.FirstOrDefault(group => GroupKey(group) == key);
            disclosure.ReleaseExcept(group == null ? [] : Members(group).Select(row => renderedRows[keys[row]].Element).ToArray());
        }
        if (snapshot != null)
        {
            foreach (var group in processGroups)
            {
                var groupKey = GroupKey(group);
                retainedGroups.Add(groupKey);
                if (!activityDisclosures.TryGetValue(groupKey, out var disclosure))
                {
                    disclosure = new ActivityDisclosure(design, group.CompletedTurn);
                    activityDisclosures[groupKey] = disclosure;
                }
                var reviewIds = NativeTranscriptActivity.ReviewRuns(snapshot.Items, group);
                if (!disclosure.ReviewIds.SequenceEqual(reviewIds))
                {
                    disclosure.ReviewIds = reviewIds;
                    disclosure.Reviews.Children.Clear();
                    foreach (var id in reviewIds)
                    {
                        var review = MessageAction(design.L("查看待审阅运行", "Review completed run"));
                        ToolTipService.SetToolTip(review, "Run · " + id);
                        review.Click += (_, _) => openRun?.Invoke(id);
                        disclosure.Reviews.Children.Add(review);
                    }
                }
                disclosure.Reviews.Visibility = reviewIds.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
                disclosure.Rows.Clear();
                foreach (var row in Members(group)) disclosure.Rows.Add(renderedRows[keys[row]].Element);
                disclosure.Update(group, NativeTranscriptActivity.Summary(snapshot.Items, group, groupKey, design.Localization));
                if (model.ScrollTarget is { } target && target >= group.Start && target < group.End
                    && appliedReadingRevision != model.ScrollRevision) disclosure.Disclosure.IsExpanded = true;
                disclosure.MountRows();
                var position = desired.IndexOf(disclosure.Rows[0]);
                foreach (var row in Enumerable.Range(group.Start, group.End - group.Start))
                    if (renderedRows.TryGetValue(keys[row], out var rendered)) desired.Remove(rendered.Element);
                desired.Insert(position, disclosure.Header);
            }
        }
        foreach (var key in activityDisclosures.Keys.Where(key => !retainedGroups.Contains(key)).ToArray()) activityDisclosures.Remove(key);
        if (model.Loading) desired.Add(new ProgressBar { IsIndeterminate = true, Width = 180 });
        else if (!modelView && model.VisibleItems.Length == 0 && hasSession() && model.Snapshot?.Exploration == null && model.Snapshot?.Error == null)
            desired.Add(new TextBlock { Text = design.L("向 Wisp Science 提问，开始这个会话。", "Ask Wisp Science a question to start this conversation."), FontSize = design.FontSize(14), Foreground = design.Brush("text-faint") });
        if (model.Snapshot?.Running == true && !model.ShowingHistory && (model.Snapshot.Stopping || !processGroups.Any(group => group.Live)))
        {
            var working = design.Text(model.Snapshot.Stopping ? design.L("正在停止…", "Stopping…") : design.L("正在处理…", "Working…"), 12);
            working.Foreground = design.Brush("text-muted"); desired.Add(working);
        }
        foreach (var key in renderedRows.Keys.Where(key => !retained.Contains(key)).ToArray()) renderedRows.Remove(key);
        foreach (var key in compactionCards.Keys.Where(key => !retained.Contains(key)).ToArray()) compactionCards.Remove(key);
        foreach (var key in branchDisclosures.Keys.Where(key => !retained.Contains(key)).ToArray()) branchDisclosures.Remove(key);
        foreach (var id in runClocks.Keys.Where(id => snapshot?.RunCards?.Any(run => run.Id == id) != true).ToArray()) runClocks.Remove(id);
        // Keep unchanged controls attached: rebuilding their parents would still
        // discard text selection, keyboard focus and nested disclosure state.
        var wanted = desired.ToHashSet();
        var anchorRevision = model.ScrollRevision;
        var anchor = !followLatest && NativeTranscriptRows.ReadingAnchorIsCurrent(anchorRevision, model.ScrollRevision, appliedReadingRevision)
            ? transcript.Children.OfType<FrameworkElement>()
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
                if (disposed || followLatest || userScrollPending || session != readingSession || !transcript.Children.Contains(anchor.Element)
                    || !NativeTranscriptRows.ReadingAnchorIsCurrent(anchorRevision, model.ScrollRevision, appliedReadingRevision)) return;
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
        var fingerprint = JsonSerializer.Serialize(model.Snapshot?.Approvals) + JsonSerializer.Serialize(model.Snapshot?.Acp)
            + $"/{model.ShowingHistory}/{model.Busy}/{model.ConnectionError}/{model.Snapshot?.ReadOnly}/{design.Typography}";
        if (fingerprint == approvalFingerprint) return;
        approvalFingerprint = fingerprint;
        approvals.Children.Clear();
        if (model.ShowingHistory) return;
        RenderAcpPermissions();
        foreach (var approval in model.Snapshot?.Approvals ?? [])
        {
            var captured = approval;
            var card = new StackPanel { Spacing = 10, Padding = new Thickness(16) };
            card.Children.Add(new TextBlock { Text = "需要确认 · " + approval.Tool, FontSize = 13 });
            card.Children.Add(new TextBlock { Text = approval.Message, TextWrapping = TextWrapping.Wrap });
            if (approval.Preview.Length > 0)
                card.Children.Add(new TextBox { AcceptsReturn = true, Text = approval.Preview, IsReadOnly = true, FontFamily = design.Font(true), FontSize = design.FontSize(12, true), MaxHeight = 130 });
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
        CloseModelSwitchConfirmation(false);
        CloseFullPermissionConfirmation(false);
        CloseCompactionDialog();
        CloseIdleCompactionDialog();
        CloseContextRecoveryDialog();
        CloseHandoffDialog();
        model.Handoff.Changed -= RefreshHandoffDialog;
        acpDraftNotice.Hide();
        composerTextMenu.Hide();
        attachmentDialog?.Hide();
        historyDialog?.Hide();
        queueMenu?.Hide();
        composerOptions.Hide();
        effortMenu.Hide();
        CloseRuntimeInterpreter(false); CloseRuntimeEnvironment(false); CloseRuntimeDialog(false);
        composeMenu.Hide();
        CloseSelectionActions(); CloseContextUsage(); contextUsageSurface?.Dispose();
        modelMenu.Hide();
        optionSubmenu.Hide();
        disposed = true; lifetime.Cancel(); model.Changed -= Refresh; model.Effort.Changed -= Refresh; model.Options.Changed -= Refresh;
        model.Runtimes.Changed -= RefreshRuntimeStrip;
        model.ContextDetails.Changed -= RefreshContextDetails;
        model.ModelContext.Changed -= ModelContextChanged;
        model.Runtimes.Inspector.Changed -= RefreshRuntimeEnvironment;
        if (readingTargetLayout != null) transcript.LayoutUpdated -= readingTargetLayout;
        model.ReferencePicker.Changed -= RenderComposerChoices; design.TypographyChanged -= Refresh; design.Welcome.Changed -= Refresh; model.Pause(); lifetime.Dispose();
    }
}
