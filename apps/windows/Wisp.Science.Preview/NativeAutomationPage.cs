using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeAutomationPage : UserControl, IWorkspaceSheet
{
    private readonly NativeAutomationModel model;
    private readonly WispDesign design;
    private readonly Action close;
    private readonly StackPanel page = new() { Spacing = 20, MaxWidth = 1120, HorizontalAlignment = HorizontalAlignment.Stretch };
    private readonly ScrollViewer scroll;
    private readonly Grid heading = new() { ColumnSpacing = 12, RowSpacing = 12 }, templates = new() { ColumnSpacing = 12, RowSpacing = 12 };
    private readonly StackPanel daily = new(), form = new() { Spacing = 12 }, rows = new() { Spacing = 10 };
    private readonly Border formCard;
    private readonly TextBlock notice, dailyStatus, dailyError, formError;
    private readonly Button newTask, refresh, runDaily, create;
    private readonly CheckBox dailyEnabled;
    private readonly TextBox dailyTime;
    private readonly CheckBox memoryEnabled;
    private readonly Button runMemory;
    private readonly TextBlock memoryStatus, memoryError;
    private readonly StackPanel proposals = new() { Spacing = 10, Margin = new Thickness(0, 10, 0, 0) };
    private readonly Grid fields = new() { ColumnSpacing = 12, RowSpacing = 12 };
    private readonly List<ComboBox> selectors = [];
    private readonly List<Control> inputs = [];
    private readonly List<Grid> taskRows = [];
    private readonly List<FrameworkElement> templateCards = [];
    private readonly DispatcherTimer timer = new() { Interval = TimeSpan.FromSeconds(3) };
    private NativeScheduleDraft? renderedDraft;
    private NativeScheduleCadence? renderedCadence;
    private string formProjects = "";
    private bool rendering, disposed, loaded;

    public NativeAutomationPage(NativeAutomationModel model, WispDesign design, Action close, Action? openSpecialists)
    {
        this.model = model; this.design = design; this.close = close; model.Chinese = design.Localization.Locale == "zh";
        Background = design.Brush("bg-app"); design.BindTypography(this);
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        AutomationProperties.SetAutomationId(this, "home-automation");
        scroll = new() { Content = page, Background = design.Brush("bg-app"), HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Content = scroll;
        var back = Tool(L("返回科研助理", "Back to research assistant"), "arrow-left", close, true); back.Padding = new Thickness(0, 6, 0, 6); back.HorizontalAlignment = HorizontalAlignment.Left;
        back.Margin = new Thickness(0, 0, 0, 16); page.Children.Add(back);
        var headingCopy = new StackPanel { Spacing = 6 }; headingCopy.Children.Add(Text(L("自动化", "Automation"), 26, bold: true));
        headingCopy.Children.Add(Text(L("按计划运行任务，或在需要时随时执行。", "Run tasks on a schedule while Wisp is open, or any time you need them."), 13, "text-muted"));
        Columns(heading, new(1, GridUnitType.Star), GridLength.Auto); Rows(heading, GridLength.Auto, GridLength.Auto); heading.Children.Add(headingCopy);
        var headingActions = new NativeActionWrap { Gap = 8, VerticalAlignment = VerticalAlignment.Center };
        refresh = Tool(L("刷新自动化", "Refresh automation"), "refresh", () => _ = model.RefreshAsync());
        newTask = Primary(L("创建定时任务", "New scheduled task"), () => model.Begin(), "plus");
        AutomationProperties.SetAutomationId(newTask, "automation-create"); headingActions.Children.Add(refresh); headingActions.Children.Add(newTask); Put(heading, headingActions, 0, 1); page.Children.Add(heading);
        notice = Text("", 12, "text-muted"); AutomationProperties.SetLiveSetting(notice, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite); page.Children.Add(notice);
        var dailyHead = new Grid { ColumnSpacing = 10 }; Columns(dailyHead, new(1, GridUnitType.Star), GridLength.Auto);
        var dailyTitle = new NativeActionWrap { Gap = 10 }; dailyTitle.Children.Add(design.Icon("sparkles", 17, design.ColorText("clay"))); dailyTitle.Children.Add(Text(L("每日研究回顾", "Daily research recap"), 15, bold: true));
        dailyTitle.Children.Add(new Border { Child = Text(L("内置", "Built-in"), 11, "clay-strong"), Background = design.Brush("clay-soft"), CornerRadius = new CornerRadius(99), Padding = new Thickness(8, 1, 8, 1) });
        dailyHead.Children.Add(dailyTitle); dailyEnabled = Check(L("启用每日研究回顾", "Enable daily research recap"), async value => await model.SaveDailyAsync(value, dailyTime!.Text));
        Put(dailyHead, dailyEnabled, 0, 1); daily.Children.Add(dailyHead);
        dailyTime = Input("09:00", L("每日回顾时间", "Daily recap time")); dailyTime.Width = 83; dailyTime.MinHeight = 26; dailyTime.Padding = new Thickness(6, 1, 6, 1);
        dailyTime.VerticalContentAlignment = VerticalAlignment.Center; design.BindTypography(dailyTime, size: 14);
        AutomationProperties.SetAutomationId(dailyTime, "daily-recap-time");
        dailyTime.LostFocus += async (_, _) => { if (!rendering && model.Ready && dailyTime.Text != model.Snapshot!.DailyRecap!.Time) await model.SaveDailyAsync(dailyEnabled.IsChecked == true, dailyTime.Text); };
        var description = new RichTextBlock { FontFamily = design.UiFont, FontSize = design.FontSize(14), Foreground = design.Brush("text-muted"), LineHeight = design.FontSize(23.8), Margin = new Thickness(0, 10, 0, 6) };
        var paragraph = new Paragraph(); paragraph.Inlines.Add(new Run { Text = L("每天 ", "Every day after ") });
        paragraph.Inlines.Add(new InlineUIContainer { Child = dailyTime });
        paragraph.Inlines.Add(new Run { Text = L(" 起，为各项目最近有记录的日子起草研究回顾（最多补齐 3 天）。草稿在研究历程中确认后才算正式记录。", " it drafts a recap for each project's recent days with recorded activity (catching up to 3 days). Drafts become records only after you confirm them in the research journey.") });
        description.Blocks.Add(paragraph); daily.Children.Add(description);
        dailyStatus = Text("", 12, "text-faint"); daily.Children.Add(dailyStatus); dailyError = Text("", 12, "err"); daily.Children.Add(dailyError);
        var dailyActions = new NativeActionWrap { Gap = 12, Margin = new Thickness(0, 12, 0, 0) };
        runDaily = Tool(L("立即运行", "Run now"), "play", () => _ = model.RunDailyAsync(), true); design.ActionButton(runDaily); dailyActions.Children.Add(runDaily);
        var specialists = Tool(L("在 设置 → 专家 → Recap 中更换模型", "Change model in Settings → Specialists → Recap"), "gear", () => openSpecialists?.Invoke(), true);
        specialists.IsEnabled = openSpecialists != null; dailyActions.Children.Add(specialists); daily.Children.Add(dailyActions);
        var dailyCard = Card(daily); AutomationProperties.SetAutomationId(dailyCard, "automation-daily-recap"); page.Children.Add(dailyCard);
        var memory = new StackPanel();
        var memoryHead = new Grid { ColumnSpacing = 10 }; Columns(memoryHead, new(1, GridUnitType.Star), GridLength.Auto);
        var memoryTitle = new NativeActionWrap { Gap = 10 }; memoryTitle.Children.Add(design.Icon("memory", 17, design.ColorText("clay"))); memoryTitle.Children.Add(Text(L("记忆整理", "Memory tidy"), 15, bold: true));
        memoryTitle.Children.Add(new Border { Child = Text(L("内置", "Built-in"), 11, "clay-strong"), Background = design.Brush("clay-soft"), CornerRadius = new CornerRadius(99), Padding = new Thickness(8, 1, 8, 1) });
        memoryHead.Children.Add(memoryTitle); memoryEnabled = Check(L("启用记忆整理", "Enable memory tidy"), model.SetMemoryTidyAsync);
        AutomationProperties.SetAutomationId(memoryEnabled, "memory-tidy-enabled"); Put(memoryHead, memoryEnabled, 0, 1); memory.Children.Add(memoryHead);
        var memoryDescription = Text(L("每周检查一次全局习惯，提出合并意思相同的条目，或淘汰已被新条目取代的条目。在你应用某条建议之前，不会改动任何记忆。", "Once a week it looks over your global habits and proposes merging entries that say the same thing or retiring one that a newer entry replaces. Nothing changes until you apply a proposal."), 14, "text-muted");
        memoryDescription.Margin = new Thickness(0, 10, 0, 6); memoryDescription.LineHeight = design.FontSize(23.8); memory.Children.Add(memoryDescription);
        memoryStatus = Text("", 12, "text-faint"); AutomationProperties.SetAutomationId(memoryStatus, "memory-tidy-status"); memory.Children.Add(memoryStatus);
        memoryError = Text("", 12, "err"); memory.Children.Add(memoryError); memory.Children.Add(proposals);
        var memoryActions = new NativeActionWrap { Gap = 12, Margin = new Thickness(0, 12, 0, 0) };
        runMemory = Tool(L("立即运行", "Run now"), "play", () => _ = model.RunMemoryTidyAsync(), true); design.ActionButton(runMemory); AutomationProperties.SetAutomationId(runMemory, "memory-tidy-run"); memoryActions.Children.Add(runMemory);
        var memorySpecialist = Tool(L("使用 Recap 专家的模型", "It uses the Recap specialist's model"), "gear", () => openSpecialists?.Invoke(), true); memorySpecialist.IsEnabled = openSpecialists != null; memoryActions.Children.Add(memorySpecialist); memory.Children.Add(memoryActions);
        var memoryCard = Card(memory); AutomationProperties.SetAutomationId(memoryCard, "automation-memory-tidy"); page.Children.Add(memoryCard);
        BuildMandates();
        formCard = Card(form); formCard.Visibility = Visibility.Collapsed; AutomationProperties.SetAutomationId(formCard, "automation-form"); page.Children.Add(formCard);
        formError = Text("", 12, "err"); create = Primary(L("创建", "Create"), () => _ = model.CreateAsync()); AutomationProperties.SetAutomationId(create, "automation-save");
        var scheduled = new StackPanel { Spacing = 10 }; scheduled.Children.Add(Text(L("定时任务", "Scheduled tasks"), 15, bold: true)); scheduled.Children.Add(rows); page.Children.Add(scheduled);
        var templateSection = new StackPanel { Spacing = 10 }; templateSection.Children.Add(Text(L("定时任务模板", "Templates"), 15, bold: true)); templateSection.Children.Add(templates); page.Children.Add(templateSection);
        for (var index = 0; index < 3; index++)
        {
            var selected = index; var draft = NativeAutomationTemplates.Create(index, model.Chinese);
            var content = new StackPanel { Spacing = 8 }; var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            label.Children.Add(design.Icon(NativeAutomationTemplates.Icons[index], 17)); label.Children.Add(Text(draft.Name, 14, bold: true)); content.Children.Add(label);
            var preview = Text(draft.Prompt, 12, "text-muted"); preview.MaxLines = 2; preview.TextTrimming = TextTrimming.CharacterEllipsis; content.Children.Add(preview);
            content.Children.Add(Text((draft.Cadence == NativeScheduleCadence.Daily ? L("每天", "Daily") : NativeAutomationModel.Weekday(draft.Weekday, model.Chinese)) + " " + draft.Time, 12, "text-faint"));
            var button = new Button { Content = content, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top, HorizontalAlignment = HorizontalAlignment.Stretch };
            design.ActionButton(button); button.Padding = new Thickness(16, 14, 16, 14); button.Click += (_, _) => model.Begin(selected);
            AutomationProperties.SetName(button, draft.Name); templateCards.Add(button); templates.Children.Add(button);
        }
        SizeChanged += (_, _) => Arrange(); model.Changed += Render;
        timer.Tick += async (_, _) => { if (!disposed && (model.Snapshot?.DailyRecap?.Running == true || model.Snapshot?.MemoryTidy?.Running == true)) await model.RefreshAsync(); };
        Loaded += async (_, _) => { if (loaded) return; loaded = true; Arrange(); timer.Start(); await model.RefreshAsync(); };
        Render();
    }
    private void Render()
    {
        if (disposed) return; rendering = true;
        notice.Text = model.Error ?? model.Notice ?? ""; notice.Foreground = design.Brush(model.Error != null ? "err" : "text-muted"); notice.Visibility = notice.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        newTask.IsEnabled = model.Ready; refresh.IsEnabled = !model.Busy;
        var recap = model.Snapshot?.DailyRecap;
        dailyEnabled.IsEnabled = model.Ready; dailyEnabled.IsChecked = recap?.Enabled; dailyEnabled.Content = Text(recap == null ? L("未知", "Unknown") : recap.Enabled ? L("已开启", "On") : L("已关闭", "Off"), 12, "text-muted");
        dailyTime.IsEnabled = model.Ready;
        if (dailyTime.FocusState == FocusState.Unfocused && recap != null) dailyTime.Text = recap.Time;
        dailyStatus.Text = recap == null ? UnavailableStatus() : recap.Running ? L("正在运行…", "Running now…")
            : recap.LastRunAt is { } at ? L($"上次运行：{Stamp(at)} · 起草 {recap.Drafted} 份", $"Last run: {Stamp(at)} · {recap.Drafted} drafted") : L("尚未运行", "Not run yet");
        dailyError.Text = recap?.Error ?? ""; dailyError.Visibility = dailyError.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed; runDaily.IsEnabled = model.Ready && recap?.Running != true;
        RenderMemory();
        RenderMandates();
        RenderRows();
        if (!ReferenceEquals(renderedDraft, model.Draft) || renderedCadence != model.Draft?.Cadence || formProjects != ProjectSignature()) RenderForm();
        formError.Text = model.FormError ?? ""; formError.Visibility = model.FormError == null ? Visibility.Collapsed : Visibility.Visible; create.IsEnabled = model.Ready;
        foreach (var input in inputs) input.IsEnabled = model.Ready;
        foreach (var template in templateCards) ((Button)template).IsEnabled = model.Ready;
        Arrange(); rendering = false;
    }
    private void RenderMemory()
    {
        var tidy = model.Snapshot?.MemoryTidy;
        memoryEnabled.IsEnabled = model.Ready; memoryEnabled.IsChecked = tidy?.Enabled;
        memoryEnabled.Content = Text(tidy == null ? L("未知", "Unknown") : tidy.Enabled ? L("已开启", "On") : L("已关闭", "Off"), 12, "text-muted");
        memoryStatus.Text = tidy == null ? UnavailableStatus() : tidy.Running ? L("正在运行…", "Running now…")
            : tidy.LastRunAt is { } at ? L($"上次运行：{Stamp(at)} · {tidy.Proposals.Length} 条建议待处理", $"Last run: {Stamp(at)} · {tidy.Proposals.Length} proposal(s) to review") : L("尚未运行", "Not run yet");
        memoryError.Text = tidy?.Error ?? ""; memoryError.Visibility = memoryError.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        runMemory.IsEnabled = model.Ready && tidy?.Running != true;
        proposals.Children.Clear(); proposals.Visibility = tidy?.Proposals.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        foreach (var proposal in tidy?.Proposals ?? [])
        {
            var content = new StackPanel { Spacing = 4 }; var merge = proposal.Action == "merge"; var count = proposal.Memories.Length;
            content.Children.Add(Text(merge ? L($"合并 {count} 条记忆", $"Merge {count} memories") : L($"淘汰 {count} 条记忆", $"Retire {count} memory(ies)"), 14, bold: true));
            if (proposal.Reason.Length > 0) content.Children.Add(Text(proposal.Reason, 12, "text-faint"));
            var entries = new StackPanel { Margin = new Thickness(0, 2, 0, 0) };
            foreach (var entry in proposal.Memories)
            {
                var line = new Grid { ColumnSpacing = 8 }; Columns(line, new(10), new(1, GridUnitType.Star));
                line.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Width = 3, Height = 3, Fill = design.Brush("text-muted"), VerticalAlignment = VerticalAlignment.Top, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 8, 0, 0) });
                Put(line, Text(entry.Content, 14, "text-muted"), 0, 1); entries.Children.Add(line);
            }
            content.Children.Add(entries);
            if (merge)
            {
                var merged = Text("", 14); merged.Margin = new Thickness(0, 2, 0, 0);
                merged.Inlines.Add(new Run { Text = L("合并为", "Becomes") + "  ", FontWeight = Microsoft.UI.Text.FontWeights.SemiBold }); merged.Inlines.Add(new Run { Text = proposal.Text }); content.Children.Add(merged);
            }
            var actions = new NativeActionWrap { Gap = 12, Margin = new Thickness(0, 6, 0, 0) };
            var apply = Primary(L("应用", "Apply"), () => _ = model.SettleMemoryAsync(proposal.Id, true)); AutomationProperties.SetAutomationId(apply, "memory-tidy-apply"); apply.IsEnabled = model.Ready;
            var dismiss = new Button { Content = L("忽略", "Dismiss") }; design.ActionButton(dismiss); AutomationProperties.SetAutomationId(dismiss, "memory-tidy-dismiss"); dismiss.IsEnabled = model.Ready;
            dismiss.Click += (_, _) => _ = model.SettleMemoryAsync(proposal.Id, false); actions.Children.Add(apply); actions.Children.Add(dismiss); content.Children.Add(actions);
            var card = Card(content); card.Padding = new Thickness(12, 10, 12, 10); card.CornerRadius = new CornerRadius(4); AutomationProperties.SetAutomationId(card, "memory-tidy-proposal"); proposals.Children.Add(card);
        }
    }
    private void RenderRows()
    {
        rows.Children.Clear(); taskRows.Clear();
        if (model.Snapshot?.Schedules is not { } schedules) { rows.Children.Add(Text(model.Error ?? L("正在读取…", "Loading…"), 12, model.Error == null ? "text-faint" : "err")); return; }
        if (schedules.Length == 0)
        {
            var text = Text(L("还没有定时任务。可以从下方模板开始，或自己写一段提示词。", "No scheduled tasks yet. Start from a template below or write your own prompt."), 14, "text-muted"); text.TextAlignment = TextAlignment.Center;
            var empty = new Grid();
            empty.Children.Add(new Microsoft.UI.Xaml.Shapes.Rectangle { Stroke = design.Brush("border"), StrokeThickness = 1, StrokeDashArray = new DoubleCollection { 3, 3 }, RadiusX = 8, RadiusY = 8, IsHitTestVisible = false });
            empty.Children.Add(new Border { Child = text, Padding = new Thickness(16, 28, 16, 28) }); rows.Children.Add(empty); return;
        }
        foreach (var schedule in schedules)
        {
            var row = new Grid { ColumnSpacing = 16, RowSpacing = 10 }; Columns(row, new(1, GridUnitType.Star), GridLength.Auto); Rows(row, GridLength.Auto, GridLength.Auto);
            var main = new StackPanel { Spacing = 3 }; main.Children.Add(Text(schedule.Name, 14, bold: true));
            var next = schedule.Enabled ? L("下次 ", "Next ") + Stamp(schedule.NextRunAt) : L("已暂停", "Paused");
            main.Children.Add(Text(model.Snapshot.Projects.First(p => p.Id == schedule.ProjectId).Name + " · " + model.Cadence(schedule) + " · " + next, 12, "text-faint"));
            var prompt = Text(schedule.Prompt, 14, "text-muted"); prompt.MaxLines = 2; prompt.TextTrimming = TextTrimming.CharacterEllipsis; prompt.Margin = new Thickness(0, 4, 0, 0); main.Children.Add(prompt); row.Children.Add(main);
            var actions = new NativeActionWrap { Gap = 4, VerticalAlignment = VerticalAlignment.Top };
            var toggle = Check(L("启用 ", "Enable ") + schedule.Name, value => model.SetEnabledAsync(schedule.Id, value)); toggle.IsChecked = schedule.Enabled; toggle.Margin = new Thickness(0, 0, 6, 0); actions.Children.Add(toggle);
            actions.Children.Add(Tool(L("立即运行 ", "Run now ") + schedule.Name, "play", () => _ = model.RunAsync(schedule.Id)));
            var delete = Tool(L("删除 ", "Delete ") + schedule.Name, "trash", () => _ = model.DeleteAsync(schedule.Id));
            if (model.Deleting == schedule.Id) { delete.Content = design.Icon("trash", 17, design.ColorText("err")); ToolTipService.SetToolTip(delete, L("再次点击确认删除", "Click again to delete")); AutomationProperties.SetName(delete, L("再次点击确认删除 ", "Click again to delete ") + schedule.Name); }
            actions.Children.Add(delete); foreach (Control control in actions.Children) control.IsEnabled = model.Ready;
            Put(row, actions, 0, 1); taskRows.Add(row); var card = Card(row); card.Padding = new Thickness(14, 12, 14, 12); card.Opacity = schedule.Enabled ? 1 : .7; rows.Children.Add(card);
        }
    }
    private void RenderForm()
    {
        var newDraft = !ReferenceEquals(renderedDraft, model.Draft); renderedDraft = model.Draft; renderedCadence = model.Draft?.Cadence; formProjects = ProjectSignature();
        if (create.Parent is Panel previousActions) previousActions.Children.Remove(create);
        selectors.Clear(); inputs.Clear(); form.Children.Clear(); fields.Children.Clear(); fields.ColumnDefinitions.Clear(); fields.RowDefinitions.Clear();
        formCard.Visibility = model.Draft == null ? Visibility.Collapsed : Visibility.Visible; if (model.Draft is not { } draft) return;
        form.Children.Add(Text(L("创建定时任务", "New scheduled task"), 15, bold: true)); form.Children.Add(fields);
        var project = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch };
        foreach (var p in model.Snapshot?.Projects ?? []) project.Items.Add(new ComboBoxItem { Content = p.Name, Tag = p.Id });
        project.SelectedItem = project.Items.OfType<ComboBoxItem>().FirstOrDefault(p => (string)p.Tag == draft.ProjectId); project.SelectionChanged += (_, _) => draft.ProjectId = (project.SelectedItem as ComboBoxItem)?.Tag as string ?? "";
        AddField(L("项目", "Project"), project);
        var name = Input(draft.Name, L("名称", "Name")); name.TextChanged += (_, _) => draft.Name = name.Text; AddField(L("名称", "Name"), name);
        var cadence = new ComboBox { ItemsSource = new[] { L("每天", "Daily"), L("每周", "Weekly"), L("每隔几小时", "Every few hours") }, SelectedIndex = (int)draft.Cadence, HorizontalAlignment = HorizontalAlignment.Stretch };
        cadence.SelectionChanged += (_, _) => { if (cadence.SelectedIndex >= 0) { draft.Cadence = (NativeScheduleCadence)cadence.SelectedIndex; RenderForm(); Arrange(); } }; AddField(L("频率", "Repeat"), cadence);
        if (draft.Cadence == NativeScheduleCadence.Hourly)
        {
            var hours = Input(draft.Hours.ToString(), L("间隔（小时）", "Every (hours)")); hours.TextChanged += (_, _) => draft.Hours = int.TryParse(hours.Text, out var value) ? value : 1; AddField(L("间隔（小时）", "Every (hours)"), hours);
        }
        else
        {
            if (draft.Cadence == NativeScheduleCadence.Weekly)
            {
                var days = new[] { 1, 2, 3, 4, 5, 6, 0 }; var weekday = new ComboBox { ItemsSource = days.Select(d => NativeAutomationModel.Weekday(d, model.Chinese)).ToArray(), SelectedIndex = Array.IndexOf(days, draft.Weekday), HorizontalAlignment = HorizontalAlignment.Stretch };
                weekday.SelectionChanged += (_, _) => { if (weekday.SelectedIndex >= 0) draft.Weekday = days[weekday.SelectedIndex]; }; AddField(L("星期", "Day"), weekday);
            }
            var time = Input(draft.Time, L("时间", "Time")); time.TextChanged += (_, _) => draft.Time = time.Text; AddField(L("时间", "Time"), time);
        }
        var prompt = Input(draft.Prompt, L("提示词", "Prompt")); prompt.AcceptsReturn = true; prompt.TextWrapping = TextWrapping.Wrap; prompt.MinHeight = 118;
        prompt.TextChanged += (_, _) => draft.Prompt = prompt.Text; AddField(L("提示词", "Prompt"), prompt, true);
        form.Children.Add(Text(L("到点后会在所选项目中新建一个会话并发送这段提示词。仅在 Wisp 运行时触发，错过的时间会在下次启动时补跑一次。", "When due, Wisp starts a new session in the project and sends this prompt. Tasks run only while Wisp is open; a missed time runs once on the next launch."), 12, "text-faint"));
        form.Children.Add(formError);
        var actions = new NativeActionWrap { Gap = 12, RightAligned = true };
        var cancel = new Button { Content = L("取消", "Cancel") }; design.ActionButton(cancel); cancel.Click += (_, _) => model.CancelForm();
        actions.Children.Add(cancel); actions.Children.Add(create); form.Children.Add(actions);
        // Bring a newly opened inline form into view; cadence changes preserve scroll.
        if (newDraft) DispatcherQueue.TryEnqueue(() => { if (!disposed && ReferenceEquals(model.Draft, draft)) formCard.StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = .1 }); });
    }
    private void AddField(string label, Control input, bool span = false)
    {
        inputs.Add(input);
        design.BindTypography(input, size: 12); AutomationProperties.SetName(input, label);
        if (input is ComboBox combo) { selectors.Add(combo); input.Background = design.Brush("bg-input"); input.Foreground = design.Brush("text"); input.BorderBrush = design.Brush("border"); input.CornerRadius = new CornerRadius(4); input.MinHeight = 30; }
        var field = new StackPanel { Spacing = 4, Tag = span }; field.Children.Add(Text(label, 12, "text-muted")); field.Children.Add(input); fields.Children.Add(field);
    }
    private string ProjectSignature() => string.Join("\n", (model.Snapshot?.Projects ?? []).Select(p => p.Id + ":" + p.Name));
    private string UnavailableStatus() => model.Error == null ? L("正在读取…", "Loading…") : L("暂不可用，请刷新重试。", "Unavailable. Refresh to retry.");
    private void Arrange()
    {
        if (ActualWidth <= 0) return;
        var padding = Math.Clamp(ActualWidth * .025, 16, 80); scroll.Padding = new Thickness(padding, 24, padding, 24);
        var width = Math.Min(1120, ActualWidth - 2 * padding); page.Width = Math.Max(0, width);
        var headingActions = (FrameworkElement)heading.Children[1]; var compact = width < 610;
        if (heading.RowDefinitions.Count != (compact ? 2 : 1)) { heading.RowDefinitions.Clear(); Rows(heading, GridLength.Auto); if (compact) Rows(heading, GridLength.Auto); }
        Grid.SetColumnSpan((FrameworkElement)heading.Children[0], compact ? 2 : 1); Grid.SetColumn(headingActions, compact ? 0 : 1); Grid.SetRow(headingActions, compact ? 1 : 0); Grid.SetColumnSpan(headingActions, compact ? 2 : 1);
        var templateColumns = Math.Max(1, (int)((width + 12) / 272)); ArrangeGrid(templates, templateCards, templateColumns, false);
        ArrangeGrid(fields, fields.Children.Cast<FrameworkElement>().ToArray(), Math.Max(1, (int)((width - 36 + 12) / 212)), true);
        ArrangeMandates(width);
        foreach (var row in taskRows)
        {
            var narrow = ActualWidth <= 640; Grid.SetColumnSpan((FrameworkElement)row.Children[0], narrow ? 2 : 1);
            Grid.SetColumn((FrameworkElement)row.Children[1], narrow ? 0 : 1); Grid.SetRow((FrameworkElement)row.Children[1], narrow ? 1 : 0); Grid.SetColumnSpan((FrameworkElement)row.Children[1], narrow ? 2 : 1);
        }
    }
    private static void ArrangeGrid(Grid grid, IReadOnlyList<FrameworkElement> children, int columns, bool allowSpan)
    {
        grid.ColumnDefinitions.Clear(); grid.RowDefinitions.Clear(); for (var col = 0; col < columns; col++) grid.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        var index = 0; foreach (var child in children)
        {
            var span = allowSpan && child.Tag is true; if (span && index % columns != 0) index += columns - index % columns;
            Grid.SetColumn(child, index % columns); Grid.SetRow(child, index / columns); Grid.SetColumnSpan(child, span ? columns : 1); index += span ? columns : 1;
        }
        for (var row = 0; row < (index + columns - 1) / columns; row++) grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
    }
    public void HandleEscape()
    {
        if (CloseMandatePicker()) return;
        if (selectors.LastOrDefault(s => s.IsDropDownOpen) is { } open) { open.IsDropDownOpen = false; return; }
        if (!model.Escape()) close();
    }
    public void Dispose() { if (disposed) return; disposed = true; timer.Stop(); model.Changed -= Render; model.Dispose(); }
    private CheckBox Check(string label, Func<bool, Task> action)
    {
        var box = new CheckBox { MinWidth = 0, MinHeight = 24, VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetName(box, label); box.Click += async (_, _) => { if (!rendering) await action(box.IsChecked == true); }; return box;
    }
    private Button Tool(string label, string icon, Action action, bool text = false)
    { var button = design.ToolButton(label, icon, text); button.Click += (_, _) => action(); return button; }
    private Button Primary(string label, Action action, string? icon = null)
    {
        var button = new Button { Content = label }; design.ActionButton(button, true);
        if (icon != null) { var copy = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 }; copy.Children.Add(design.Icon(icon, 17, design.ColorText("on-clay"))); copy.Children.Add(Text(label, 13, "on-clay")); button.Content = copy; }
        AutomationProperties.SetName(button, label); button.Click += (_, _) => action(); return button;
    }
    private TextBox Input(string value, string label)
    { var box = new TextBox { Text = value, MinHeight = 30, Padding = new Thickness(8, 6, 8, 6), Background = design.Brush("bg-input"), Foreground = design.Brush("text"), BorderBrush = design.Brush("border"), CornerRadius = new CornerRadius(4) }; AutomationProperties.SetName(box, label); design.BindTypography(box, size: 12); return box; }
    private Border Card(UIElement child) => new() { Child = child, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Padding = new Thickness(18, 16, 18, 16) };
    private TextBlock Text(string value, double size, string color = "text", bool bold = false)
    { var text = design.Text(value, size); text.Foreground = design.Brush(color); if (bold) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; return text; }
    private string L(string zh, string en) => design.L(zh, en);
    private static string Stamp(long timestamp) => DateTimeOffset.FromUnixTimeSeconds(timestamp).LocalDateTime.ToString("yyyy-MM-dd HH:mm");
    private static void Columns(Grid grid, params GridLength[] values) { foreach (var value in values) grid.ColumnDefinitions.Add(new() { Width = value }); }
    private static void Rows(Grid grid, params GridLength[] values) { foreach (var value in values) grid.RowDefinitions.Add(new() { Height = value }); }
    private static void Put(Grid grid, FrameworkElement element, int row, int col = 0) { Grid.SetRow(element, row); Grid.SetColumn(element, col); grid.Children.Add(element); }
}
