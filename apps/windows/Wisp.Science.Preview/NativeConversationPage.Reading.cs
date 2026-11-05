using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private int appliedReadingRevision = -1, pendingReadingRevision = -1;
    private string? targetReadingSession;
    private EventHandler<object>? readingTargetLayout;
    private void ApplyReadingTarget()
    {
        if (model.ModelContext.Active) return;
        if (targetReadingSession != readingSession)
        {
            targetReadingSession = readingSession; appliedReadingRevision = pendingReadingRevision = -1;
            if (readingTargetLayout != null) transcript.LayoutUpdated -= readingTargetLayout;
            readingTargetLayout = null;
        }
        var revision = model.ScrollRevision;
        if (appliedReadingRevision == revision || pendingReadingRevision == revision) return;
        if (readingTargetLayout != null) transcript.LayoutUpdated -= readingTargetLayout;
        readingTargetLayout = null;
        if (model.ScrollTarget is not { } index)
        {
            appliedReadingRevision = revision;
            if (!model.ShowingHistory) { followLatest = true; userScrollPending = false; }
            return;
        }
        var page = model.ShowingHistory ? model.History : model.Snapshot;
        if (page == null || index < 0 || index >= page.Items.Length) return;
        var key = NativeTranscriptRows.Keys(page)[index];
        if (renderedRows.TryGetValue(key, out var targetRow))
        {
            foreach (var expander in Expanders(targetRow.Element)) expander.IsExpanded = true;
            foreach (var disclosure in TranscriptDisclosures(targetRow.Element)) disclosure.IsExpanded = true;
        }
        var session = readingSession;
        pendingReadingRevision = revision; followLatest = false; userScrollPending = false;
        readingTargetLayout = (_, _) =>
        {
            if (disposed || session != readingSession || revision != model.ScrollRevision)
            {
                transcript.LayoutUpdated -= readingTargetLayout; readingTargetLayout = null; pendingReadingRevision = -1; return;
            }
            // Outline selection can finish while the sheet still hides this
            // page. Wait for the actual visible layout before resolving Y.
            if (scroll.ActualHeight <= 0 || !renderedRows.TryGetValue(key, out var row)
                || row.Element is not FrameworkElement element || !element.IsLoaded || element.ActualHeight <= 0) return;
            var y = element.TransformToVisual(scroll).TransformPoint(new Windows.Foundation.Point()).Y;
            transcript.LayoutUpdated -= readingTargetLayout; readingTargetLayout = null;
            pendingReadingRevision = -1; appliedReadingRevision = revision;
            scroll.ChangeView(null, Math.Max(0, scroll.VerticalOffset + y - 8), null, true);
        };
        transcript.LayoutUpdated += readingTargetLayout;
        DispatcherQueue.TryEnqueue(() => readingTargetLayout?.Invoke(this, EventArgs.Empty));
    }
    private readonly Button contextUsage = new() { Visibility = Visibility.Collapsed };
    private readonly ContentControl contextUsageSlot = new() { Visibility = Visibility.Collapsed,
        HorizontalContentAlignment = HorizontalAlignment.Stretch, MaxWidth = PreviewLayout.ConversationMaxWidth, Margin = new(16, 0, 16, 8) };
    private NativeContextUsageSurface? contextUsageSurface;
    private bool contextUsageOpen;
    private string? contextDetailKey;
    private TextBlock? contextEpochLine;
    private readonly Dictionary<string, (Button Button, Border Detail, TextBlock Text, Button Retry)> contextDetailRows = [];
    private FlyoutBase? selectionFlyout;
    private Popup? selectionPopup;
    private string? contextFingerprint;
    private readonly StackPanel followUpQuestions = new() { Spacing = 4, Visibility = Visibility.Collapsed };
    private string? suggestionFingerprint;

    private void SetupReading()
    {
        design.QuietButton(contextUsage);
        contextUsage.MinWidth = 32; contextUsage.Height = contextUsage.MinHeight = 32;
        contextUsage.Padding = new Thickness(4, 0, 4, 0);
        contextUsage.VerticalAlignment = VerticalAlignment.Center;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(contextUsage, design.T("context_usage.open"));
        contextUsageSurface = new(this, contextUsage, contextUsageSlot, design, () => CloseContextUsage(), ResizeContextDetails);
        contextUsage.Click += async (_, _) =>
        {
            if (contextUsageOpen) { CloseContextUsage(); return; }
            contextUsageOpen = true; contextUsageSurface.Show();
            await model.ContextDetails.LoadAsync(lifetime.Token);
        };
        model.ContextDetails.Changed += RefreshContextDetails;
        Unloaded += (_, _) => CloseContextUsage();
    }

    private void RefreshReading()
    {
        if (disposed) return;
        var questions = !model.ShowingHistory && model.Snapshot is { Running: false, ReadOnly: false }
            ? model.Snapshot.FollowUps ?? [] : [];
        var nextSuggestions = model.Snapshot?.SessionId + "/" + System.Text.Json.JsonSerializer.Serialize(questions) + "/" + model.CanAttach;
        if (suggestionFingerprint != nextSuggestions)
        {
            suggestionFingerprint = nextSuggestions;
            followUpQuestions.Children.Clear();
            foreach (var question in questions.Take(8))
            {
                var choice = new Button { Content = new TextBlock { Text = question, TextWrapping = TextWrapping.Wrap },
                    HorizontalContentAlignment = HorizontalAlignment.Left, IsEnabled = model.CanAttach };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(choice, "继续提问：" + question);
                choice.Click += (_, _) => { if (model.Prefill(question, append: true)) composer.Focus(FocusState.Programmatic); };
                followUpQuestions.Children.Add(choice);
            }
            followUpQuestions.Visibility = questions.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        }
        var usage = NativeContextUsage.Latest(model.Snapshot?.Items ?? [], design.Localization);
        model.Handoff.Bind(model.Snapshot, model.CanOpenHandoff && !model.ShowingHistory);
        if (model.ShowingHistory) model.Handoff.Close();
        model.ContextDetails.Bind(model.Snapshot, !model.ShowingHistory && model.ContextReady);
        var fingerprint = model.Snapshot?.SessionId + "/" + System.Text.Json.JsonSerializer.Serialize(usage,
            new System.Text.Json.JsonSerializerOptions { IncludeFields = true }) + $"/{design.Typography}/{design.Dark}/{design.Localization.Locale}/{model.CanOpenCompaction}/{model.ContextDetails.Available}";
        contextUsage.Visibility = usage != null ? Visibility.Visible : Visibility.Collapsed;
        if (fingerprint == contextFingerprint) return;
        contextFingerprint = fingerprint;
        if (usage == null) { CloseContextUsage(); return; }
        var indicator = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        indicator.Children.Add(design.Icon("gauge", 18, design.ColorText(usage.Tone)));
        indicator.Children.Add(design.Text(usage.CompactLabel, 11));
        contextUsage.Content = indicator;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(contextUsage, usage.LocalizedLabel(design.Localization));
        contextUsage.Foreground = design.Brush(usage.Tone);
        ToolTipService.SetToolTip(contextUsage, usage.LocalizedLabel(design.Localization) + "\n" + usage.LocalizedTotal(design.Localization));
        var content = new StackPanel { Spacing = 10 };
        var head = new Grid { ColumnSpacing = 10 };
        head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = design.Text(design.T("context_usage.title"), 18); title.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        title.VerticalAlignment = VerticalAlignment.Center; head.Children.Add(title);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        var compactLabel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
        compactLabel.Children.Add(design.Icon("context-compact", 14, design.ColorText("clay-strong")));
        var compactText = design.Text(design.T("context_usage.nudge_compact"), 11.5); compactText.Foreground = design.Brush("clay-strong"); compactLabel.Children.Add(compactText);
        var compact = new Button { Content = compactLabel, IsEnabled = model.CanOpenCompaction };
        design.QuietButton(compact);
        compact.MinHeight = compact.Height = 28; compact.MinWidth = 0; compact.Padding = new Thickness(9, 0, 9, 0);
        compact.CornerRadius = new CornerRadius(14); compact.BorderThickness = new Thickness(1); compact.BorderBrush = design.Brush("clay");
        compact.Background = design.Brush("clay-soft");
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(compact, design.T("context_usage.nudge_compact"));
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(compact, "context-usage-compact-header");
        compact.Click += (_, _) => { CloseContextUsage(); model.OpenCompaction(); };
        var dock = new Button { Content = design.Icon("dock", 16), Visibility = Visibility.Collapsed }; design.QuietButton(dock);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(dock, design.T("context_usage.dock"));
        ToolTipService.SetToolTip(dock, design.T("context_usage.dock"));
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(dock, "context-usage-dock");
        var close = new Button { Content = design.Icon("close", 16) }; design.QuietButton(close);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(close, design.T("context_usage.close"));
        ToolTipService.SetToolTip(close, design.T("context_usage.close"));
        foreach (var iconButton in new[] { dock, close }) { iconButton.Width = iconButton.Height = 28; iconButton.MinWidth = iconButton.MinHeight = 0; iconButton.Padding = new(0); }
        close.Click += (_, _) => CloseContextUsage(true); actions.Children.Add(compact); actions.Children.Add(dock); actions.Children.Add(close);
        Grid.SetColumn(actions, 1); head.Children.Add(actions); content.Children.Add(head);
        var summary = new Grid { ColumnSpacing = 16, Margin = new Thickness(0, 4, 0, 0) };
        summary.ColumnDefinitions.Add(new()); summary.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var percent = design.Text(usage.Max == 0 ? design.T("context_usage.open_unknown")
            : design.Localization.Format("context_usage.full", ("pct", usage.Percent.ToString())), 13);
        percent.Foreground = design.Brush("text-muted"); summary.Children.Add(percent);
        var total = design.Text(usage.LocalizedTotal(design.Localization), 13); total.Foreground = design.Brush("text-muted");
        Grid.SetColumn(total, 1); summary.Children.Add(total); content.Children.Add(summary);
        contextEpochLine = design.Text("", 12); contextEpochLine.Foreground = design.Brush("text-muted");
        contextEpochLine.Visibility = Visibility.Collapsed; content.Children.Add(contextEpochLine);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(contextEpochLine, "context-usage-epoch");
        contextHealthLine = design.Text("", 12); content.Children.Add(contextHealthLine);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(contextHealthLine, "context-usage-health");
        content.Children.Add(new NativeTranscriptViewToggle(model.ModelContext, design, SelectModelView, labels: true));
        content.Children.Add(BuildContextNudge());
        var bar = new Grid { Height = 8, Background = design.Brush("border"), CornerRadius = new CornerRadius(4) };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(bar, percent.Text);
        // Widths are relative to the whole window, leaving unused capacity visible.
        var positive = usage.Rows.Where(row => row.Tokens > 0).ToArray();
        var occupied = 0d;
        foreach (var row in positive)
        {
            var share = Math.Min(Math.Max(0, 1 - occupied), usage.SegmentShare(row)); occupied += share;
            bar.ColumnDefinitions.Add(new() { Width = new GridLength(share, GridUnitType.Star) });
            var segment = new Border { Background = ContextCategoryBrush(row), CornerRadius = new CornerRadius(2), Margin = new Thickness(0, 0, 2, 0) };
            Grid.SetColumn(segment, bar.ColumnDefinitions.Count - 1); bar.Children.Add(segment);
        }
        bar.ColumnDefinitions.Add(new() { Width = new GridLength(Math.Max(0, 1 - occupied), GridUnitType.Star) });
        content.Children.Add(bar);
        var rows = new StackPanel { Spacing = 4, Margin = new Thickness(0, 2, 0, 0) };
        contextDetailRows.Clear();
        foreach (var row in usage.Rows)
        {
            var line = new Grid { ColumnSpacing = 12, MinHeight = 34 };
            line.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); line.ColumnDefinitions.Add(new());
            line.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var swatch = new Border { Width = 14, Height = 14, CornerRadius = new CornerRadius(3), Background = ContextCategoryBrush(row) }; line.Children.Add(swatch);
            var label = design.Text(row.Label, 13); label.TextTrimming = TextTrimming.CharacterEllipsis; label.TextWrapping = TextWrapping.NoWrap;
            label.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(label, 1); line.Children.Add(label);
            var tokens = design.Text(NativeContextUsage.FormatTokens(row.Tokens), 13); tokens.Foreground = design.Brush("text-muted");
            tokens.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(tokens, 2); line.Children.Add(tokens);
            var button = new Button { Content = line, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                Padding = new Thickness(10, 0, 10, 0), MinHeight = 36, CornerRadius = new CornerRadius(6), Background = design.Brush("bg-sunken"),
                Foreground = design.Brush("text"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1) };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, row.Label + " · " + NativeContextUsage.FormatTokens(row.Tokens));
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(button, "context-usage-row-" + row.Key);
            ToolTipService.SetToolTip(button, row.Label + " · " + row.Tokens.ToString("N0"));
            rows.Children.Add(button);
            if (!row.Expandable || !model.ContextDetails.Available)
            {
                button.IsEnabled = false; button.Resources["ButtonBackgroundDisabled"] = design.Brush("bg-sunken");
                button.Resources["ButtonForegroundDisabled"] = design.Brush("text"); button.Resources["ButtonBorderBrushDisabled"] = design.Brush("border");
                continue;
            }
            var chevron = design.Icon("chevron-down", 14); Grid.SetColumn(chevron, 3); line.Children.Add(chevron);
            var text = design.Text("", 12); text.IsTextSelectionEnabled = true; text.FontFamily = new FontFamily("Cascadia Mono, Consolas");
            design.BindLineHeight(text, 12, 1.55);
            text.Foreground = design.Brush("text-muted");
            var again = new Button { Content = design.L("重试", "Retry"), HorizontalAlignment = HorizontalAlignment.Left };
            design.QuietButton(again); again.Click += async (_, _) => await model.ContextDetails.RetryAsync(lifetime.Token);
            var body = new StackPanel { Spacing = 8 }; body.Children.Add(text); body.Children.Add(again);
            var detail = new Border { Child = new ScrollViewer { Content = body, MaxHeight = 240, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
                HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto },
                Visibility = Visibility.Collapsed, Padding = new Thickness(12, 10, 12, 10), Margin = new Thickness(38, 0, 10, 6), CornerRadius = new CornerRadius(8),
                BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), Background = design.Brush("bg-sunken") };
            rows.Children.Add(detail); contextDetailRows[row.Key] = (button, detail, text, again);
            button.Click += async (_, _) => { contextDetailKey = contextDetailKey == row.Key ? null : row.Key; RefreshContextDetails(); await model.ContextDetails.LoadAsync(lifetime.Token); };
        }
        contextUsageSurface?.SetContent(content, rows, head, dock);
        RefreshContextDetails();
        if (contextUsageOpen) _ = model.ContextDetails.LoadAsync(lifetime.Token);
    }

    private void CloseContextUsage(bool focus = false)
    {
        contextUsageSurface?.Hide();
        if (!contextUsageOpen) return;
        contextUsageOpen = false; contextDetailKey = null; model.ContextDetails.Invalidate(); RefreshContextDetails();
        if (focus && contextUsage.IsLoaded) contextUsage.Focus(FocusState.Programmatic);
    }
    private void ResizeContextDetails(bool floating, double height)
    {
        foreach (var row in contextDetailRows.Values)
            if (row.Detail.Child is ScrollViewer scroll)
            {
                scroll.MaxHeight = floating ? double.PositiveInfinity : 240;
                scroll.Height = floating ? Math.Max(80, height * .45) : double.NaN;
            }
    }
    private bool HandleContextUsageEscape()
    {
        if (!contextUsageOpen) return false;
        CloseContextUsage(true); return true;
    }

    private static Brush ContextCategoryBrush(NativeContextUsageRow row) => new SolidColorBrush(Windows.UI.Color.FromArgb(255,
        Convert.ToByte(row.Color.Substring(1, 2), 16), Convert.ToByte(row.Color.Substring(3, 2), 16), Convert.ToByte(row.Color.Substring(5, 2), 16)));

    private void RefreshContextDetails()
    {
        RefreshContextHealth();
        if (contextEpochLine != null)
        {
            var context = model.ContextDetails.Context;
            contextEpochLine.Visibility = context?.State.HeadEpoch > 0 ? Visibility.Visible : Visibility.Collapsed;
            if (context != null) contextEpochLine.Text = design.Localization.Format(
                context.State.ContextEpochs.Any(row => row.Epoch == context.State.HeadEpoch && row.CheckpointSeq != null)
                    ? "context_usage.epoch_line" : "context_usage.epoch_line_no_checkpoint",
                ("epoch", context.State.HeadEpoch.ToString()), ("turns", context.Items.Count(row => row.Role == "user").ToString()));
        }
        foreach (var (key, row) in contextDetailRows)
        {
            var expanded = contextDetailKey == key;
            row.Detail.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed;
            row.Button.BorderBrush = design.Brush(expanded ? "clay" : "border");
            if (row.Button.Content is Grid line && line.Children.LastOrDefault() is FrameworkElement chevron)
                chevron.RenderTransform = new RotateTransform { Angle = expanded ? 180 : 0, CenterX = 7, CenterY = 7 };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetHelpText(row.Button,
                design.L(expanded ? "已展开" : "已折叠", expanded ? "Expanded" : "Collapsed"));
            row.Text.Text = model.ContextDetails.Error != null ? design.L("无法加载上下文详情。", "Could not load context details.") + "\n" + model.ContextDetails.Error
                : model.ContextDetails.Loaded ? model.ContextDetails.Sections.GetValueOrDefault(key, "") : design.T("context_usage.loading");
            row.Retry.Visibility = model.ContextDetails.Error != null ? Visibility.Visible : Visibility.Collapsed;
            row.Retry.IsEnabled = !model.ContextDetails.Busy;
        }
    }

    private void AddQuote(ConversationSnapshot source, int index, string text)
    {
        CloseSelectionActions();
        if (model.AddQuote(source, index, text)) composer.Focus(FocusState.Programmatic);
    }

    private void AttachSelectionActions(FrameworkElement element, ConversationSnapshot source, int index)
    {
        var attached = new HashSet<DependencyObject>();
        element.Loaded += (_, _) => Visit(element);
        void Visit(DependencyObject node)
        {
            if (node is RichTextBlock rich && attached.Add(rich))
            {
                rich.ContextFlyout = Menu(() => rich.SelectedText, true).Context;
                rich.SelectionFlyout = null;
                Automatic(rich, () => rich.SelectedText, model.InputPreferences.SelectionPopupEnabled ? Menu(() => rich.SelectedText, false) : null);
            }
            else if (node is TextBlock text && text.IsTextSelectionEnabled && attached.Add(text))
            {
                text.ContextFlyout = Menu(() => text.SelectedText, true).Context;
                text.SelectionFlyout = null;
                Automatic(text, () => text.SelectedText, model.InputPreferences.SelectionPopupEnabled ? Menu(() => text.SelectedText, false) : null);
            }
            for (var n = 0; n < VisualTreeHelper.GetChildrenCount(node); n++) Visit(VisualTreeHelper.GetChild(node, n));
        }
        void Automatic(FrameworkElement target, Func<string> selected, TranscriptSelectionSurface? surface)
        {
            if (surface?.Popup is not { } popup) return;
            var content = (FrameworkElement)popup.Child;
            var anchor = new Windows.Foundation.Point();
            void Position()
            {
                var root = (FrameworkElement)target.XamlRoot.Content;
                var width = content.ActualWidth > 0 ? content.ActualWidth : content.DesiredSize.Width;
                var height = content.ActualHeight > 0 ? content.ActualHeight : content.DesiredSize.Height;
                popup.HorizontalOffset = Math.Clamp(anchor.X - width / 2, 12, Math.Max(12, root.ActualWidth - width - 12));
                popup.VerticalOffset = Math.Clamp(anchor.Y - height - 20 >= 12 ? anchor.Y - height - 20 : anchor.Y + 20,
                    12, Math.Max(12, root.ActualHeight - height - 12));
            }
            content.SizeChanged += (_, _) => { if (popup.IsOpen && target.IsLoaded) Position(); };
            target.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler((_, e) =>
            {
                var point = e.GetCurrentPoint(target);
                if (point.Properties.PointerUpdateKind != Microsoft.UI.Input.PointerUpdateKind.LeftButtonReleased) return;
                anchor = e.GetCurrentPoint((UIElement)target.XamlRoot.Content).Position;
                // Text controls handle the pointer event themselves. Wait until
                // their final selection is available, without relying on focus.
                DispatcherQueue.TryEnqueue(() =>
                {
                    if (disposed || !target.IsLoaded || !model.InputPreferences.SelectionPopupEnabled
                        || string.IsNullOrWhiteSpace(selected())) return;
                    surface.Prepare();
                    popup.XamlRoot = target.XamlRoot;
                    var root = (FrameworkElement)target.XamlRoot.Content;
                    content.MaxWidth = Math.Max(160, Math.Min(570, root.ActualWidth - 24));
                    content.Measure(new Windows.Foundation.Size(content.MaxWidth, double.PositiveInfinity));
                    Position();
                    popup.IsOpen = true;
                });
            }), true);
        }
        TranscriptSelectionSurface Menu(Func<string> selected, bool context)
        {
            var row = new NativeActionWrap { Gap = 2, RowGap = 2 };
            var menu = context ? new MenuFlyout() : null;
            var popup = context ? null : new Popup { Child = new Border { Child = row, Padding = new(4), CornerRadius = new(10),
                Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1) } };
            void Hide() { menu?.Hide(); if (popup != null) popup.IsOpen = false; }
            var labels = NativeTranscriptLabels.Actions(design.Localization);
            var selection = "";
            bool Current() => !disposed && element.IsLoaded && source.ProjectId == model.CurrentProjectId
                && source.SessionId == model.Snapshot?.SessionId && !string.IsNullOrWhiteSpace(selection);
            Control Item(string label, string icon, Action action)
            {
                if (menu != null)
                {
                    var item = new MenuFlyoutItem { Text = label, Icon = new ImageIcon { Source = design.Icon(icon, 16).Source } };
                    item.Click += (_, _) => { if (Current()) { Hide(); action(); } };
                    menu.Items.Add(item); return item;
                }
                var button = design.ToolButton(label, icon, true);
                button.Click += (_, _) => { if (Current()) { Hide(); action(); } };
                row.Children.Add(button); return button;
            }
            // WebView exposes Copy in the context menu and Highlight in the
            // automatic selection popup. The preference gates only the latter.
            var first = context ? Item(labels.Copy, "copy", () => { try { var data = new DataPackage(); data.SetText(selection); Clipboard.SetContent(data); } catch { } })
                : Item(labels.Highlight, "star", async () => await model.SaveSelectionAsync(selection, lifetime.Token));
            var add = Item(labels.AddToChat, "plus", () => AddQuote(source, index, selection));
            var side = Item(labels.SideChat, "chat", () => { if (NativeComposerQuote.From(source, index, selection) is { } value) quote(value.Message); });
            void Prepare()
            {
                selection = selected();
                if (selectionFlyout != menu) selectionFlyout?.Hide();
                if (selectionPopup != popup && selectionPopup != null) selectionPopup.IsOpen = false;
                selectionFlyout = menu;
                selectionPopup = popup;
                first.IsEnabled = side.IsEnabled = Current();
                add.IsEnabled = Current() && model.CanQuote(source, index, selection);
            }
            if (menu != null)
            {
                menu.Opening += (_, _) => Prepare();
                menu.Closed += (_, _) => { if (selectionFlyout == menu) selectionFlyout = null; };
            }
            if (popup != null)
            {
                UIElement? inputRoot = null; XamlRoot? xamlRoot = null;
                // Outside clicks must continue to the text underneath, so a
                // right-click replaces the bar with its context menu in one step.
                PointerEventHandler outside = (_, _) => popup.IsOpen = false;
                Windows.Foundation.TypedEventHandler<XamlRoot, XamlRootChangedEventArgs> resize = (_, _) => popup.IsOpen = false;
                popup.Opened += (_, _) => { inputRoot = (UIElement)element.XamlRoot.Content; xamlRoot = element.XamlRoot;
                    inputRoot.AddHandler(UIElement.PointerPressedEvent, outside, true); xamlRoot.Changed += resize; };
                popup.Closed += (_, _) => { inputRoot?.RemoveHandler(UIElement.PointerPressedEvent, outside); if (xamlRoot != null) xamlRoot.Changed -= resize;
                    inputRoot = null; xamlRoot = null; if (selectionPopup == popup) selectionPopup = null; };
            }
            element.Unloaded += (_, _) => Hide();
            return new(menu, popup, Prepare);
        }
    }

    private sealed record TranscriptSelectionSurface(MenuFlyout? Context, Popup? Popup, Action Prepare);
    private void CloseSelectionActions() { selectionFlyout?.Hide(); if (selectionPopup != null) selectionPopup.IsOpen = false; }

    public bool HandleReadingEscape()
    {
        if (selectionFlyout is { } selection) { selection.Hide(); return true; }
        if (selectionPopup is { IsOpen: true } popup) { popup.IsOpen = false; return true; }
        return false;
    }
}
