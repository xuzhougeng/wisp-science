using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private bool lastModelView;
    private FrameworkElement? modelContextStatus;
    private string? modelContextStatusKey;
    public async void SelectModelView(bool selected)
    {
        if (disposed) return;
        if (selected && model.ShowingHistory) model.Latest();
        model.ModelContext.Bind(model.Snapshot, model.ContextReady);
        CancelExcerptSelection();
        await model.ModelContext.SelectAsync(selected, lifetime.Token);
    }
    private void ModelContextChanged()
    {
        if (disposed) return;
        if (lastModelView != model.ModelContext.Active)
        {
            lastModelView = model.ModelContext.Active; followLatest = false; userScrollPending = true;
            scroll.ChangeView(null, 0, null, true);
        }
        Refresh();
    }
    private FrameworkElement? ContextSpecialRow(ConversationItem item)
    {
        if (item.Role == "checkpoint")
        {
            var block = new StackPanel { Spacing = 4 };
            var title = design.Text(design.T("chat.compaction_checkpoint"), 11); title.Foreground = design.Brush("text-faint"); block.Children.Add(title);
            block.Children.Add(TranscriptView.RenderMarkdown(item.Text, design));
            AutomationProperties.SetAutomationId(block, "context-checkpoint-row"); return block;
        }
        var tombstone = NativeTranscriptRows.IsContextTombstone(item);
        if (item.Role != "system" && !tombstone) return null;
        var label = tombstone && item.Role == "tool" && !string.IsNullOrEmpty(item.ToolName) && item.ToolName != "attempt_completion"
            ? design.Localization.Format("chat.context_tombstone_named", ("name", item.ToolName))
            : design.T(item.Role == "system" ? "chat.context_system" : "chat.context_tombstone");
        var header = new Grid { ColumnSpacing = 6 };
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); header.ColumnDefinitions.Add(new());
        var icon = design.Icon(tombstone ? "archive" : "chevron-right", 12, design.ColorText("text-faint")); header.Children.Add(icon);
        var titleText = design.Text(label, 12); titleText.Foreground = design.Brush("text-faint"); Grid.SetColumn(titleText, 1); header.Children.Add(titleText);
        if (tombstone && NativeTranscriptRows.ContextArchiveReference(item.Text) is { } archive)
        {
            var reference = design.Text(archive, 11); reference.FontFamily = design.Font(true); reference.Foreground = design.Brush("text-faint");
            reference.TextWrapping = TextWrapping.NoWrap; reference.TextTrimming = TextTrimming.CharacterEllipsis;
            Grid.SetColumn(reference, 2); header.Children.Add(reference);
        }
        var text = design.Text(item.Text, 12); text.FontFamily = design.Font(true); text.IsTextSelectionEnabled = true; design.BindLineHeight(text, 12, 1.45);
        var body = new Border { Child = text, Padding = new(10, 8, 10, 8), CornerRadius = new(6), Background = design.Brush("bg-elev") };
        var row = new NativeTranscriptDisclosure(design, header, body, label, false);
        if (!tombstone) row.ExpansionChanged += () => icon.RenderTransform = new RotateTransform { Angle = row.IsExpanded ? 90 : 0, CenterX = 6, CenterY = 6 };
        AutomationProperties.SetAutomationId(row, item.Role == "system" ? "context-system-row" : "context-tombstone-row"); return row;
    }
    private FrameworkElement ModelContextStatus()
    {
        var context = model.ModelContext;
        var key = $"{context.Busy}/{context.Error}/{context.ChangedDuringRead}/{design.Dark}/{design.LightPalette}/{design.DarkPalette}/{design.Typography}/{design.Localization.Locale}";
        if (modelContextStatus != null && modelContextStatusKey == key) return modelContextStatus;
        modelContextStatusKey = key;
        var box = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
        modelContextStatus = box;
        if (context.Busy)
        {
            box.Children.Add(design.Text(design.T("chat.context_view_loading"), 13));
            AutomationProperties.SetAutomationId(box, "context-view-loading");
        }
        else if (context.Error != null)
        {
            var text = design.Text(context.ChangedDuringRead ? design.T("chat.context_view_changed") : context.Error, 13);
            text.Foreground = design.Brush("err"); box.Children.Add(text);
            var retry = new Button { Content = design.T("transcript.retry"), HorizontalAlignment = HorizontalAlignment.Center };
            retry.Click += async (_, _) => await context.RetryAsync(lifetime.Token); box.Children.Add(retry);
            AutomationProperties.SetAutomationId(box, "context-view-error");
        }
        return box;
    }
}
