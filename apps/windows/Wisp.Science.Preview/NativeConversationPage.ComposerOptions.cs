using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly ToggleSwitch fullPermission = new(), autoReview = new();
    private readonly Button reviewerChoice = new(), specialistChoice = new(), computeChoice = new();
    private readonly TextBlock optionStatus = new() { TextWrapping = TextWrapping.Wrap, MaxWidth = 300, FontSize = 12 };
    private readonly Button optionReload = new() { Content = "重新读取" };
    private readonly Flyout optionSubmenu = new() { Placement = FlyoutPlacementMode.RightEdgeAlignedTop };
    private bool optionSubmenuOpen, updatingOptions;
    private FrameworkElement? planRow, specialistRow;
    private readonly List<Control> submenuControls = [];
    private readonly List<(TextBlock Text, string Key, bool Group)> optionLabels = [];
    private readonly Dictionary<Control, string> optionLabelKeys = [];
    private readonly Dictionary<Button, TextBlock> optionValues = [];
    private readonly Action? openAgentSettings;
    private (string? Project, string? Session) optionsOwner;
    private Control? optionSubmenuAnchor;
    private bool restoreOptionAnchor, restoreComposerOptionsTrigger;
    private Popup? permissionDialog;

    private async Task<bool> PrepareSessionOptionAsync(bool loadOptions = true)
    {
        if (!model.HasProjectDraft) return model.CanAttach;
        var project = model.CurrentProjectId;
        if (!await EnsureSessionForActionAsync() || disposed || model.HasProjectDraft || model.CurrentProjectId != project) return false;
        if (!loadOptions) return model.CanAttach;
        var session = model.CurrentSessionId;
        await model.Options.LoadAsync(lifetime.Token);
        return !disposed && model.CurrentProjectId == project && model.CurrentSessionId == session && model.Options.CanEdit;
    }

    private async Task<bool> SaveSessionOptionAsync(Func<Task<bool>> save)
        => await PrepareSessionOptionAsync() && await save();

    public bool HandleComposerOptionsEscape()
    {
        if (computeDefault?.IsDropDownOpen == true) { computeDefault.IsDropDownOpen = false; computeDefault.Focus(FocusState.Programmatic); return true; }
        if (optionSubmenuOpen)
        {
            restoreOptionAnchor = true; optionSubmenu.Hide();
            return true;
        }
        if (composerOptionsOpen) { restoreComposerOptionsTrigger = true; composerOptions.Hide(); return true; }
        return false;
    }

    private FrameworkElement OptionRow(string labelKey, Control control, bool indent = false)
    {
        var row = new Grid { MinHeight = 28, ColumnSpacing = 10 };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var text = design.Text(design.T(labelKey), 14); text.VerticalAlignment = VerticalAlignment.Center;
        text.MaxLines = 1; text.TextTrimming = TextTrimming.CharacterEllipsis;
        optionLabels.Add((text, labelKey, false)); optionLabelKeys[control] = labelKey;
        row.Children.Add(text);
        if (control is Button button)
        {
            row.MinHeight = 0; // A 40px indented row / 38px footer includes its own padding.
            row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var value = design.Text("", 14); value.Foreground = design.Brush("text-faint"); value.MaxWidth = 138;
            value.MaxLines = 1; value.TextTrimming = TextTrimming.CharacterEllipsis; value.VerticalAlignment = VerticalAlignment.Center;
            optionValues[button] = value; Grid.SetColumn(value, 1); row.Children.Add(value);
            var chevron = design.Icon("chevron-right", 15); Grid.SetColumn(chevron, 2); row.Children.Add(chevron);
            button.Content = row; design.QuietButton(button); button.MinHeight = indent ? 40 : 44;
            button.Padding = new(indent ? 28 : 12, 8, 12, 8); button.HorizontalAlignment = HorizontalAlignment.Stretch;
            button.HorizontalContentAlignment = HorizontalAlignment.Stretch;
            if (indent) text.Foreground = design.Brush("text-muted");
            return button;
        }
        row.MinHeight = 44; row.Padding = new(12, 8, 12, 8);
        control.VerticalAlignment = VerticalAlignment.Center;
        control.HorizontalAlignment = HorizontalAlignment.Right; Grid.SetColumn(control, 1); row.Children.Add(control);
        AutomationProperties.SetName(control, design.T(labelKey));
        if (control is ToggleSwitch toggle)
        {
            toggle.OnContent = ""; toggle.OffContent = ""; toggle.MinWidth = 0;
            toggle.MinHeight = 0; toggle.Width = 36;
            if (!new Windows.UI.ViewManagement.AccessibilitySettings().HighContrast)
                PreferenceToggleSwitch.Apply(toggle, design);
            else PreferenceToggleSwitch.Apply(toggle);
            row.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            row.CornerRadius = new(8);
            row.PointerEntered += (_, _) => { if (toggle.IsEnabled) row.Background = design.Brush("bg-sunken"); };
            row.PointerExited += (_, _) => row.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            row.Tapped += (_, e) =>
            {
                if (!toggle.IsEnabled) return;
                for (var el = e.OriginalSource as DependencyObject; el != null && el != row; el = VisualTreeHelper.GetParent(el))
                    if (el == toggle) return; // The switch handles its own pointer and keyboard input.
                e.Handled = true; toggle.Focus(FocusState.Pointer); toggle.IsOn = !toggle.IsOn;
            };
            if (toggle == fullPermission) ToolTipService.SetToolTip(row, design.T("full_permission.confirm_body"));
        }
        return row;
    }
    private void BuildComposerOptions()
    {
        Unloaded += (_, _) => CloseFullPermissionConfirmation(false);
        // The shared .compose-menu max-width is 320px including padding and border.
        var body = new StackPanel();
        var presenter = new Style(typeof(FlyoutPresenter));
        presenter.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        presenter.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        presenter.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(16)));
        presenter.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(8)));
        optionSubmenu.FlyoutPresenterStyle = presenter;
        composerOptions.FlyoutPresenterStyle = presenter;
        // The confirmation lives in a separate XAML popup. Its pointer events
        // must not light-dismiss the options menu underneath it.
        composerOptions.Closing += (_, args) => { if (permissionDialog != null) args.Cancel = true; };
        // Set the content width: the native template adds its own padding and
        // border around it. Recalculate on each open after a window resize.
        composerOptions.Opening += (_, _) => body.Width = Math.Max(0, NativeComposerMenu.Width(XamlRoot?.Size.Width ?? 352) - 18);
        var controls = new Dictionary<string, Control> { ["plan"] = planMode, ["full_permission"] = fullPermission,
            ["specialist"] = specialistChoice, ["auto_review"] = autoReview, ["reviewer"] = reviewerChoice, ["compute"] = computeChoice };
        foreach (var group in NativeComposerMenu.Groups)
        {
            var heading = design.Text("", 11); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            heading.Foreground = design.Brush("text-faint"); heading.CharacterSpacing = 50;
            heading.Margin = new(12, body.Children.Count == 0 ? 4 : 10, 12, 3);
            optionLabels.Add((heading, group.LabelKey, true)); body.Children.Add(heading);
            foreach (var item in group.Items)
            {
                var row = OptionRow(item.LabelKey, controls[item.Id], item.Indent);
                AutomationProperties.SetAutomationId(row, "agent-menu-" + item.Id);
                body.Children.Add(row);
                if (item.Id == "plan") planRow = row;
                if (item.Id == "specialist") specialistRow = row;
            }
        }
        body.Children.Add(new Border { Height = 1, Background = design.Brush("border"), Margin = new(12, 5, 12, 5) });
        var more = new Button { Visibility = openAgentSettings != null ? Visibility.Visible : Visibility.Collapsed };
        body.Children.Add(OptionRow("composer.more_settings", more));
        more.MinHeight = 38; more.Foreground = design.Brush("text-muted");
        more.Click += (_, _) => { optionSubmenu.Hide(); composerOptions.Hide(); openAgentSettings?.Invoke(); };
        body.Children.Add(optionStatus); body.Children.Add(optionReload);
        composerOptions.Content = new ScrollViewer { Content = body, MaxHeight = 650, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        optionSubmenu.Opened += (_, _) => optionSubmenuOpen = true;
        optionSubmenu.Closed += (_, _) =>
        {
            optionSubmenuOpen = false; submenuControls.Clear(); ClearComputeMenu();
            var anchor = restoreOptionAnchor ? optionSubmenuAnchor : null; restoreOptionAnchor = false;
            if (anchor != null) DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low,
                () => { if (!disposed && composerOptionsOpen && anchor.IsLoaded) anchor.Focus(FocusState.Keyboard); });
        };
        optionReload.Click += async (_, _) => await model.Options.LoadAsync(lifetime.Token);
        planMode.Toggled += async (_, _) =>
        {
            if (updatingOptions) return;
            var enabled = planMode.IsOn;
            if (model.HasProjectDraft && !enabled || !await PrepareSessionOptionAsync(loadOptions: false)) { RefreshComposerOptions(); return; }
            if (model.Snapshot?.PlanMode != null) await model.SetPlanModeAsync(enabled, lifetime.Token);
            else if (NativeComposerMenu.PlanPair(model.Snapshot?.AcpState) is { } pair)
                await model.SetAcpModeAsync(enabled ? pair.Plan : pair.Exit, lifetime.Token);
        };
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
        autoReview.Toggled += async (_, _) =>
        {
            if (updatingOptions) return;
            var enabled = autoReview.IsOn;
            if (model.HasProjectDraft && !enabled) { RefreshComposerOptions(); return; }
            await SaveSessionOptionAsync(() => model.Options.SetToggleAsync("auto_review", enabled, token: lifetime.Token));
        };
        reviewerChoice.Click += (_, _) => ShowChoices(reviewerChoice, model.Options.Reviewers.Select(choice => choice with { Label = NativeComposerMenu.ReviewerLabel(choice, design.Localization) }).ToArray(), model.Options.Reviewer,
            id => model.Options.SetReviewerAsync(id, lifetime.Token));
        specialistChoice.Click += (_, _) => ShowChoices(specialistChoice, model.Options.Specialists.Select(choice => choice with { Label = NativeComposerMenu.SpecialistLabel(choice, design.Localization) }).ToArray(), NativeComposerOptionsModel.S(model.Options.Session?.Specialist, "id"),
            id => model.Options.SetSpecialistAsync(id, lifetime.Token));
        computeChoice.Click += (_, _) => ShowComputeOptions();
    }
    private void OptionSummary(Button button, string value)
    {
        optionValues[button].Text = value;
        AutomationProperties.SetName(button, design.T(optionLabelKeys[button]) + ": " + value);
        ToolTipService.SetToolTip(button, value);
    }
    private void RefreshComposerOptions()
    {
        if (planRow == null) return;
        AutomationProperties.SetName(composerSettingsButton, design.T("composer.agent_options"));
        ToolTipService.SetToolTip(composerSettingsButton, design.T("composer.agent_options"));
        var owner = (model.CurrentProjectId, model.CurrentSessionId);
        if (owner != optionsOwner) { optionsOwner = owner; CloseFullPermissionConfirmation(false); optionSubmenu.Hide(); composerOptions.Hide(); }
        foreach (var (text, key, group) in optionLabels)
            text.Text = group ? design.T(key).ToUpperInvariant() : design.T(key);
        foreach (var (control, key) in optionLabelKeys) AutomationProperties.SetName(control, design.T(key));
        optionReload.Content = design.L("重新读取", "Reload");
        var state = model.Options;
        state.Writable = model.CanAttach || model.CanPrepareProjectDraft;
        updatingOptions = true;
        try
        {
            var acpPlan = NativeComposerMenu.PlanPair(model.Snapshot?.AcpState);
            planRow.Visibility = !AcpSelected && (model.HasProjectDraft || model.Snapshot?.PlanMode != null) || acpPlan != null ? Visibility.Visible : Visibility.Collapsed;
            planMode.IsOn = model.Snapshot?.PlanMode ?? NativeComposerMenu.IsPlan(model.Snapshot?.AcpState?.CurrentMode ?? "");
            planMode.IsEnabled = (model.HasProjectDraft ? model.CanPrepareProjectDraft : model.Snapshot?.PlanMode != null ? model.CanChangePlanMode : model.CanChangeAcpSettings) && !state.Busy;
            fullPermission.IsOn = state.Session?.FullPermission == true;
            autoReview.IsOn = state.Session?.AutoReview == true;
            foreach (var control in new Control[] { fullPermission, autoReview, reviewerChoice }) control.IsEnabled = state.CanEdit;
            computeChoice.IsEnabled = state.Contexts != null && !state.Busy;
            specialistChoice.IsEnabled = state.CanEdit && (model.HasProjectDraft || state.Session?.SpecialistLocked == false
                && model.Snapshot is { Running: false, Items.Length: 0 });
            specialistRow!.Opacity = specialistChoice.IsEnabled ? 1 : 0.45;
            OptionSummary(reviewerChoice, state.Reviewers.FirstOrDefault(v => v.Id == state.Reviewer) is { } reviewer
                ? NativeComposerMenu.ReviewerLabel(reviewer, design.Localization) : design.L("模型不可用", "Model unavailable"));
            OptionSummary(specialistChoice, NativeComposerOptionsModel.S(state.Session?.Specialist, "name") is { Length: > 0 } name ? name : design.T("composer.specialist.none"));
            if (state.Session?.SpecialistLocked == true) ToolTipService.SetToolTip(specialistChoice, design.T("composer.specialist.locked"));
            var defaultId = NativeComputeMenu.DefaultId(state.Contexts?.DefaultContext?.ContextId, state.GlobalDefaultContext);
            var defaultLabel = state.Contexts?.Contexts.FirstOrDefault(c => c.Id == defaultId)?.Label;
            OptionSummary(computeChoice, NativeComposerMenu.ComputeSummary(defaultId, defaultLabel, state.Contexts?.EnabledIds.Length ?? 0, design.Localization));
            optionStatus.Text = state.ErrorText(design.Localization) ?? (state.Busy ? design.L("正在同步…", "Syncing…") : "");
            optionStatus.Visibility = optionStatus.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            optionReload.Visibility = state.Error != null ? Visibility.Visible : Visibility.Collapsed;
            optionReload.IsEnabled = !state.Busy;
            foreach (var control in submenuControls) control.IsEnabled = state.CanEdit;
            RefreshComputeMenu();
        }
        finally { updatingOptions = false; }
    }
    private void ShowOptionSubmenu(FrameworkElement anchor, StackPanel content)
    {
        optionSubmenu.Hide(); submenuControls.Clear();
        optionSubmenuAnchor = anchor as Control;
        optionSubmenu.Placement = FlyoutPlacementMode.RightEdgeAlignedTop;
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
            design.QuietButton(button); design.BindTypography(button);
            var content = new Grid { ColumnSpacing = 10 };
            content.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var caption = design.Text(choice.Label, 14); content.Children.Add(caption);
            if (choice.Id == selected) { var check = design.Icon("check", 16); Grid.SetColumn(check, 1); content.Children.Add(check); }
            button.Content = content; button.HorizontalContentAlignment = HorizontalAlignment.Stretch;
            AutomationProperties.SetName(button, choice.Label + (choice.Id == selected ? design.L("，已选择", ", selected") : ""));
            var owner = optionsOwner;
            button.Click += async (_, _) => { optionSubmenu.Hide(); if (owner == (model.CurrentProjectId, model.CurrentSessionId)) await save(choice.Id); };
            body.Children.Add(button);
        }
        ShowOptionSubmenu(anchor, body);
    }
    private void ShowFullPermissionConfirmation()
    {
        if (permissionDialog != null || XamlRoot is not { } root || disposed) return;
        var owner = optionsOwner;
        var overlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light, UseLayoutRounding = false };
        NativeDialogVisuals.Backdrop(overlay, design);
        var body = new Grid { Padding = new(22), RowSpacing = 14 };
        body.RowDefinitions.Add(new() { Height = GridLength.Auto });
        body.RowDefinitions.Add(new() { Height = new(1, GridUnitType.Star) });
        body.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var title = design.Text(design.T("full_permission.confirm_title"), 17);
        title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; body.Children.Add(title);
        var hint = design.Text(design.T("full_permission.confirm_body"), 13);
        design.BindLineHeight(hint, 13, 1.55); hint.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        hint.Foreground = design.Brush("text-muted");
        var scroll = new ScrollViewer { Content = hint, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        Grid.SetRow(scroll, 1); body.Children.Add(scroll);
        var actions = new NativeActionWrap { RightAligned = true, Gap = 10, RowGap = 8, Margin = new(0, 4, 0, 0) };
        Button Action(string key, bool danger)
        {
            var label = design.Text(design.T(key), 13.5); var button = new Button { Content = label };
            design.ActionButton(button, destructive: danger); button.CornerRadius = new(10); button.Padding = new(16, 9, 16, 9);
            if (!danger)
            {
                button.Background = design.Brush("bg-input"); button.BorderBrush = design.Brush("border-strong");
            }
            actions.Children.Add(button); return button;
        }
        var cancel = Action("settings.cancel", false);
        var confirm = Action("full_permission.confirm_action", true);
        Grid.SetRow(actions, 2); body.Children.Add(actions);
        var card = new Border { Child = body, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"),
            BorderThickness = new(1), CornerRadius = new(16), Shadow = new ThemeShadow(), Translation = new(0, 0, 24),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle };
        AutomationProperties.SetName(card, title.Text); AutomationProperties.SetAutomationId(card, "composer-full-permission-confirm");
        overlay.Children.Add(card);
        void Fit(XamlRoot sender, XamlRootChangedEventArgs? args)
        {
            overlay.Width = sender.Size.Width; overlay.Height = sender.Size.Height;
            var bounds = ProjectDialogLayout.ForViewport(overlay.Width, overlay.Height, 480);
            card.Width = bounds.Width; card.MaxHeight = bounds.MaxHeight;
        }
        var popup = permissionDialog = new Popup { XamlRoot = root, Child = overlay, IsLightDismissEnabled = false };
        cancel.Click += (_, _) => CloseFullPermissionConfirmation();
        confirm.Click += async (_, _) =>
        {
            if (permissionDialog != popup || disposed || owner != (model.CurrentProjectId, model.CurrentSessionId) || !model.Options.CanEdit) return;
            CloseFullPermissionConfirmation();
            await SaveSessionOptionAsync(() => model.Options.SetToggleAsync("full_permission", true, confirmed: true, token: lifetime.Token));
        };
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, e) => e.Handled = HandleFullPermissionEscape();
        overlay.KeyboardAccelerators.Add(escape); overlay.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        popup.Closed += (_, _) => root.Changed -= Fit;
        root.Changed += Fit; Fit(root, null); popup.IsOpen = true; cancel.Focus(FocusState.Programmatic);
    }
    public bool HandleFullPermissionEscape()
    {
        if (permissionDialog == null) return false;
        CloseFullPermissionConfirmation(); return true;
    }
    private void CloseFullPermissionConfirmation(bool restoreFocus = true)
    {
        if (permissionDialog is not { } popup) return;
        permissionDialog = null; popup.IsOpen = false; popup.Child = null;
        if (restoreFocus && !disposed && fullPermission.IsLoaded) fullPermission.Focus(FocusState.Programmatic);
    }
}
