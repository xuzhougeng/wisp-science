using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly StackPanel planProgress = new() { Spacing = 4, Margin = new Thickness(24, 0, 24, 6), Visibility = Visibility.Collapsed };
    private bool planExpanded;
    private string? planOwner, planFingerprint;
    private void RenderPlanProgress()
    {
        var plan = model.ExecutionPlan;
        planProgress.Visibility = plan == null ? Visibility.Collapsed : Visibility.Visible;
        if (plan == null) { planOwner = null; planFingerprint = null; planExpanded = false; return; }
        if (planOwner != plan.Owner) { planExpanded = false; planOwner = plan.Owner; }
        var fingerprint = $"{plan.Key}/{model.Snapshot?.Running}/{planExpanded}/{design.Typography}/{design.Dark}";
        if (planFingerprint == fingerprint) return;
        planFingerprint = fingerprint; planProgress.Children.Clear();
        var head = new Grid { ColumnSpacing = 8 };
        head.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var content = new StackPanel { Spacing = 2 };
        content.Children.Add(design.Text($"{plan.Label(model.Snapshot?.Running == true)} · {plan.Done}/{plan.Steps.Length}", 12));
        if (!planExpanded && plan.Current is { } current) content.Children.Add(new TextBlock { Text = current.Content, MaxLines = 1, TextTrimming = TextTrimming.CharacterEllipsis, FontSize = design.FontSize(12) });
        var toggle = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
        design.QuietButton(toggle);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(toggle, (planExpanded ? "收起" : "展开") + "执行计划");
        toggle.Click += (_, _) => { planExpanded = !planExpanded; RenderPlanProgress(); };
        head.Children.Add(toggle);
        if (plan.Complete)
        {
            var dismiss = design.ToolButton("关闭已完成计划", "close");
            dismiss.Click += (_, _) => model.DismissExecutionPlan(plan.Key);
            Grid.SetColumn(dismiss, 1); head.Children.Add(dismiss);
        }
        planProgress.Children.Add(head);
        planProgress.Children.Add(new ProgressBar { Minimum = 0, Maximum = plan.Steps.Length, Value = plan.Done, Height = 3 });
        if (planExpanded) planProgress.Children.Add(new ScrollViewer { Content = PlanRows(plan.Steps), MaxHeight = 120, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled });
    }
    private StackPanel PlanRows(NativePlanStep[] steps)
    {
        var rows = new StackPanel { Spacing = 6 };
        foreach (var step in steps)
        {
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            var status = new TextBlock { Text = NativeExecutionPlan.StatusLabel(step.Status), FontSize = design.FontSize(12), Foreground = design.Brush("text-muted") };
            row.Children.Add(status);
            var text = new TextBlock { Text = step.Content, TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true, FontSize = design.FontSize(12) };
            Grid.SetColumn(text, 1); row.Children.Add(text); rows.Children.Add(row);
        }
        return rows;
    }
    private bool RenderPlanTool(StackPanel card, ConversationItem item)
    {
        if (item.ToolName != "update_plan" || item.Role != "tool") return false;
        var steps = item.Ok == true && item.PlanSteps != null && NativeExecutionPlan.Valid(item.PlanSteps) ? item.PlanSteps : [];
        var body = new StackPanel { Spacing = 8 };
        if (steps.Length > 0) body.Children.Add(PlanRows(steps));
        else body.Children.Add(new TextBlock { Text = item.Ok == false ? "计划更新失败" : item.Ok == null ? "正在更新计划…" : "计划内容不可用", TextWrapping = TextWrapping.Wrap });
        body.Children.Add(RawToolDetails(item));
        var complete = steps.Length > 0 && steps.All(step => step.Status == "done");
        card.Children.Add(new Expander { Header = $"{(complete ? "计划已完成" : "执行计划")} · {steps.Count(step => step.Status == "done")}/{steps.Length}",
            Content = body, IsExpanded = !complete, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        return true;
    }
    private bool RenderPlanProposal(StackPanel card, ConversationItem item, int index)
    {
        if (item.Role != "plan") return false;
        var latest = !model.ShowingHistory && model.Snapshot is { } snapshot
            && Array.FindLastIndex(snapshot.Items, row => row.Role == "plan") == index ? model.LatestProposal : null;
        var title = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        title.Children.Add(design.Icon("plan", 16));
        title.Children.Add(design.Text("执行方案", 16)); card.Children.Add(title);
        if (latest != null && model.Snapshot?.Running == true) card.Children.Add(design.Text("正在修订计划…", 12));
        if (item.Proposal is { Valid: true } proposal)
        {
            foreach (var entry in proposal.Entries)
            {
                var row = new Grid { ColumnSpacing = 12 };
                row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
                var state = entry.Status switch { "completed" => "已完成", "in_progress" => "进行中", _ => "待执行" };
                row.Children.Add(design.Text(state + (entry.Priority == "high" ? "\n高优先级" : ""), 12));
                var content = TranscriptView.RenderMarkdown(entry.Content, design);
                Grid.SetColumn(content, 1); row.Children.Add(content); card.Children.Add(row);
            }
        }
        else card.Children.Add(ToolText(item.Text));
        if (latest != null && model.ProposalModeActive(latest))
        {
            card.Children.Add(new TextBlock { Text = "需要修改计划时，请在输入框说明调整要求。批准后会先退出计划模式，再发送当前草稿；保存并退出不会启动执行。", TextWrapping = TextWrapping.Wrap, FontSize = design.FontSize(12), Foreground = design.Brush("text-muted") });
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            var approve = new Button { Content = "批准并执行", IsEnabled = model.CanDecidePlan(latest) };
            approve.Click += async (_, _) => await model.DecidePlanAsync(latest, true, lifetime.Token);
            var save = new Button { Content = "保存并退出", IsEnabled = model.CanDecidePlan(latest) };
            save.Click += async (_, _) => await model.DecidePlanAsync(latest, false, lifetime.Token);
            actions.Children.Add(approve); actions.Children.Add(save); card.Children.Add(actions);
            if (model.PlanDecisionUncertain(latest))
            {
                var acknowledge = MessageAction("已核对计划模式"); acknowledge.IsEnabled = !model.Busy && model.ConnectionError == null;
                acknowledge.Click += (_, _) => model.AcknowledgePlanDecision(latest); card.Children.Add(acknowledge);
            }
        }
        return true;
    }
}
