using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private FrameworkElement ReviewCard(ConversationSnapshot snapshot, int row, NativeReviewPresentation report)
    {
        TextBlock Text(string value, double size, string color = "text-muted", bool strong = false)
        {
            var text = design.Text(value, size); text.Foreground = design.Brush(color);
            if (strong) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            design.BindLineHeight(text, size, 1.5); text.IsTextSelectionEnabled = true; return text;
        }
        SolidColorBrush Tint(string color, double weight, string? background = null)
        {
            var fg = design.Brush(color).Color;
            if (background == null) return new(Windows.UI.Color.FromArgb((byte)(255 * weight), fg.R, fg.G, fg.B));
            var bg = design.Brush(background).Color;
            byte Mix(byte a, byte b) => (byte)Math.Round(a * weight + b * (1 - weight));
            return new(Windows.UI.Color.FromArgb(Mix(fg.A, bg.A), Mix(fg.R, bg.R), Mix(fg.G, bg.G), Mix(fg.B, bg.B)));
        }
        Border Pill(string label, string color, Brush? background = null, bool mono = false)
        {
            var text = Text(label, 10.5, color, !mono);
            if (mono) text.FontFamily = design.Font(true);
            // A 999-DIP radius clips wrapped model names into an ellipse in
            // WinUI. Cap the radius at the single-line pill's natural radius.
            return new() { Child = text, Padding = new(7, 2, 7, 2), CornerRadius = new(design.FontSize(10)), Background = background,
                VerticalAlignment = VerticalAlignment.Center };
        }
        Border Section(UIElement child, Thickness padding, Brush? background = null, bool line = true) => new() {
            Child = child, Padding = padding, Background = background, BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, line ? 1 : 0)
        };
        Button Action(string label)
        {
            var caption = Text(label, 11.5); caption.IsTextSelectionEnabled = false;
            var button = new Button { Content = caption, Padding = new(6, 3, 6, 3) };
            design.QuietButton(button); button.MinHeight = 26; button.Padding = new(6, 3, 6, 3);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, label); return button;
        }
        var body = new StackPanel();
        var head = new Grid { ColumnSpacing = 8 };
        head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var badges = new NativeActionWrap { Gap = 8, RowGap = 4, VerticallyCentered = true };
        badges.Children.Add(design.Icon("review", 16, design.ColorText("clay-strong")));
        badges.Children.Add(Text(design.T("review.title"), 12.5, "clay-strong", true));
        badges.Children.Add(Pill(design.Localization.Format("review.findings_n", ("n", report.Findings.Length.ToString())), "clay-strong", Tint("clay", .15)));
        if (report.Model.Length > 0) badges.Children.Add(Pill(report.Model, "text-faint", design.Brush("bg-panel"), true));
        head.Children.Add(badges);
        var copy = Action(design.T("msg.copy")); copy.VerticalAlignment = VerticalAlignment.Top;
        ToolTipService.SetToolTip(copy, design.T("ctx.copy_message"));
        copy.Click += (_, _) => { try { var data = new DataPackage(); data.SetText(report.CopyText); Clipboard.SetContent(data); } catch { } };
        Grid.SetColumn(copy, 1); head.Children.Add(copy);
        body.Children.Add(Section(head, new(14, 10, 14, 10), Tint("clay", .07)));
        body.Children.Add(Section(Text(report.Summary, 12.5), new(14, 11, 14, 11)));
        if (report.NoFindings || report.Unreviewable)
        {
            var empty = new Grid { ColumnSpacing = 6 };
            empty.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); empty.ColumnDefinitions.Add(new());
            var color = report.Unreviewable ? "warn" : "ok";
            empty.Children.Add(design.Icon(report.Unreviewable ? "circle-alert" : "check", 16, design.ColorText(color)));
            var note = Text(report.Unreviewable ? design.Localization.Format("review.unreviewable", ("pct", report.EvidenceCoverage.ToString())) : design.T("review.no_findings"), 13, color);
            Grid.SetColumn(note, 1); empty.Children.Add(note);
            body.Children.Add(Section(empty, new(14), line: false));
        }
        if (report.CoverageGaps.Length > 0)
        {
            var gaps = new StackPanel { Spacing = 6 };
            foreach (var gap in report.CoverageGaps)
            {
                var line = new Grid { ColumnSpacing = 8 };
                line.ColumnDefinitions.Add(new() { Width = new(4) }); line.ColumnDefinitions.Add(new());
                line.Children.Add(new Border { Width = 3, Height = 3, CornerRadius = new(2), Background = design.Brush("text-muted"), VerticalAlignment = VerticalAlignment.Top, Margin = new(0, 7, 0, 0) });
                var text = Text(gap, 12); Grid.SetColumn(text, 1); line.Children.Add(text); gaps.Children.Add(line);
            }
            var gapHeader = Text(design.T("review.coverage_gaps"), 12); gapHeader.IsTextSelectionEnabled = false;
            body.Children.Add(new Expander { Header = gapHeader, Content = gaps,
                Margin = new(14, 0, 14, 14), Padding = new(0), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
                BorderThickness = new(0), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        }
        for (var i = 0; i < report.Findings.Length; i++)
        {
            var findingIndex = i; var finding = report.Findings[i];
            var findingBody = new StackPanel();
            var heading = new Grid { ColumnSpacing = 6, Margin = new(0, 0, 0, 8) };
            heading.ColumnDefinitions.Add(new()); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var pills = new NativeActionWrap { Gap = 6, RowGap = 4, VerticallyCentered = true };
            var number = Text((i + 1).ToString(), 10, "text-muted", true); number.TextAlignment = TextAlignment.Center; number.VerticalAlignment = VerticalAlignment.Center;
            pills.Children.Add(new Border { Child = number, MinWidth = design.FontSize(18), MinHeight = design.FontSize(18), CornerRadius = new(999), Background = design.Brush("bg-panel") });
            var verdict = finding.Verdict == "fail" ? "err" : finding.Verdict == "warn" ? "warn" : "text-muted";
            pills.Children.Add(Pill(finding.Verdict, verdict, verdict == "text-muted" ? design.Brush("bg-panel") : Tint(verdict, verdict == "warn" ? .18 : .13)));
            pills.Children.Add(Pill(finding.Severity, finding.Severity == "high" ? "err" : "text-muted"));
            pills.Children.Add(Pill(design.T(finding.StatusKey), "ok", Tint("ok", .12)));
            heading.Children.Add(pills);
            var jump = Action(design.T("review.go_to_transcript")); jump.VerticalAlignment = VerticalAlignment.Top;
            jump.IsEnabled = NativeReviewPresentation.JumpIndex(snapshot, row, findingIndex) != null;
            if (!jump.IsEnabled) ToolTipService.SetToolTip(jump, design.L("当前加载记录无法确定原始位置。", "The original position cannot be resolved in the loaded transcript."));
            jump.Click += (_, _) => { if (!disposed) model.JumpToReviewMessage(snapshot, row, findingIndex); };
            Grid.SetColumn(jump, 1); heading.Children.Add(jump); findingBody.Children.Add(heading);
            var claim = Text(finding.Claim, 13, finding.Resolved ? "text-muted" : "text");
            design.BindLineHeight(claim, 13, 1.55); claim.Margin = new(0, 0, 0, 8);
            if (finding.Resolved) claim.TextDecorations = Windows.UI.Text.TextDecorations.Strikethrough;
            findingBody.Children.Add(claim);
            void Detail(string key, string value)
            {
                var detail = new Grid { ColumnSpacing = 8, Margin = new(0, 5, 0, 0) };
                detail.ColumnDefinitions.Add(new() { Width = new(design.FontSize(68)) }); detail.ColumnDefinitions.Add(new());
                detail.Children.Add(Text(design.T(key), 11.5, "text-faint", true));
                var text = Text(value, 11.5); Grid.SetColumn(text, 1); detail.Children.Add(text); findingBody.Children.Add(detail);
            }
            Detail("review.evidence", finding.Evidence); Detail("review.fix", finding.Fix);
            body.Children.Add(Section(findingBody, new(14, 13, 14, 13), finding.Resolved ? Tint("ok", .05) : null));
        }
        if (report.FooterKey is { } footer)
            body.Children.Add(Section(Text(design.T(footer), 11.5, footer == "review.all_fixed" ? "ok" : footer == "review.needs_attention" ? "warn" : "text-muted"), new(14, 9, 14, 9), Tint("clay", .05), false));
        return new Border { Child = body, CornerRadius = new(12), BorderThickness = new(1), BorderBrush = Tint("clay", .30, "border-strong"), Background = Tint("clay", .04, "bg-elev") };
    }
}
