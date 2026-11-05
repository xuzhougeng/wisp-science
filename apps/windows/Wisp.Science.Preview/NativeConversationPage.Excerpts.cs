using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly StackPanel excerptArea = new() { Spacing = 8, Margin = new(16, 0, 16, 10), Visibility = Visibility.Collapsed };
    private string? excerptFingerprint;
    private readonly Func<string, string, string, Task>? openExcerpt;

    private FrameworkElement ComposerQuoteCard(NativeComposerQuote quote)
    {
        var row = new Grid { Width = 224 };
        var content = new Grid { ColumnSpacing = 9, MinHeight = 52, Padding = new(7, 7, 27, 7) };
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.ColumnDefinitions.Add(new());
        content.Children.Add(new Border { Width = 32, Height = 32, CornerRadius = new(6), Background = design.Brush("clay-soft"),
            Child = design.Icon("chat", 20, design.ColorText("clay-strong")), VerticalAlignment = VerticalAlignment.Center });
        var copy = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        var label = design.Text(quote.Label, 12); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis; copy.Children.Add(label);
        var sourceLabel = quote.SourceLabel(design.Localization);
        var source = design.Text(sourceLabel, 10.5); source.Foreground = design.Brush("text-faint");
        source.TextWrapping = TextWrapping.NoWrap; source.TextTrimming = TextTrimming.CharacterEllipsis; copy.Children.Add(source);
        Grid.SetColumn(copy, 1); content.Children.Add(copy);
        row.Children.Add(new Border { Child = content, Margin = new(0, 7, 7, 0), Background = design.Brush("bg-elev"),
            BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(8) });
        var remove = design.ToolButton(design.T("composer.remove_attachment"), "close");
        remove.Content = design.Icon("close", 12); remove.Width = remove.Height = 20; remove.MinWidth = remove.MinHeight = 0;
        remove.Padding = new(0); remove.CornerRadius = new(10); remove.Background = design.Brush("bg-elev");
        remove.BorderBrush = design.Brush("border-strong"); remove.BorderThickness = new(1);
        remove.HorizontalAlignment = HorizontalAlignment.Right; remove.VerticalAlignment = VerticalAlignment.Top;
        remove.IsEnabled = model.CanAttach || model.CanPrepareProjectDraft;
        AutomationProperties.SetName(remove, design.T("composer.remove_attachment") + ": " + sourceLabel);
        remove.Click += (_, _) => model.RemoveQuote(quote); row.Children.Add(remove);
        ToolTipService.SetToolTip(row, quote.Tooltip());
        AutomationProperties.SetAutomationId(row, "composer-quote"); return row;
    }

    private async Task SelectExcerpt(NativeHistoryTarget target)
    {
        var id = await model.SelectExcerptAsync(target, design.Localization, lifetime.Token);
        if (id != null && !disposed && openExcerpt != null
            && model.CurrentProjectId == target.ProjectId && model.CurrentSessionId == target.SessionId)
            await openExcerpt(target.ProjectId, target.SessionId, id);
    }
    private bool CancelExcerptSelection()
    {
        var start = model.ExcerptStart;
        if (!model.CancelExcerpt()) return false;
        if (start != null) RestoreHistoryFocus(start, "excerpt");
        return true;
    }
    private void RefreshExcerpt()
    {
        var start = model.ExcerptStart;
        var fingerprint = $"{start}/{model.ExcerptBusy}/{model.ExcerptUncertain}/{model.ExcerptFailure}/{model.ExcerptTruncated}/{model.Busy}/{model.ConnectionError}/{design.Localization.Locale}/{design.Typography}/{design.Dark}/{design.LightPalette}/{design.DarkPalette}";
        if (excerptFingerprint == fingerprint) return;
        excerptFingerprint = fingerprint; excerptArea.Children.Clear();
        if (start != null)
        {
            var copy = new StackPanel { Spacing = 2 };
            var eyebrow = design.Text(design.T("excerpt.banner_eyebrow"), 9.5);
            eyebrow.FontWeight = Microsoft.UI.Text.FontWeights.Bold; eyebrow.CharacterSpacing = 60;
            eyebrow.Foreground = design.Brush("clay-strong"); copy.Children.Add(eyebrow);
            var title = design.Text(design.T("excerpt.banner_title"), 13.5);
            title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; copy.Children.Add(title);
            var body = design.Text(design.Localization.Format("excerpt.banner_body", ("n", (start.Turn.UserIndex + 1).ToString())), 11);
            body.Foreground = design.Brush("text-muted"); design.BindLineHeight(body, 11, 1.4); copy.Children.Add(body);
            var layout = new Grid { ColumnSpacing = 14 };
            layout.ColumnDefinitions.Add(new()); layout.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            layout.RowDefinitions.Add(new()); layout.RowDefinitions.Add(new() { Height = GridLength.Auto }); layout.Children.Add(copy);
            var cancel = new Button { Content = design.Text(design.T("excerpt.cancel"), 10.5), VerticalAlignment = VerticalAlignment.Center };
            design.ActionButton(cancel); cancel.Padding = new(8, 5, 8, 5); cancel.MinWidth = cancel.MinHeight = 0;
            cancel.Click += (_, _) => CancelExcerptSelection(); layout.Children.Add(cancel);
            void Resize()
            {
                var compact = excerptArea.ActualWidth < 600;
                layout.ColumnDefinitions[1].Width = compact ? new(0) : GridLength.Auto;
                layout.RowSpacing = compact ? 9 : 0; Grid.SetColumn(cancel, compact ? 0 : 1); Grid.SetRow(cancel, compact ? 1 : 0);
                cancel.HorizontalAlignment = compact ? HorizontalAlignment.Left : HorizontalAlignment.Right;
            }
            layout.SizeChanged += (_, _) => Resize(); Resize();
            var banner = new Border { Child = layout, Padding = new(14, 12, 14, 12), CornerRadius = new(6),
                BorderThickness = new(1), BorderBrush = design.Brush("clay"), Background = design.Brush("clay-soft") };
            AutomationProperties.SetAutomationId(banner, "excerpt-selection"); excerptArea.Children.Add(banner);
        }
        var note = model.ExcerptFailure is { } failure ? design.Localization.Format("excerpt.failed", ("error", failure))
            : model.ExcerptBusy ? design.L("正在准备引用…", "Preparing quoted conversation…")
            : model.ExcerptTruncated ? design.T("excerpt.truncated") : null;
        if (note != null)
        {
            var text = design.Text(note, 12); text.Foreground = design.Brush(model.ExcerptFailure == null ? "text-muted" : "err");
            excerptArea.Children.Add(text);
        }
        if (model.ExcerptUncertain)
        {
            var text = design.Text(design.L("新会话的创建结果未确认。请先检查会话列表，再决定是否重新创建。",
                "Conversation creation was not confirmed. Check the session list before creating another."), 12);
            excerptArea.Children.Add(text);
            var acknowledge = new Button { Content = design.Text(design.L("已检查会话列表，允许重新创建", "I checked the session list; allow creation again"), 11),
                HorizontalAlignment = HorizontalAlignment.Left, IsEnabled = !model.ExcerptBusy && !model.Busy && model.ConnectionError == null };
            design.ActionButton(acknowledge); acknowledge.Click += (_, _) => model.AcknowledgeExcerptCreation(); excerptArea.Children.Add(acknowledge);
        }
        excerptArea.Visibility = excerptArea.Children.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
    }
}
