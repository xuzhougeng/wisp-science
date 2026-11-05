using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeCompactionCard : StackPanel
{
    private readonly ToggleButton toggle;
    private readonly StackPanel details;
    private readonly Image chevron;
    private readonly WispDesign design;
    public bool IsExpanded { get; private set; }
    public event Action<bool>? ExpansionChanged;
    public NativeCompactionCard(NativeCompactionRecord row, WispDesign design, bool expanded, bool canUndo, Action undo,
        bool showRewind, bool canRewind, Action rewind, string? error, Action reload)
    {
        this.design = design;
        Spacing = 10; Padding = new(0, 10, 0, 10); Opacity = row.Undone ? .55 : 1;
        AutomationProperties.SetAutomationId(this, "context-compaction-flag");
        var header = new Grid { ColumnSpacing = 12 };
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); header.ColumnDefinitions.Add(new());
        var mark = new Border { Width = 32, Height = 32, CornerRadius = new(10), BorderThickness = new(1), BorderBrush = design.Brush("border"),
            Background = design.Brush("clay-soft"), Child = design.Icon("check", 18, design.ColorText("clay-strong")), VerticalAlignment = VerticalAlignment.Top };
        header.Children.Add(mark);
        var copy = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        var title = design.Text(design.T(row.TitleKey), 12); title.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        title.Foreground = design.Brush("text-muted");
        if (row.Undone) title.TextDecorations = Windows.UI.Text.TextDecorations.Strikethrough;
        copy.Children.Add(title);
        var counts = new NativeActionWrap { Gap = 10, RowGap = 4 };
        counts.Children.Add(Note(row.Counts, 11));
        if (row.Reduction is { } percent) { var reduction = Note(design.Localization.Format("chat.compaction_reduction", ("percent", percent)), 11); reduction.Foreground = design.Brush("clay-strong"); counts.Children.Add(reduction); }
        copy.Children.Add(counts); Grid.SetColumn(copy, 1); header.Children.Add(copy);
        chevron = design.Icon("chevron-down", 16, design.ColorText("text-faint")); Grid.SetColumn(chevron, 2); header.Children.Add(chevron);
        var rule = new Border { Height = 1, MinWidth = 16, Margin = new(4, 0, 0, 0), Background = design.Brush("border"), VerticalAlignment = VerticalAlignment.Center };
        Grid.SetColumn(rule, 3); header.Children.Add(rule);
        toggle = new ToggleButton { Content = header, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Padding = new(0), BorderThickness = new(0), MinHeight = 0, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        foreach (var suffix in new[] { "", "Checked", "PointerOver", "CheckedPointerOver", "Pressed", "CheckedPressed" }) toggle.Resources["ToggleButtonBackground" + suffix] = toggle.Background;
        AutomationProperties.SetAutomationId(toggle, "context-compaction-expand"); AutomationProperties.SetName(toggle, design.T(row.TitleKey));
        Children.Add(toggle);
        details = new StackPanel { Spacing = 10, Padding = new(44, 0, 0, 12) }; Children.Add(details);
        AutomationProperties.SetAutomationId(details, "context-compaction-details");
        if (!string.IsNullOrWhiteSpace(row.Checkpoint))
        {
            var checkpoint = new StackPanel { Spacing = 4 };
            var label = Note(design.T("chat.compaction_checkpoint"), 11); label.Foreground = design.Brush("text-faint"); checkpoint.Children.Add(label);
            checkpoint.Children.Add(TranscriptView.RenderMarkdown(row.Checkpoint, design)); details.Children.Add(checkpoint);
        }
        var meta = new NativeActionWrap { Gap = 14, RowGap = 8 };
        meta.Children.Add(Note(design.Localization.Format("chat.compaction_strategy", ("strategy", design.T(row.Strategy == "auto" ? "chat.compaction_strategy_auto" : "chat.compaction_strategy_manual"))), 11));
        if (row.Epoch is { } epoch) meta.Children.Add(Note(design.Localization.Format("chat.compaction_epoch", ("epoch", epoch.ToString())), 11));
        if (row.KeptFromUserIndex is { } kept) meta.Children.Add(Note(design.Localization.Format("chat.compaction_kept_from", ("turn", ((long)kept + 1).ToString())), 11));
        details.Children.Add(meta);
        var actions = new NativeActionWrap { Gap = 8, RowGap = 8 };
        var undoButton = ActionButton("chat.compaction_undo", "undo-compact", undo); undoButton.IsEnabled = canUndo; undoButton.Opacity = canUndo ? 1 : .4;
        if (row.UndoReasonKey is { } reason) { ToolTipService.SetToolTip(undoButton, design.T(reason)); AutomationProperties.SetHelpText(undoButton, design.T(reason)); }
        AutomationProperties.SetAutomationId(undoButton, "undo-compaction"); actions.Children.Add(undoButton);
        if (showRewind)
        {
            var rewindButton = ActionButton("chat.compaction_rewind", "arrow-left", rewind); rewindButton.IsEnabled = canRewind; rewindButton.Opacity = canRewind ? 1 : .4;
            AutomationProperties.SetAutomationId(rewindButton, "rewind-before-compact"); actions.Children.Add(rewindButton);
        }
        details.Children.Add(actions);
        if (error != null)
        {
            var failed = Note(design.L("撤销结果尚未确认，请重新读取会话后再试。", "Undo is unconfirmed. Reload the conversation before trying again.") + "\n" + error, 12);
            failed.Foreground = design.Brush("err"); details.Children.Add(failed);
            var retry = new Button { Content = design.L("重新读取", "Reload"), HorizontalAlignment = HorizontalAlignment.Left };
            design.QuietButton(retry); retry.Click += (_, _) => reload(); details.Children.Add(retry);
            AutomationProperties.SetAutomationId(retry, "compaction-undo-reload");
        }
        toggle.Click += (_, _) => SetExpanded(toggle.IsChecked == true);
        SizeChanged += (_, e) =>
        {
            var narrow = e.NewSize.Width <= 620;
            header.ColumnSpacing = narrow ? 8 : 12; rule.Visibility = narrow ? Visibility.Collapsed : Visibility.Visible;
            header.ColumnDefinitions[1].Width = narrow ? new(1, GridUnitType.Star) : GridLength.Auto;
            header.ColumnDefinitions[3].Width = narrow ? new(0) : new(1, GridUnitType.Star);
            mark.Width = mark.Height = narrow ? 28 : 32; details.Padding = new(narrow ? 36 : 44, 0, 0, 12);
        };
        SetExpanded(expanded);

        TextBlock Note(string text, double size) { var label = design.Text(text, size); label.Foreground = design.Brush("text-muted"); design.BindLineHeight(label, size, 1.4); return label; }
        Button ActionButton(string key, string icon, Action click)
        {
            var content = new Grid { ColumnSpacing = 6 }; content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.ColumnDefinitions.Add(new());
            content.Children.Add(design.Icon(icon, 16)); var label = Note(design.T(key), 12); Grid.SetColumn(label, 1); content.Children.Add(label);
            var button = new Button { Content = content }; design.QuietButton(button); button.Padding = new(8, 4, 8, 4);
            AutomationProperties.SetName(button, design.T(key)); button.Click += (_, _) => click(); return button;
        }
    }
    public void SetExpanded(bool expanded)
    {
        IsExpanded = expanded; toggle.IsChecked = expanded; details.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed;
        chevron.RenderTransform = new RotateTransform { Angle = expanded ? 180 : 0, CenterX = 8, CenterY = 8 };
        var label = design.T(expanded ? "chat.compaction_collapse" : "chat.compaction_expand");
        ToolTipService.SetToolTip(toggle, label); AutomationProperties.SetHelpText(toggle, label); ExpansionChanged?.Invoke(expanded);
    }
}
