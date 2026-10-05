using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
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
    private readonly Flyout contextUsageFlyout = new();
    private bool contextUsageOpen;
    private FlyoutBase? selectionFlyout;
    private string? contextFingerprint;
    private readonly StackPanel followUpQuestions = new() { Spacing = 4, Visibility = Visibility.Collapsed };
    private string? suggestionFingerprint;

    private void SetupReading()
    {
        contextUsage.Flyout = contextUsageFlyout;
        design.QuietButton(contextUsage);
        contextUsage.MinWidth = 32; contextUsage.Height = contextUsage.MinHeight = 32;
        contextUsage.Padding = new Thickness(4, 0, 4, 0);
        contextUsage.VerticalAlignment = VerticalAlignment.Center;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(contextUsage, "查看上下文用量");
        contextUsageFlyout.Opened += (_, _) => contextUsageOpen = true;
        contextUsageFlyout.Closed += (_, _) => contextUsageOpen = false;
    }

    private void RefreshReading()
    {
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
        var usage = NativeContextUsage.Latest(model.Snapshot?.Items ?? []);
        var fingerprint = model.Snapshot?.SessionId + "/" + System.Text.Json.JsonSerializer.Serialize(usage,
            new System.Text.Json.JsonSerializerOptions { IncludeFields = true }) + "/" + design.Typography;
        contextUsage.Visibility = usage != null ? Visibility.Visible : Visibility.Collapsed;
        if (fingerprint == contextFingerprint) return;
        contextFingerprint = fingerprint;
        if (usage == null) { contextUsageFlyout.Hide(); return; }
        var indicator = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        indicator.Children.Add(design.Icon("gauge", 14));
        indicator.Children.Add(design.Text(usage.CompactLabel, 11));
        contextUsage.Content = indicator;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(contextUsage, "查看" + usage.Label);
        contextUsage.Foreground = design.Brush(usage.Tone);
        ToolTipService.SetToolTip(contextUsage, usage.Label + "\n" + usage.Total);
        var content = new StackPanel { Spacing = 10, MinWidth = 240, MaxWidth = 400 };
        content.Children.Add(design.Text("上下文用量", 18));
        content.Children.Add(design.Text(usage.Total, 13));
        foreach (var (label, tokens) in usage.Rows) content.Children.Add(design.Text($"{label} · {tokens:N0}", 13));
        var close = new Button { Content = "关闭" };
        close.Click += (_, _) => contextUsageFlyout.Hide(); content.Children.Add(close);
        contextUsageFlyout.Content = content;
    }

    private void AddQuote(ConversationSnapshot source, int index, string text)
    {
        selectionFlyout?.Hide();
        if (model.AddQuote(source, index, text)) composer.Focus(FocusState.Programmatic);
    }

    private void AttachSelectionActions(FrameworkElement element, ConversationSnapshot source, int index)
    {
        if (!model.InputPreferences.SelectionPopupEnabled) return;
        element.Loaded += (_, _) => Visit(element);
        void Visit(DependencyObject node)
        {
            if (node is RichTextBlock rich) rich.SelectionFlyout = Menu(() => rich.SelectedText);
            else if (node is TextBlock text && text.IsTextSelectionEnabled) text.SelectionFlyout = Menu(() => text.SelectedText);
            for (var n = 0; n < VisualTreeHelper.GetChildrenCount(node); n++) Visit(VisualTreeHelper.GetChild(node, n));
        }
        MenuFlyout Menu(Func<string> selected)
        {
            var menu = new MenuFlyout();
            var copy = new MenuFlyoutItem { Text = "复制" };
            copy.Click += (_, _) => { try { var data = new DataPackage(); data.SetText(selected()); Clipboard.SetContent(data); } catch { } };
            var add = new MenuFlyoutItem { Text = "加入当前对话" };
            add.Click += (_, _) => AddQuote(source, index, selected());
            var side = new MenuFlyoutItem { Text = "询问辅助聊天" };
            side.Click += (_, _) => quote($"{NativeComposerQuote.From(source, index, selected())?.Source}\n\n{selected()}");
            var save = new MenuFlyoutItem { Text = "收藏选中内容" };
            save.Click += async (_, _) => await model.SaveSelectionAsync(selected(), lifetime.Token);
            menu.Items.Add(copy); menu.Items.Add(add); menu.Items.Add(side); menu.Items.Add(save);
            menu.Opening += (_, _) =>
            {
                selectionFlyout = menu;
                add.IsEnabled = !model.Busy && !model.UncertainSend && model.Snapshot is { ReadOnly: false }
                    && model.Snapshot.SessionId == source.SessionId;
            };
            menu.Closed += (_, _) => { if (selectionFlyout == menu) selectionFlyout = null; };
            return menu;
        }
    }

    public bool HandleReadingEscape()
    {
        if (selectionFlyout is { } selection) { selection.Hide(); return true; }
        if (contextUsageOpen) { contextUsageFlyout.Hide(); return true; }
        return false;
    }
}
