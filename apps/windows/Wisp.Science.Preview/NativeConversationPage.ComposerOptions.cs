using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly ToggleSwitch fullPermission = new(), delegation = new(), autoReview = new(), failureAnalysis = new(), memory = new(), autoResume = new();
    private readonly ComboBox completionChoice = new() { Width = 148, MinWidth = 0 };
    private readonly NumberBox failureThreshold = new() { Minimum = 1, Maximum = 100, SmallChange = 1, Width = 82, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Hidden };
    private readonly NumberBox failureMinimum = new() { Minimum = 1, Maximum = 100, SmallChange = 1, Width = 82, SpinButtonPlacementMode = NumberBoxSpinButtonPlacementMode.Hidden };
    private readonly Button reviewerChoice = new(), specialistChoice = new(), computeChoice = new();
    private readonly TextBlock optionStatus = new() { TextWrapping = TextWrapping.Wrap, MaxWidth = 300, FontSize = 12 };
    private readonly Button optionReload = new() { Content = "重新读取" };
    private readonly Flyout optionSubmenu = new() { Placement = FlyoutPlacementMode.RightEdgeAlignedTop };
    private bool optionSubmenuOpen, updatingOptions;
    private FrameworkElement? planRow, thresholdRow, minimumRow, resumeRow, specialistRow;
    private readonly List<Control> submenuControls = [];

    public bool HandleComposerOptionsEscape()
    {
        if (optionSubmenuOpen) { optionSubmenu.Hide(); return true; }
        if (completionChoice.IsDropDownOpen) { completionChoice.IsDropDownOpen = false; return true; }
        if (composerOptionsOpen) { composerOptions.Hide(); return true; }
        return false;
    }

    private Grid OptionRow(string label, FrameworkElement control, bool indent = false)
    {
        var row = new Grid { MinHeight = 44, ColumnSpacing = 14, Padding = new Thickness(indent ? 16 : 0, 3, 0, 3) };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var text = design.Text(label, 14); text.VerticalAlignment = VerticalAlignment.Center;
        row.Children.Add(text); control.VerticalAlignment = VerticalAlignment.Center;
        control.HorizontalAlignment = HorizontalAlignment.Right; Grid.SetColumn(control, 1); row.Children.Add(control);
        AutomationProperties.SetName(control, label);
        if (control is ToggleSwitch toggle)
        {
            toggle.OnContent = ""; toggle.OffContent = ""; toggle.MinWidth = 0;
            // Retain native keyboard/accessibility behavior and high-contrast colors.
            if (!new Windows.UI.ViewManagement.AccessibilitySettings().HighContrast)
            {
                foreach (var suffix in new[] { "", "PointerOver", "Pressed" })
                {
                    toggle.Resources["ToggleSwitchFillOn" + suffix] = design.Brush(suffix == "" ? "clay" : "clay-strong");
                    toggle.Resources["ToggleSwitchFillOff" + suffix] = design.Brush("border-strong");
                    toggle.Resources["ToggleSwitchStrokeOff" + suffix] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
                    toggle.Resources["ToggleSwitchKnobFillOff" + suffix] = new SolidColorBrush(Microsoft.UI.Colors.White);
                    toggle.Resources["ToggleSwitchKnobFillOn" + suffix] = new SolidColorBrush(Microsoft.UI.Colors.White);
                }
            }
        }
        return row;
    }
    private void BuildComposerOptions()
    {
        var body = new StackPanel { Width = 320, Spacing = 2 };
        var presenter = new Style(typeof(FlyoutPresenter));
        presenter.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        presenter.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        presenter.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(16)));
        presenter.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(20, 12, 20, 12)));
        composerOptions.FlyoutPresenterStyle = optionSubmenu.FlyoutPresenterStyle = presenter;
        foreach (var field in new Control[] { completionChoice, failureThreshold, failureMinimum })
        {
            field.CornerRadius = new CornerRadius(8); field.BorderThickness = new Thickness(1);
            field.Background = design.Brush("bg-elev"); field.BorderBrush = design.Brush("border-strong");
        }
        body.Children.Add(planRow = OptionRow("先计划", planMode));
        body.Children.Add(OptionRow("完全权限", fullPermission));
        body.Children.Add(OptionRow("子代理委派", delegation));
        completionChoice.Items.Add("当前轮内返回"); completionChoice.Items.Add("后台返回");
        body.Children.Add(OptionRow("结果返回方式", completionChoice));
        body.Children.Add(resumeRow = OptionRow("自动续接", autoResume));
        body.Children.Add(OptionRow("自动审查", autoReview));
        body.Children.Add(OptionRow("自动分析工具失败", failureAnalysis));
        body.Children.Add(thresholdRow = OptionRow("失败率阈值 (%)", failureThreshold, true));
        body.Children.Add(minimumRow = OptionRow("最少失败次数", failureMinimum, true));
        body.Children.Add(OptionRow("审查模型", reviewerChoice));
        body.Children.Add(OptionRow("记忆", memory));
        body.Children.Add(new Border { Height = 1, Background = design.Brush("border"), Margin = new Thickness(0, 8, 0, 8) });
        body.Children.Add(specialistRow = OptionRow("专家", specialistChoice));
        body.Children.Add(OptionRow("计算环境", computeChoice));
        foreach (var action in new[] { newAcp, acpSettings })
        {
            action.HorizontalAlignment = HorizontalAlignment.Stretch;
            action.HorizontalContentAlignment = HorizontalAlignment.Left;
            body.Children.Add(action);
        }
        body.Children.Add(optionStatus); body.Children.Add(optionReload);
        composerOptions.Content = new ScrollViewer { Content = body, MaxHeight = 650,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        optionSubmenu.Opened += (_, _) => optionSubmenuOpen = true;
        optionSubmenu.Closed += (_, _) => { optionSubmenuOpen = false; submenuControls.Clear(); };
        optionReload.Click += async (_, _) => await model.Options.LoadAsync(lifetime.Token);
        planMode.Toggled += async (_, _) => { if (!updatingOptions) await model.SetPlanModeAsync(planMode.IsOn, lifetime.Token); };
        fullPermission.Toggled += async (_, _) =>
        {
            if (updatingOptions) return;
            if (fullPermission.IsOn)
            {
                updatingOptions = true; fullPermission.IsOn = false; updatingOptions = false;
                ShowFullPermissionConfirmation();
            }
            else await model.Options.SetToggleAsync("full_permission", false, token: lifetime.Token);
        };
        delegation.Toggled += async (_, _) => { if (!updatingOptions) await model.Options.SetToggleAsync("delegation", delegation.IsOn, token: lifetime.Token); };
        autoReview.Toggled += async (_, _) => { if (!updatingOptions) await model.Options.SetToggleAsync("auto_review", autoReview.IsOn, token: lifetime.Token); };
        completionChoice.SelectionChanged += async (_, _) =>
        {
            if (!updatingOptions) await model.Options.SetCompletionAsync(completionChoice.SelectedIndex == 1 ? "background" : "inline", model.Options.Session?.Completion.AutoResume == true, lifetime.Token);
        };
        autoResume.Toggled += async (_, _) => { if (!updatingOptions) await model.Options.SetCompletionAsync("background", autoResume.IsOn, lifetime.Token); };
        failureAnalysis.Toggled += async (_, _) => { if (!updatingOptions) await SaveFailureAsync(); };
        failureThreshold.ValueChanged += async (_, _) => { if (!updatingOptions) await SaveFailureAsync(); };
        failureMinimum.ValueChanged += async (_, _) => { if (!updatingOptions) await SaveFailureAsync(); };
        memory.Toggled += async (_, _) => { if (!updatingOptions) await model.Options.SetMemoryAsync(memory.IsOn, lifetime.Token); };
        reviewerChoice.Click += (_, _) => ShowChoices(reviewerChoice, model.Options.Reviewers, model.Options.Reviewer,
            id => model.Options.SetReviewerAsync(id, lifetime.Token));
        specialistChoice.Click += (_, _) => ShowChoices(specialistChoice, model.Options.Specialists, NativeComposerOptionsModel.S(model.Options.Session?.Specialist, "id"),
            id => model.Options.SetSpecialistAsync(id, lifetime.Token));
        computeChoice.Click += (_, _) => ShowComputeOptions();
    }
    private async Task SaveFailureAsync()
    {
        if (double.IsNaN(failureThreshold.Value) || double.IsNaN(failureMinimum.Value)) { RefreshComposerOptions(); return; }
        await model.Options.SetFailureAsync(failureAnalysis.IsOn, (int)failureThreshold.Value, (int)failureMinimum.Value, lifetime.Token);
        RefreshComposerOptions();
    }
    private void OptionSummary(Button button, string value)
    {
        if (button.Tag as string == value) return;
        button.Tag = value;
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        var label = design.Text(value, 14); label.Foreground = design.Brush("text-muted");
        label.MaxWidth = 158; label.TextTrimming = TextTrimming.CharacterEllipsis;
        row.Children.Add(label); row.Children.Add(design.Icon("chevron-right", 14));
        button.Content = row; button.Padding = new Thickness(0, 5, 0, 5); design.QuietButton(button);
        ToolTipService.SetToolTip(button, value);
    }
    private void RefreshComposerOptions()
    {
        if (planRow == null) return;
        var state = model.Options;
        state.Writable = model.CanAttach;
        updatingOptions = true;
        try
        {
            planRow.Visibility = model.Snapshot?.PlanMode == null ? Visibility.Collapsed : Visibility.Visible;
            planMode.IsOn = model.Snapshot?.PlanMode == true; planMode.IsEnabled = model.CanChangePlanMode && !state.Busy;
            fullPermission.IsOn = state.Session?.FullPermission == true;
            delegation.IsOn = state.Session?.Delegation == true;
            autoReview.IsOn = state.Session?.AutoReview == true;
            completionChoice.SelectedIndex = state.Session?.Completion.Policy == "background" ? 1 : 0;
            autoResume.IsOn = state.Session?.Completion.AutoResume == true;
            resumeRow!.Visibility = completionChoice.SelectedIndex == 1 ? Visibility.Visible : Visibility.Collapsed;
            failureAnalysis.IsOn = state.Failure?.Enabled == true;
            failureThreshold.Value = state.Failure?.FailureRateThreshold ?? 30; failureMinimum.Value = state.Failure?.MinimumFailures ?? 2;
            thresholdRow!.Visibility = minimumRow!.Visibility = failureAnalysis.IsOn ? Visibility.Visible : Visibility.Collapsed;
            memory.IsOn = state.MemoryEnabled;
            foreach (var control in new Control[] { fullPermission, delegation, autoReview, failureAnalysis, failureThreshold, failureMinimum, memory, reviewerChoice, computeChoice }) control.IsEnabled = state.CanEdit;
            completionChoice.IsEnabled = autoResume.IsEnabled = state.CanEdit && state.Session?.Delegation == true;
            specialistChoice.IsEnabled = state.CanEdit && state.Session?.SpecialistLocked == false
                && model.Snapshot is { Running: false, Items.Length: 0 };
            specialistRow!.Opacity = specialistChoice.IsEnabled ? 1 : 0.45;
            OptionSummary(reviewerChoice, state.Reviewers.FirstOrDefault(v => v.Id == state.Reviewer)?.Label ?? "模型不可用");
            OptionSummary(specialistChoice, NativeComposerOptionsModel.S(state.Session?.Specialist, "name") is { Length: > 0 } name ? name : "无");
            var defaultId = state.Contexts?.DefaultContext?.ContextId ?? state.GlobalDefaultContext;
            var defaultLabel = defaultId == "local" ? "本机" : state.Contexts?.Contexts.FirstOrDefault(c => c.Id == defaultId)?.Label;
            OptionSummary(computeChoice, defaultLabel ?? (state.Contexts?.EnabledIds.Length > 0 ? $"{state.Contexts.EnabledIds.Length} 个远程环境" : "本机"));
            optionStatus.Text = state.Error ?? (state.Busy ? "正在同步…" : "");
            optionStatus.Visibility = optionStatus.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            optionReload.Visibility = state.Error != null ? Visibility.Visible : Visibility.Collapsed;
            optionReload.IsEnabled = !state.Busy;
            foreach (var control in submenuControls) control.IsEnabled = state.CanEdit;
        }
        finally { updatingOptions = false; }
    }
    private void ShowOptionSubmenu(FrameworkElement anchor, StackPanel content)
    {
        optionSubmenu.Hide(); submenuControls.Clear();
        foreach (var control in content.Children.OfType<Control>()) submenuControls.Add(control);
        optionSubmenu.Content = new ScrollViewer { Content = content, MaxHeight = 440,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        optionSubmenu.ShowAt(anchor);
    }
    private void ShowChoices(Button anchor, ComposerOptionChoice[] choices, string selected, Func<string, Task<bool>> save)
    {
        var body = new StackPanel { MinWidth = 220, MaxWidth = 330, Spacing = 4 };
        foreach (var choice in choices)
        {
            var button = new Button { Content = choice.Label, HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Left };
            if (choice.Id == selected) button.Background = design.Brush("bg-elev");
            AutomationProperties.SetName(button, choice.Label + (choice.Id == selected ? "，已选择" : ""));
            button.Click += async (_, _) => { optionSubmenu.Hide(); await save(choice.Id); };
            body.Children.Add(button);
        }
        ShowOptionSubmenu(anchor, body);
    }
    private void ShowFullPermissionConfirmation()
    {
        var body = new StackPanel { Width = 300, Spacing = 12 };
        var text = design.Text("开启后，当前会话中的工具调用、危险命令和 ACP 权限请求都会自动批准，直到你手动关闭或重启应用。项目路径限制和明确禁用的工具规则仍然有效。当前等待中的操作也会获准执行。", 14);
        text.TextWrapping = TextWrapping.Wrap; body.Children.Add(text);
        var confirm = new Button { Content = "开启完全权限" };
        confirm.Click += async (_, _) => { optionSubmenu.Hide(); await model.Options.SetToggleAsync("full_permission", true, confirmed: true, token: lifetime.Token); };
        body.Children.Add(confirm);
        var cancel = new Button { Content = "取消" }; cancel.Click += (_, _) => optionSubmenu.Hide(); body.Children.Add(cancel);
        ShowOptionSubmenu(fullPermission, body);
    }
    private void ShowComputeOptions()
    {
        var state = model.Options;
        var body = new StackPanel { Width = 300, Spacing = 8 };
        body.Children.Add(design.Text("会话默认计算环境", 14));
        var choices = new[] { new ComposerOptionChoice("local", "本机") }.Concat((state.Contexts?.Contexts ?? [])
            .Where(c => c.Kind != "local").Select(c => new ComposerOptionChoice(c.Id, c.Label))).ToArray();
        foreach (var choice in choices)
        {
            var button = new Button { Content = choice.Label, HorizontalAlignment = HorizontalAlignment.Stretch };
            if (choice.Id == (state.Contexts?.DefaultContext?.ContextId ?? state.GlobalDefaultContext ?? "local")) button.Background = design.Brush("bg-elev");
            button.Click += async (_, _) => { optionSubmenu.Hide(); await state.SetDefaultContextAsync(choice.Id, lifetime.Token); };
            body.Children.Add(button);
        }
        body.Children.Add(design.Text("本会话可用的远程环境", 14));
        foreach (var context in state.Contexts?.Contexts.Where(c => c.Kind != "local") ?? [])
        {
            var toggle = new CheckBox { Content = context.Label, IsChecked = state.Contexts!.EnabledIds.Contains(context.Id) };
            toggle.Click += async (_, _) => { optionSubmenu.Hide(); await state.SetContextAsync(context.Id, toggle.IsChecked == true, lifetime.Token); };
            body.Children.Add(toggle);
        }
        if (openHosts != null)
        {
            var manage = new Button { Content = "管理计算环境" };
            manage.Click += (_, _) => { optionSubmenu.Hide(); composerOptions.Hide(); openHosts(); }; body.Children.Add(manage);
        }
        ShowOptionSubmenu(computeChoice, body);
    }
}
