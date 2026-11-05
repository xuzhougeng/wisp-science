using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Dictionary<string, (NativeCompactionCard Card, long Order)> compactionCards = [];
    private long compactionExpandOrder;
    private NativeCompactionRecord CompactionRecord(ConversationSnapshot source, NativeCompactionRecord row) =>
        model.CompactionUndoOutcome(model.CompactionTarget(source, row))?.Confirmed == true ? row with { Undone = true, CanUndo = false, UndoReason = "undone" } : row;
    private FrameworkElement CompactionRow(string key, ConversationSnapshot source, NativeCompactionRecord raw)
    {
        var row = CompactionRecord(source, raw);
        if (row.AutomaticContinue)
        {
            var line = new Grid { ColumnSpacing = 6 };
            line.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); line.ColumnDefinitions.Add(new());
            line.Children.Add(design.Icon("sync", 14, design.ColorText("text-muted")));
            var text = design.Text(design.Localization.Format("chat.auto_continued", ("count", row.Before.ToString()), ("limit", row.After.ToString())), 12);
            text.Foreground = design.Brush("text-muted"); Grid.SetColumn(text, 1); line.Children.Add(text);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetAutomationId(line, "auto-continue-flag"); return line;
        }
        var target = model.CompactionTarget(source, row);
        var rewind = model.CompactionRewindTarget(source, row);
        var previous = compactionCards.GetValueOrDefault(key);
        var card = new NativeCompactionCard(row, design, previous.Card?.IsExpanded == true, model.CanUndoCompaction(target),
            async () => { if (target != null) await model.UndoCompactionAsync(target, lifetime.Token); }, row.KeptFromUserIndex != null,
            rewind != null && !row.Undone, async () => { if (rewind != null && model.CanHistoryAction(rewind, "rewind")) await RunHistoryAction(model.CurrentHistoryTarget(rewind), "rewind"); },
            model.CompactionUndoOutcome(target)?.Error, async () => { if (target != null) await model.ReloadCompactionUndoAsync(target, lifetime.Token); });
        compactionCards[key] = (card, previous.Order);
        card.ExpansionChanged += expanded => compactionCards[key] = (card, expanded ? ++compactionExpandOrder : 0);
        return card;
    }
    private bool HandleCompactionRecordEscape()
    {
        bool Visible(FrameworkElement card)
        {
            if (!card.IsLoaded) return false;
            for (DependencyObject? element = card; element != null; element = VisualTreeHelper.GetParent(element))
                if (element is FrameworkElement { Visibility: Visibility.Collapsed }) return false;
            return true;
        }
        var open = compactionCards.Values.Where(value => value.Card.IsExpanded && Visible(value.Card)).OrderByDescending(value => value.Order).FirstOrDefault();
        if (open.Card == null) return false;
        open.Card.SetExpanded(false); return true;
    }
}
