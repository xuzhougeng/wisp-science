using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private sealed class ActivityDisclosure
    {
        private readonly WispDesign design;
        private readonly TextBlock title, note, now, meta;
        private readonly Grid heading = new() { ColumnSpacing = 8 };
        private readonly Border indicator = new() { VerticalAlignment = VerticalAlignment.Center }, chevron = new() { VerticalAlignment = VerticalAlignment.Center };
        private readonly StackPanel copy = new() { Spacing = 3, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Left };
        private readonly ColumnDefinition copyColumn = new(), nowColumn = new();
        private readonly bool completed;
        private NativeActivitySummary? summary;
        private bool live;
        private string? iconKey, chevronKey;
        public StackPanel Body { get; } = new() { Spacing = 2 };
        public StackPanel Header { get; } = new() { Spacing = 4 };
        public StackPanel Reviews { get; } = new() { Spacing = 4 };
        public NativeTranscriptDisclosure Disclosure { get; }
        public Microsoft.UI.Xaml.Controls.Primitives.ToggleButton Button => Disclosure.Button;
        public string[] ReviewIds { get; set; } = [];
        public List<FrameworkElement> Rows { get; } = [];
        public ActivityDisclosure(WispDesign design, bool completed)
        {
            this.design = design; this.completed = completed;
            heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); heading.ColumnDefinitions.Add(copyColumn); heading.ColumnDefinitions.Add(nowColumn);
            heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            title = design.Text("", 12.5); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            note = design.Text("", 11); design.BindLineHeight(note, 11, 1.45); copy.Children.Add(title); copy.Children.Add(note);
            now = design.Text("", 12); now.FontFamily = design.Font(true); now.TextWrapping = TextWrapping.NoWrap; now.TextTrimming = TextTrimming.CharacterEllipsis; now.VerticalAlignment = VerticalAlignment.Center;
            meta = design.Text("", 11.5); meta.VerticalAlignment = VerticalAlignment.Center; meta.TextAlignment = TextAlignment.Right;
            heading.Children.Add(indicator); Grid.SetColumn(copy, 1); heading.Children.Add(copy); Grid.SetColumn(now, 2); heading.Children.Add(now);
            Grid.SetColumn(meta, 3); heading.Children.Add(meta); Grid.SetColumn(chevron, 4); heading.Children.Add(chevron);
            Disclosure = new(design, heading, Body, "", false, process: completed, steps: !completed);
            Disclosure.ExpansionChanged += UpdateHeader;
            Header.Children.Add(new Border { Child = Disclosure, BorderThickness = new(completed ? 0 : 1), CornerRadius = new(completed ? 0 : 8),
                BorderBrush = design.Brush("border"), Background = completed ? null : design.Brush("bg-sunken") });
            Header.Children.Add(Reviews);
            heading.SizeChanged += (_, e) => { copy.MaxWidth = Math.Max(70, e.NewSize.Width * .7); meta.MaxWidth = Math.Max(50, e.NewSize.Width * .3); };
        }
        public void Update(NativeActivityGroup group, NativeActivitySummary value)
        {
            summary = value; live = group.Live;
            if (!completed) Button.Padding = new(13, live ? 11 : 9, 13, live ? 11 : 9);
            Disclosure.SetAutomatic(live); UpdateHeader();
        }
        private void UpdateHeader()
        {
            if (summary == null) return;
            title.Text = summary.Title; title.Foreground = design.Brush("text-muted");
            note.Text = summary.Note ?? ""; note.Foreground = design.Brush("text-faint"); note.Visibility = summary.Note == null ? Visibility.Collapsed : Visibility.Visible;
            now.Text = summary.Now ?? ""; now.Foreground = design.Brush("text-faint");
            var showNow = summary.Now != null && !Disclosure.IsExpanded;
            now.Visibility = showNow ? Visibility.Visible : Visibility.Collapsed;
            nowColumn.Width = showNow ? new(1, GridUnitType.Star) : GridLength.Auto;
            copyColumn.Width = showNow ? GridLength.Auto : new(1, GridUnitType.Star);
            meta.Text = summary.Duration ?? ""; meta.Foreground = design.Brush("text-faint"); meta.Visibility = summary.Duration == null ? Visibility.Collapsed : Visibility.Visible;
            var kind = live ? "activity-orbit" : summary.SettledOk ? "circle-check" : "circle-minus";
            var color = design.ColorText(live || summary.SettledOk ? "clay-strong" : "text-muted");
            if (iconKey != kind + color) { iconKey = kind + color; indicator.Child = live ? NativeLoadingVisuals.Activity(design, 22) : design.Icon(kind, 22, color); }
            var nextChevron = (Disclosure.IsExpanded ? "chevron-down" : "chevron-right") + design.ColorText("text-faint");
            if (chevronKey != nextChevron) { chevronKey = nextChevron; chevron.Child = design.Icon(Disclosure.IsExpanded ? "chevron-down" : "chevron-right", 12, design.ColorText("text-faint")); }
            AutomationProperties.SetName(Button, string.Join(" · ", new[] { summary.Title, summary.Note, summary.Now, summary.Duration }.Where(text => !string.IsNullOrEmpty(text))));
        }
        public void ReleaseExcept(IReadOnlyCollection<FrameworkElement> retained)
        {
            foreach (var child in Body.Children.OfType<FrameworkElement>().Where(child => !retained.Contains(child)).ToArray()) Body.Children.Remove(child);
        }
        public void MountRows()
        {
            ReleaseExcept(Rows);
            for (var i = 0; i < Rows.Count; i++)
            {
                var row = Rows[i];
                if (i < Body.Children.Count && ReferenceEquals(Body.Children[i], row)) continue;
                if (row.Parent is Panel parent) parent.Children.Remove(row);
                Body.Children.Insert(i, row);
            }
        }
    }

    private NativeTranscriptDisclosure ToolStep(ConversationItem item, StackPanel body, bool live)
    {
        var label = NativeToolPresentation.StepLabel(item);
        var failed = NativeToolPresentation.IsFailure(item);
        var running = NativeToolPresentation.Running(item);
        var complete = NativeToolPresentation.Completed(item);
        var icon = failed ? "close" : running ? "activity-orbit" : complete ? "check" : "circle-minus";
        var color = failed ? "err" : running ? "clay" : complete ? "ok" : "text-muted";
        var header = new Grid { ColumnSpacing = 8 };
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        header.Children.Add(design.Icon(icon, 14, design.ColorText(color)));
        var labels = new NativeActionWrap { Gap = 6, RowGap = 2, VerticallyCentered = true };
        if (label.BadgeKey is { } badge)
        {
            var badgeText = design.Text(design.T(badge), 10); badgeText.Foreground = design.Brush("text-muted");
            labels.Children.Add(new Border { Child = badgeText, Background = design.Brush("bg-panel"), CornerRadius = new(4), Padding = new(4, 1, 4, 1) });
        }
        var title = design.Text(label.Title, 12.5); title.Foreground = design.Brush("clay"); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis;
        labels.Children.Add(title);
        if (label.Detail.Length > 0)
        {
            var detail = design.Text(label.Detail, 12); detail.FontFamily = design.Font(true); detail.Foreground = design.Brush("text-faint");
            detail.TextWrapping = TextWrapping.NoWrap; detail.TextTrimming = TextTrimming.CharacterEllipsis; labels.Children.Add(detail);
        }
        Grid.SetColumn(labels, 1); header.Children.Add(labels);
        var meta = design.Text(NativeToolPresentation.StepMeta(item, design.Localization), 11.5); meta.Foreground = design.Brush("text-faint");
        meta.TextAlignment = TextAlignment.Right; Grid.SetColumn(meta, 2); header.Children.Add(meta);
        header.SizeChanged += (_, e) => meta.MaxWidth = Math.Max(70, Math.Min(180, e.NewSize.Width * .35));
        ToolTipService.SetToolTip(header, label.Title + "\n" + NativeToolPresentation.Heading(item, design.Localization) + (label.Detail.Length > 0 ? "\n" + label.Detail : ""));
        var step = new NativeTranscriptDisclosure(design, header, body,
            label.Title + (label.Detail.Length > 0 ? " · " + label.Detail : "") + " · " + NativeToolPresentation.Heading(item, design.Localization),
            body.Children.Count > 0 && (NativeToolPresentation.InitiallyExpanded(item, 0) || live && running), hasBody: body.Children.Count > 0);
        return step;
    }

    private NativeTranscriptDisclosure ProseStep(ConversationItem item, int index, ConversationSnapshot snapshot, bool modelView = false)
    {
        var thinking = item.Role == "reasoning";
        var heading = new Grid { ColumnSpacing = 8 };
        heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); heading.ColumnDefinitions.Add(new());
        heading.Children.Add(design.Icon(thinking ? "brain" : "minus", 14, design.ColorText("text-faint")));
        var title = design.Text(design.T(thinking ? "chat.thinking" : "chat.progress"), 11); title.Foreground = design.Brush("text-faint"); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        Grid.SetColumn(title, 1); heading.Children.Add(title);
        if (!thinking)
        {
            var text = item.Text.Split('\n').FirstOrDefault(line => !string.IsNullOrWhiteSpace(line))?.Trim() ?? "";
            var detail = design.Text(text, 12); detail.Foreground = design.Brush("text-faint"); detail.TextWrapping = TextWrapping.NoWrap; detail.TextTrimming = TextTrimming.CharacterEllipsis;
            Grid.SetColumn(detail, 2); heading.Children.Add(detail);
        }
        var body = new StackPanel();
        if (thinking)
        {
            var text = design.Text(item.Text, 13); text.Foreground = design.Brush("text-faint"); text.FontStyle = Windows.UI.Text.FontStyle.Italic;
            text.IsTextSelectionEnabled = true; design.BindLineHeight(text, 13, 1.55); body.Children.Add(text);
        }
        else
        {
            var text = TranscriptView.Create(new BrowserMessage(index, item.Role, item.Text, item.ToolName), design, commentary: true);
            if (!modelView && !model.ExampleMode) AttachSelectionActions(text, snapshot, index); body.Children.Add(text);
        }
        return new(design, heading, body, title.Text, false);
    }

    private static IEnumerable<NativeTranscriptDisclosure> TranscriptDisclosures(DependencyObject element)
    {
        if (element is NativeTranscriptDisclosure disclosure) yield return disclosure;
        IEnumerable<DependencyObject> children = element switch {
            Panel panel => panel.Children.Cast<DependencyObject>(), Border border when border.Child != null => [border.Child],
            ContentControl content when content.Content is DependencyObject child => [child], _ => [] };
        foreach (var child in children) foreach (var nested in TranscriptDisclosures(child)) yield return nested;
    }
}
