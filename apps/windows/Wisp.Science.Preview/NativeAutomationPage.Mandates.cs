using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeAutomationPage
{
    private readonly StackPanel mandateForm = new() { Spacing = 12 }, mandateRows = new() { Spacing = 10 };
    private readonly Grid mandateTemplates = new() { ColumnSpacing = 12, RowSpacing = 12 };
    private Border mandateFormCard = null!;
    private TextBlock mandateFormError = null!;
    private Button newMandate = null!;
    private readonly List<Control> mandateFormInputs = [], mandateRowActions = [];
    private readonly List<Grid> mandateGrids = [], mandateKpiGrids = [], mandateHeads = [];
    private readonly List<ComboBox> mandateSelectors = [];
    private readonly Dictionary<string, bool> expandedReports = new();
    private readonly Dictionary<CheckBox, bool> mandateToggles = new();
    private CalendarDatePicker? mandateDate;
    private NativeMandateDraft? renderedMandate;
    private int renderedKpis = -1;
    private string mandateProjects = "", mandateRowSignature = "";

    private void BuildMandates()
    {
        mandateFormCard = Card(mandateForm); mandateFormCard.Visibility = Visibility.Collapsed;
        AutomationProperties.SetAutomationId(mandateFormCard, "mandate-form"); page.Children.Add(mandateFormCard);
        var section = new StackPanel { Spacing = 10 }; AutomationProperties.SetAutomationId(section, "mandate-section");
        var head = new Grid { ColumnSpacing = 12 }; Columns(head, new(1, GridUnitType.Star), GridLength.Auto);
        var title = Text(L("研究职责", "Research mandates"), 15, bold: true); title.VerticalAlignment = VerticalAlignment.Center; head.Children.Add(title);
        newMandate = Tool(L("新建职责", "New mandate"), "plus", () => model.BeginMandate(), true); design.ActionButton(newMandate);
        AutomationProperties.SetAutomationId(newMandate, "mandate-create"); Put(head, newMandate, 0, 1); section.Children.Add(head);
        section.Children.Add(Text(L("交出一份职责而不是一段提示词：目标、衡量方式，以及哪些事可以不问就做。Agent 一轮轮推进，并向你汇报。", "Hand over a responsibility instead of a prompt: a goal, how it is measured, and what may be done without asking. The agent carries it round by round and reports back."), 12, "text-faint"));
        section.Children.Add(mandateRows); section.Children.Add(Text(L("从职责模板开始：", "Start from a mandate template:"), 12, "text-faint"));
        section.Children.Add(mandateTemplates); page.Children.Add(section);
        for (var i = 0; i < 4; i++)
        {
            var index = i; var draft = NativeMandateTemplates.Create(i, model.Chinese, DateTimeOffset.Now, TimeZoneInfo.Local);
            var content = new StackPanel { Spacing = 8 }; var label = new NativeActionWrap { Gap = 8 };
            label.Children.Add(design.Icon(NativeMandateTemplates.Icons[i], 17)); label.Children.Add(Text(draft.Name, 14, bold: true)); content.Children.Add(label);
            var description = Text(draft.Goal, 12, "text-muted"); description.MaxLines = 2; description.TextTrimming = TextTrimming.CharacterEllipsis; content.Children.Add(description);
            content.Children.Add(Text(L($"默认每 {draft.Hours} 小时 · 每 {draft.ReportDays} 天汇报", $"Every {draft.Hours} h · report every {draft.ReportDays} d"), 12, "text-faint"));
            var template = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top };
            design.ActionButton(template); template.Padding = new Thickness(16, 14, 16, 14); AutomationProperties.SetName(template, draft.Name);
            AutomationProperties.SetAutomationId(template, "mandate-template-" + i); template.Click += (_, _) => model.BeginMandate(index); mandateTemplates.Children.Add(template);
        }
    }
    private void RenderMandates()
    {
        newMandate.IsEnabled = model.Ready;
        foreach (Button template in mandateTemplates.Children) template.IsEnabled = model.Ready;
        var signature = JsonSerializer.Serialize(model.Snapshot?.Mandates) + "|" + model.DeletingMandate + "|" + ProjectSignature()
            + (model.Snapshot == null ? "|" + UnavailableStatus() : "");
        if (signature != mandateRowSignature) { mandateRowSignature = signature; RenderMandateRows(); }
        foreach (var control in mandateRowActions) control.IsEnabled = model.Ready && control.Tag is not false;
        foreach (var (toggle, value) in mandateToggles) toggle.IsChecked = value;
        if (!ReferenceEquals(renderedMandate, model.MandateDraft) || renderedKpis != (model.MandateDraft?.Kpis.Count ?? -1) || mandateProjects != ProjectSignature()) RenderMandateForm();
        if (mandateFormError != null) { mandateFormError.Text = model.MandateError ?? ""; mandateFormError.Visibility = model.MandateError == null ? Visibility.Collapsed : Visibility.Visible; }
        foreach (var control in mandateFormInputs) control.IsEnabled = model.Ready && control.Tag is not false;
    }
    private void RenderMandateRows()
    {
        mandateRows.Children.Clear(); mandateRowActions.Clear(); mandateHeads.Clear(); mandateToggles.Clear();
        if (model.Snapshot?.Mandates is not { } rows) { mandateRows.Children.Add(Text(UnavailableStatus(), 12, "text-faint")); return; }
        if (rows.Length == 0)
        {
            var text = Text(L("还没有职责。可以从下方模板开始，或自己定义一份。", "No mandates yet. Start from a template below or define your own."), 14, "text-muted"); text.TextAlignment = TextAlignment.Center;
            var empty = new Grid(); AutomationProperties.SetAutomationId(empty, "mandate-empty");
            empty.Children.Add(new Microsoft.UI.Xaml.Shapes.Rectangle { Stroke = design.Brush("border"), StrokeThickness = 1, StrokeDashArray = new DoubleCollection { 3, 3 }, RadiusX = 8, RadiusY = 8, IsHitTestVisible = false });
            empty.Children.Add(new Border { Child = text, Padding = new Thickness(16, 28, 16, 28) }); mandateRows.Children.Add(empty); return;
        }
        foreach (var overview in rows)
        {
            var m = overview.Mandate; var active = m.Status is "active" or "waiting"; var closed = m.Status == "done";
            var content = new StackPanel { Spacing = 6 }; var head = new Grid { ColumnSpacing = 8, RowSpacing = 6 }; Columns(head, new(1, GridUnitType.Star), GridLength.Auto); Rows(head, GridLength.Auto, GridLength.Auto);
            var title = new NativeActionWrap { Gap = 8 }; title.Children.Add(design.Icon("research-trail", 17, design.ColorText("clay"))); title.Children.Add(Text(m.Name, 14, bold: true));
            var status = m.Status switch { "waiting" => L("等你处理", "Needs you"), "paused" => L("已暂停", "Paused"), "done" => L("已结束", "Closed"), _ => L("进行中", "Active") };
            title.Children.Add(new Border { Child = Text(status, 11, m.Status == "waiting" ? "err" : m.Status == "active" ? "clay-strong" : "text-muted"),
                BorderBrush = design.Brush(m.Status == "waiting" ? "err" : "border"), BorderThickness = new Thickness(m.Status == "active" ? 0 : 1),
                Background = m.Status == "active" ? design.Brush("clay-soft") : null, CornerRadius = new CornerRadius(99), Padding = new Thickness(8, 1, 8, 1) });
            head.Children.Add(title); var actions = new NativeActionWrap { Gap = 4 };
            var toggle = Check(L("启用 ", "Enable ") + m.Name, v => model.MandateActionAsync(m.Id, "set_mandate_status", v ? "active" : "paused")); toggle.IsChecked = active; toggle.Margin = new Thickness(0, 0, 6, 0); actions.Children.Add(toggle); mandateRowActions.Add(toggle);
            mandateToggles[toggle] = active;
            void Action(string zh, string en, string icon, System.Action action, bool enabled = true)
            { var button = Tool(L(zh, en) + " " + m.Name, icon, action); button.Tag = enabled; actions.Children.Add(button); mandateRowActions.Add(button); }
            Action("立即运行一轮", "Run a round now", "play", () => _ = model.MandateActionAsync(m.Id, "run_mandate"), !closed);
            Action("立即生成汇报", "Write a report now", "clipboard", () => _ = model.MandateActionAsync(m.Id, "report_mandate"), !closed);
            Action("结束职责", "Close mandate", "circle-check", () => _ = model.MandateActionAsync(m.Id, "set_mandate_status", "done"), !closed);
            Action("编辑", "Edit", "edit", () => model.BeginMandate(id: m.Id));
            var delete = Tool((model.DeletingMandate == m.Id ? L("再次点击确认删除", "Click again to delete") : L("删除", "Delete")) + " " + m.Name, "trash", () => _ = model.DeleteMandateAsync(m.Id));
            if (model.DeletingMandate == m.Id) delete.Content = design.Icon("trash", 17, design.ColorText("err")); actions.Children.Add(delete); mandateRowActions.Add(delete);
            Put(head, actions, 0, 1); mandateHeads.Add(head); content.Children.Add(head);
            var when = m.Status == "active" ? L("下一轮 ", "Next round ") + Stamp(m.NextRunAt) : m.Status == "waiting" ? L("等待你的处理", "Waiting for you") : status;
            var until = m.EndsAt is { } end ? L(" · 截至 ", " · until ") + Day(end) : "";
            content.Children.Add(Text(model.Snapshot!.Projects.First(p => p.Id == m.ProjectId).Name + " · " + when + until, 12, "text-faint"));
            var goal = Text(m.Goal, 14, "text-muted"); goal.MaxLines = 2; goal.TextTrimming = TextTrimming.CharacterEllipsis; content.Children.Add(goal);
            foreach (var note in m.Constraints.Notes) content.Children.Add(MandateBullet(note, 12));
            if (m.Kpis.Length > 0)
            {
                var kpis = new NativeActionWrap { Gap = 8 };
                foreach (var k in m.Kpis)
                {
                    var copy = new StackPanel { Spacing = 1, MinWidth = 118 }; copy.Children.Add(Text(k.Name, 11, "text-faint")); copy.Children.Add(Text((k.Current ?? "–") + (k.Target.Length > 0 ? " / " + k.Target : ""), 14, bold: true)); copy.Children.Add(Text(k.Period, 11, "text-faint"));
                    var card = Card(copy); card.Padding = new Thickness(10, 6, 10, 6); card.CornerRadius = new CornerRadius(4); ToolTipService.SetToolTip(card, k.Definition); kpis.Children.Add(card);
                }
                content.Children.Add(kpis);
            }
            if (overview.LastRound is { } round)
            {
                var copy = new StackPanel { Spacing = 2 }; copy.Children.Add(Text(L($"第 {round.Seq} 轮", $"Round {round.Seq}") + " · " + Stamp(round.CreatedAt) + (round.Source == "host" ? L(" · 未提交回合报告", " · ended without a report") : ""), 11, "text-faint"));
                copy.Children.Add(MandateLine(L("已完成", "Done"), round.Done, 3));
                if (round.Blockers.Length > 0) copy.Children.Add(MandateLine(L("阻塞", "Blocked"), round.Blockers, 3, true));
                if (round.NextStep.Length > 0) copy.Children.Add(MandateLine(L("下一步", "Next"), round.NextStep, 3));
                var box = Card(copy); box.Padding = new Thickness(10, 8, 10, 8); box.Background = design.Brush("bg-input"); box.BorderThickness = new Thickness(2, 0, 0, 0); box.CornerRadius = new CornerRadius(0, 4, 4, 0); AutomationProperties.SetAutomationId(box, "mandate-last-round"); content.Children.Add(box);
            }
            if (overview.Request is { } request)
            {
                var copy = new StackPanel { Spacing = 3 };
                var kind = request.Kind switch { "login" => L("登录", "Sign-in"), "materials" => L("补充资料", "Materials"), "payment" => L("付款授权", "Payment"), "release" => L("发布", "Release"), _ => L("需要你判断", "Your judgement") };
                copy.Children.Add(Text(kind + L(" · 提出于 ", " · asked ") + Stamp(request.CreatedAt), 11, "err")); copy.Children.Add(MandateLine(L("需要", "Needs"), request.What));
                if (request.Why.Length > 0) copy.Children.Add(MandateLine(L("原因", "Why"), request.Why)); if (request.ThenWhat.Length > 0) copy.Children.Add(MandateLine(L("之后", "Then"), request.ThenWhat));
                var reply = Input(model.ReplyText(request.Id), L("回复 ", "Reply to ") + m.Name); reply.AcceptsReturn = true; reply.TextWrapping = TextWrapping.Wrap; reply.MinHeight = 62;
                reply.PlaceholderText = L("你做了什么、决定了什么或提供了什么。回复后立即开始下一轮。", "What you did, decided or provided. Your reply starts the next round."); AutomationProperties.SetAutomationId(reply, "mandate-reply-text");
                var send = Primary(L("回复", "Reply"), () => _ = model.ReplyMandateAsync(m.Id, request.Id)); send.VerticalAlignment = VerticalAlignment.Bottom; send.Tag = !string.IsNullOrWhiteSpace(reply.Text); AutomationProperties.SetAutomationId(send, "mandate-reply-send");
                reply.TextChanged += (_, _) => { model.SetReply(request.Id, reply.Text); send.Tag = !string.IsNullOrWhiteSpace(reply.Text); send.IsEnabled = model.Ready && (bool)send.Tag; };
                var line = new Grid { ColumnSpacing = 8, Margin = new Thickness(0, 6, 0, 0) }; Columns(line, new(1, GridUnitType.Star), GridLength.Auto); line.Children.Add(reply); Put(line, send, 0, 1); copy.Children.Add(line); mandateRowActions.Add(reply); mandateRowActions.Add(send);
                var box = Card(copy); box.Padding = new Thickness(10, 8, 10, 8); box.BorderBrush = design.Brush("err"); box.CornerRadius = new CornerRadius(4); AutomationProperties.SetAutomationId(box, "mandate-request"); content.Children.Add(box);
            }
            if (overview.Reports.Length > 0)
            {
                var reports = new StackPanel { Spacing = 8 }; foreach (var report in overview.Reports) reports.Children.Add(MandateReport(report));
                var caption = L($"汇报（{overview.Reports.Length}）· 最近 ", $"Reports ({overview.Reports.Length}) · latest ") + Day(overview.Reports[0].PeriodUntil);
                var disclosure = Tool(caption, expandedReports.GetValueOrDefault(m.Id) ? "chevron-down" : "chevron-right", () => { }, true); disclosure.HorizontalAlignment = HorizontalAlignment.Left;
                reports.Visibility = expandedReports.GetValueOrDefault(m.Id) ? Visibility.Visible : Visibility.Collapsed;
                disclosure.Click += (_, _) => { var expanded = !expandedReports.GetValueOrDefault(m.Id); expandedReports[m.Id] = expanded; reports.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed;
                    if (disclosure.Content is StackPanel panel && panel.Children.Count > 0) { panel.Children.RemoveAt(0); panel.Children.Insert(0, design.Icon(expanded ? "chevron-down" : "chevron-right", 16)); } };
                AutomationProperties.SetAutomationId(disclosure, "mandate-reports"); content.Children.Add(disclosure); content.Children.Add(reports);
            }
            var container = Card(content); container.Padding = new Thickness(14, 12, 14, 12); container.Opacity = active ? 1 : .7; AutomationProperties.SetAutomationId(container, "mandate-" + m.Id); mandateRows.Children.Add(container);
        }
    }
    private UIElement MandateReport(AssistantMandateReport report)
    {
        var content = new StackPanel { Spacing = 4 }; content.Children.Add(Text(Day(report.PeriodFrom) + " – " + Day(report.PeriodUntil) + " · " + L($"{report.Rounds} 轮", $"{report.Rounds} round(s)")
            + (report.Rounds > 0 && report.Body.Model.Length == 0 ? L(" · 由账本整理", " · from the ledger") : ""), 11, "text-faint")); content.Children.Add(Text(report.Body.Headline, 14, bold: true));
        foreach (var (label, items) in new[] { (L("已完成", "Done"), report.Body.Done), (L("进展", "Progress"), report.Body.Findings), (L("阻塞", "Blocked"), report.Body.Issues), (L("下一步", "Next"), report.Body.Next) })
        {
            if (items.Length == 0) continue; content.Children.Add(Text(label, 12, "text-muted", true));
            foreach (var item in items)
            {
                var cited = item.Refs.Where(i => i >= 0 && i < report.Body.Sources.Length).Select(i => report.Body.Sources[i].Title);
                content.Children.Add(MandateBullet(item.Text, 14, string.Join(", ", cited)));
            }
        }
        var box = Card(content); box.Padding = new Thickness(12, 10, 12, 10); box.CornerRadius = new CornerRadius(4); AutomationProperties.SetAutomationId(box, "mandate-report"); return box;
    }
    private UIElement MandateBullet(string value, double size, string? citation = null)
    {
        var line = new Grid { ColumnSpacing = 8 }; Columns(line, new(10), new(1, GridUnitType.Star));
        line.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Width = 3, Height = 3, Fill = design.Brush("text-muted"), HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top, Margin = new Thickness(0, 7, 0, 0) });
        var copy = Text(value, size, "text-muted"); if (!string.IsNullOrEmpty(citation)) copy.Inlines.Add(new Run { Text = "  " + citation, FontSize = design.FontSize(11), Foreground = design.Brush("text-faint") }); Put(line, copy, 0, 1); return line;
    }
    private TextBlock MandateLine(string label, string value, int maxLines = 0, bool error = false)
    {
        var line = Text("", 14, "text-muted"); line.MaxLines = maxLines; line.TextTrimming = TextTrimming.CharacterEllipsis;
        line.Inlines.Add(new Run { Text = label + "  ", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Foreground = design.Brush(error ? "err" : "text") }); line.Inlines.Add(new Run { Text = value }); return line;
    }
    private static string Day(long timestamp) => DateTimeOffset.FromUnixTimeSeconds(timestamp).LocalDateTime.ToString("yyyy-MM-dd");
}
