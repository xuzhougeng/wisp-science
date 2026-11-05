using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;

namespace Wisp.Science.Preview;

internal sealed partial class NativeAutomationPage
{
    private void RenderMandateForm()
    {
        var wasNew = !ReferenceEquals(renderedMandate, model.MandateDraft); renderedMandate = model.MandateDraft; renderedKpis = model.MandateDraft?.Kpis.Count ?? -1; mandateProjects = ProjectSignature();
        mandateForm.Children.Clear(); mandateGrids.Clear(); mandateKpiGrids.Clear(); mandateFormInputs.Clear(); mandateSelectors.Clear(); mandateDate = null;
        mandateFormCard.Visibility = renderedMandate == null ? Visibility.Collapsed : Visibility.Visible; if (renderedMandate is not { } draft) return;
        mandateForm.Children.Add(Text(draft.Id == null ? L("新建职责", "New mandate") : L("编辑职责", "Edit mandate"), 15, bold: true));
        var first = MandateGrid(); mandateForm.Children.Add(first);
        var project = new ComboBox { MinWidth = 0, HorizontalAlignment = HorizontalAlignment.Stretch, Tag = draft.Id == null };
        foreach (var p in model.Snapshot?.Projects ?? []) project.Items.Add(new ComboBoxItem { Content = p.Name, Tag = p.Id });
        project.SelectedItem = project.Items.OfType<ComboBoxItem>().FirstOrDefault(p => (string)p.Tag == draft.ProjectId);
        project.SelectionChanged += (_, _) => draft.ProjectId = (project.SelectedItem as ComboBoxItem)?.Tag as string ?? ""; mandateSelectors.Add(project); MandateField(first, L("项目", "Project"), project);
        var name = MandateEdit(first, L("名称", "Name"), draft.Name, v => draft.Name = v);
        mandateDate = new CalendarDatePicker { MinWidth = 0, HorizontalAlignment = HorizontalAlignment.Stretch, PlaceholderText = L("不限日期", "No end date"), DateFormat = "{year.full}-{month.integer(2)}-{day.integer(2)}" };
        if (DateTime.TryParseExact(draft.Until, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out var date)) mandateDate.Date = new DateTimeOffset(date);
        var dateRow = new Grid { ColumnSpacing = 4 }; Columns(dateRow, new(1, GridUnitType.Star), GridLength.Auto); dateRow.Children.Add(mandateDate);
        var clearDate = Tool(L("清除截止日期", "Clear end date"), "close", () => mandateDate.Date = null); clearDate.IsEnabled = mandateDate.Date != null; Put(dateRow, clearDate, 0, 1);
        mandateDate.DateChanged += (_, _) => { draft.Until = mandateDate.Date?.ToString("yyyy-MM-dd") ?? ""; clearDate.IsEnabled = mandateDate.Date != null; };
        AutomationProperties.SetName(mandateDate, L("截止日期", "Until")); MandateField(first, L("截止日期", "Until"), new ContentControl { Content = dateRow, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        MandateEdit(first, L("目标", "Goal"), draft.Goal, v => draft.Goal = v, 4, L("为谁工作、希望达成什么结果、持续多久？", "Who is this for, what should be achieved, and for how long?"));
        var kpis = new StackPanel { Spacing = 8 }; kpis.Children.Add(Text(L("怎样衡量进展：写清统计口径、目标值和统计周期。", "How progress is measured: say what is counted, the target, and the period it applies to."), 12, "text-faint"));
        for (var i = 0; i < draft.Kpis.Count; i++)
        {
            var index = i; var k = draft.Kpis[i]; var row = new Grid { ColumnSpacing = 8, RowSpacing = 8 }; mandateKpiGrids.Add(row);
            void Cell(string label, string value, System.Action<string> set) { var box = Input(value, label); box.PlaceholderText = label; box.TextChanged += (_, _) => set(box.Text); mandateFormInputs.Add(box); row.Children.Add(box); }
            Cell(L("指标名称", "KPI name"), k.Name, v => k.Name = v); Cell(L("统计口径", "How it is counted"), k.Definition, v => k.Definition = v); Cell(L("目标值", "Target"), k.Target, v => k.Target = v); Cell(L("统计周期", "Period"), k.Period, v => k.Period = v);
            var remove = Tool(L("移除指标", "Remove KPI"), "minus", () => model.ChangeMandateKpi(index)); mandateFormInputs.Add(remove); row.Children.Add(remove); AutomationProperties.SetAutomationId(row, "mandate-kpi-row"); kpis.Children.Add(row);
        }
        var add = Tool(L("添加指标", "Add KPI"), "plus", () => model.ChangeMandateKpi(), true); add.HorizontalAlignment = HorizontalAlignment.Left; mandateFormInputs.Add(add); AutomationProperties.SetAutomationId(add, "mandate-add-kpi"); kpis.Children.Add(add);
        mandateForm.Children.Add(MandateFieldset(L("KPI", "KPIs"), kpis));
        var constraints = MandateGrid(); MandateEdit(constraints, L("可自行完成", "May do on its own"), draft.Autonomous, v => draft.Autonomous = v, 2);
        MandateEdit(constraints, L("需要先审核", "Needs your review first"), draft.ReviewFirst, v => draft.ReviewFirst = v, 2);
        MandateEdit(constraints, L("何时请求协助", "Ask for help when"), draft.AskForHelp, v => draft.AskForHelp = v, 2);
        MandateEdit(constraints, L("长期要求（每行一条）", "Standing instructions (one per line)"), draft.Notes, v => draft.Notes = v, 3,
            L("之后的工作应当怎样做。在职责对话里确认过的反馈会加到这里。", "How the work should go from now on. Feedback you confirm in the mandate's conversation is added here."));
        var review = new CheckBox { Content = Text(L("修改文件、运行命令或提交任何内容之前先问我", "Ask me before anything that changes files, runs commands or submits work"), 12), IsChecked = draft.ReviewMutations, Tag = true, MinWidth = 0 };
        review.Click += (_, _) => draft.ReviewMutations = review.IsChecked == true; mandateFormInputs.Add(review); constraints.Children.Add(review); AutomationProperties.SetAutomationId(review, "mandate-review-mutations");
        mandateForm.Children.Add(MandateFieldset(L("工作约束", "Constraints"), constraints));
        var rhythm = MandateGrid();
        Number(L("默认每隔（小时）", "Check in every (hours)"), draft.Hours, 1, 720, v => draft.Hours = v);
        Number(L("汇报间隔（天）", "Report every (days)"), draft.ReportDays, 1, 90, v => draft.ReportDays = v);
        Number(L("最短间隔（分钟）", "Soonest next round (minutes)"), draft.MinMinutes, 5, 43200, v => draft.MinMinutes = v);
        Number(L("最长间隔（天）", "Latest next round (days)"), draft.MaxDays, 1, 30, v => draft.MaxDays = v);
        Number(L("每天最多回合", "Rounds per day at most"), draft.MaxRounds, 1, 48, v => draft.MaxRounds = v);
        void Number(string label, int value, int min, int max, System.Action<int> set) { var box = new NumberBox { Value = value, Minimum = min, Maximum = max, SmallChange = 1, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Inline, HorizontalAlignment = HorizontalAlignment.Stretch, MinWidth = 0 }; box.ValueChanged += (_, _) => { if (double.IsFinite(box.Value)) set((int)box.Value); }; MandateField(rhythm, label, box); }
        mandateForm.Children.Add(MandateFieldset(L("工作节奏", "Rhythm"), rhythm));
        mandateForm.Children.Add(Text(L("职责在所选项目中使用自己的一条对话，创建后立即开始第一轮。仅在 Wisp 运行时工作，错过的回合会在下次启动时补跑一次。", "The mandate works in its own conversation in the project and starts its first round right away. Rounds run only while Wisp is open; a missed round runs once on the next launch."), 12, "text-faint"));
        mandateFormError = Text("", 12, "err"); AutomationProperties.SetLiveSetting(mandateFormError, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive); mandateForm.Children.Add(mandateFormError);
        var actions = new NativeActionWrap { Gap = 12, RightAligned = true };
        var cancel = new Button { Content = L("取消", "Cancel") }; design.ActionButton(cancel); cancel.Click += (_, _) => model.CancelMandate(); actions.Children.Add(cancel);
        var save = Primary(draft.Id == null ? L("创建", "Create") : L("保存", "Save"), () => _ = model.SaveMandateAsync()); AutomationProperties.SetAutomationId(save, "mandate-save"); mandateFormInputs.Add(save); actions.Children.Add(save); mandateForm.Children.Add(actions);
        if (wasNew) DispatcherQueue.TryEnqueue(() =>
        {
            if (disposed || !ReferenceEquals(model.MandateDraft, draft)) return;
            // A picker restores the previous focus when it closes. Keep that focus
            // in the new form, so Escape cannot scroll back to its template button.
            name.Focus(FocusState.Programmatic);
            mandateFormCard.StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 });
        });
    }
    private Grid MandateGrid() { var grid = new Grid { ColumnSpacing = 12, RowSpacing = 12 }; mandateGrids.Add(grid); return grid; }
    private TextBox MandateEdit(Grid grid, string label, string value, System.Action<string> set, int rows = 0, string? placeholder = null)
    {
        var input = Input(value, label); if (rows > 0) { input.AcceptsReturn = true; input.TextWrapping = TextWrapping.Wrap; input.MinHeight = rows * 14 + 12; }
        if (placeholder != null) input.PlaceholderText = placeholder; input.TextChanged += (_, _) => set(input.Text); MandateField(grid, label, input, rows > 0); return input;
    }
    private void MandateField(Grid grid, string label, Control input, bool span = false)
    {
        input.MinWidth = 0; input.MinHeight = Math.Max(input.MinHeight, 30); input.Background = design.Brush("bg-input"); input.Foreground = design.Brush("text"); input.BorderBrush = design.Brush("border"); input.CornerRadius = new CornerRadius(4);
        design.BindTypography(input, size: 12); AutomationProperties.SetName(input, label); mandateFormInputs.Add(input);
        var field = new StackPanel { Spacing = 4, Tag = span }; field.Children.Add(Text(label, 12, "text-muted")); field.Children.Add(input); grid.Children.Add(field);
    }
    private UIElement MandateFieldset(string title, UIElement content)
    {
        var frame = new Grid(); var border = Card(content); border.Padding = new Thickness(12, 16, 12, 12); border.Margin = new Thickness(0, 7, 0, 0); frame.Children.Add(border);
        frame.Children.Add(new Border { Child = Text(title, 12, "text-muted"), Background = design.Brush("bg-elev"), Padding = new Thickness(6, 0, 6, 0), Margin = new Thickness(12, 0, 0, 0), VerticalAlignment = VerticalAlignment.Top, HorizontalAlignment = HorizontalAlignment.Left }); return frame;
    }
    private void ArrangeMandates(double width)
    {
        ArrangeGrid(mandateTemplates, mandateTemplates.Children.Cast<FrameworkElement>().ToArray(), Math.Max(1, (int)((width + 12) / 272)), false);
        foreach (var grid in mandateGrids) ArrangeGrid(grid, grid.Children.Cast<FrameworkElement>().ToArray(), Math.Max(1, (int)((width - 64 + 12) / 212)), true);
        foreach (var grid in mandateKpiGrids)
        {
            if (ActualWidth <= 640) ArrangeGrid(grid, grid.Children.Cast<FrameworkElement>().ToArray(), 2, false);
            else { grid.ColumnDefinitions.Clear(); grid.RowDefinitions.Clear(); Rows(grid, GridLength.Auto); Columns(grid, new(1.2, GridUnitType.Star), new(2, GridUnitType.Star), new(.6, GridUnitType.Star), new(1, GridUnitType.Star), GridLength.Auto);
                for (var i = 0; i < grid.Children.Count; i++) { var child = (FrameworkElement)grid.Children[i]; Grid.SetRow(child, 0); Grid.SetColumn(child, i); Grid.SetColumnSpan(child, 1); } }
        }
        foreach (var head in mandateHeads) { var narrow = ActualWidth <= 640; head.RowDefinitions.Clear(); Rows(head, GridLength.Auto); if (narrow) Rows(head, GridLength.Auto); Grid.SetColumnSpan((FrameworkElement)head.Children[0], narrow ? 2 : 1); Grid.SetColumn((FrameworkElement)head.Children[1], narrow ? 0 : 1); Grid.SetRow((FrameworkElement)head.Children[1], narrow ? 1 : 0); Grid.SetColumnSpan((FrameworkElement)head.Children[1], narrow ? 2 : 1); }
    }
    private bool CloseMandatePicker()
    {
        if (mandateDate?.IsCalendarOpen == true) { mandateDate.IsCalendarOpen = false; return true; }
        if (mandateSelectors.LastOrDefault(s => s.IsDropDownOpen) is { } select) { select.IsDropDownOpen = false; return true; }
        return false;
    }
}
