using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Dictionary<(string Project, string Session, string Request), string> acpAnswers = [];
    private readonly Border acpSettingsSection = new() { Visibility = Visibility.Collapsed, BorderThickness = new(0, 1, 0, 0),
        Margin = new(0, 4, 0, 0), Padding = new(0, 4, 0, 0) };
    private readonly List<ComboBox> acpSelectors = [];
    private readonly List<Control> acpSettingControls = [];
    private ComboBox? acpModeSelector;
    private TextBlock? acpModeValue;
    private readonly Dictionary<string, ComboBox> acpConfigSelectors = [];
    private readonly Dictionary<string, ToggleSwitch> acpConfigToggles = [];
    private bool updatingAcpSettings;
    // Do not disable a ComboBox synchronously from SelectionChanged: WinUI is
    // still closing its child popup and may dismiss the containing Flyout.
    // The model's Busy guard serializes writes; the next snapshot reconciles values.
    private bool AcpControlsEnabled => !model.ShowingHistory && !model.UncertainSend && model.ConnectionError == null
        && model.Snapshot is { ReadOnly: false, Running: false };
    private string? acpSettingsFingerprint;
    private void RefreshAcpSettings()
    {
        acpSettingsSection.BorderBrush = design.Brush("border");
        var state = model.Snapshot?.AcpState;
        foreach (var control in acpSettingControls) control.IsEnabled = AcpControlsEnabled;
        // Preserve the actual selectors while a response updates their values:
        // replacing a focused ComboBox inside an open Flyout breaks light-dismiss.
        var shape = System.Text.Json.JsonSerializer.SerializeToNode(state) as JsonObject;
        if (shape?["modes"] is JsonObject modes) modes.Remove("currentModeId");
        if (shape?["configOptions"] is JsonArray configs)
            foreach (var config in configs.OfType<JsonObject>()) config.Remove("currentValue");
        var fingerprint = shape?.ToJsonString() + $"/{model.ShowingHistory}/{design.Localization.Locale}/{design.Typography}/{design.Dark}";
        if (fingerprint == acpSettingsFingerprint) { SyncAcpValues(state); return; }
        acpSettingsFingerprint = fingerprint;
        foreach (var selector in acpSelectors) selector.IsDropDownOpen = false;
        acpSelectors.Clear();
        acpSettingControls.Clear();
        acpModeSelector = null; acpModeValue = null; acpConfigSelectors.Clear(); acpConfigToggles.Clear();
        var rows = new StackPanel { HorizontalAlignment = HorizontalAlignment.Stretch };
        acpSettingsSection.Child = rows;
        var heading = design.Text(design.T("composer.session_options").ToUpperInvariant(), 11);
        heading.Foreground = design.Brush("text-faint"); heading.Margin = new(10, 4, 10, 2); rows.Children.Add(heading);
        if (state != null && !model.ShowingHistory)
        {
            var project = model.CurrentProjectId;
            bool ownsState() => model.CurrentProjectId == project && model.Snapshot?.SessionId == state.FrameId && model.CanChangeAcpSettings;
            void addRow(string name, FrameworkElement value, string? description)
            {
                var row = new Grid { ColumnSpacing = 10, MinHeight = 32, Padding = new(10, 4, 10, 4) };
                row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                var label = design.Text(name, 12); label.Foreground = design.Brush("text-muted");
                label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
                label.VerticalAlignment = VerticalAlignment.Center; row.Children.Add(label);
                value.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(value, 1); row.Children.Add(value);
                ToolTipService.SetToolTip(row, string.IsNullOrEmpty(description) ? name : name + "\n" + description);
                rows.Children.Add(row);
            }
            ComboBox addSelect(string name, string? description, NativeAcpChoice[] choices, string? current, Func<string, Task> change)
            {
                var selector = new ComboBox { PlaceholderText = design.L("选择…", "Choose…"), MinWidth = 0, MaxWidth = 150,
                    MinHeight = 28, Padding = new(6, 4, 6, 4), IsEnabled = AcpControlsEnabled,
                    Background = design.Brush("bg-input"), BorderBrush = design.Brush("border-strong"), CornerRadius = new(6) };
                design.BindTypography(selector, code: true, size: 12); AutomationProperties.SetName(selector, name);
                foreach (var choice in choices) selector.Items.Add(new ComboBoxItem { Content = choice.Label, Tag = choice.Id });
                selector.SelectedItem = selector.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == current);
                if (!string.IsNullOrEmpty(description)) ToolTipService.SetToolTip(selector, description);
                selector.SelectionChanged += async (_, _) =>
                {
                    if (!updatingAcpSettings && ownsState() && selector.SelectedItem is ComboBoxItem item && item.Tag is string id) await change(id);
                };
                acpSelectors.Add(selector); acpSettingControls.Add(selector); addRow(name, selector, description);
                return selector;
            }
            if (NativeComposerModelMenu.ShowMode(state))
            {
                if (state.ModeChoices.Length > 1)
                    acpModeSelector = addSelect(design.T("composer.group.mode"), null, state.ModeChoices, state.CurrentMode, id => model.SetAcpModeAsync(id, lifetime.Token));
                else
                {
                    var value = acpModeValue = design.Text(NativeComposerModelMenu.ModeLabel(state), 12); value.FontFamily = design.Font(true);
                    value.MaxWidth = 150; value.TextWrapping = TextWrapping.NoWrap; value.TextTrimming = TextTrimming.CharacterEllipsis;
                    addRow(design.T("composer.group.mode"), value, null);
                }
            }
            foreach (var option in state.ConfigOptions ?? [])
            {
                var id = NativeAcpSessionState.Text(option["id"]);
                if (string.IsNullOrEmpty(id)) continue;
                var name = NativeAcpSessionState.Text(option["name"]) ?? id;
                var description = NativeAcpSessionState.Text(option["description"]);
                if (NativeAcpSessionState.Text(option["type"]) == "boolean")
                {
                    var toggle = new ToggleSwitch { IsEnabled = AcpControlsEnabled, OnContent = "", OffContent = "", MinWidth = 0, MinHeight = 0, Height = 28,
                        IsOn = option["currentValue"] is JsonValue boolean && boolean.TryGetValue<bool>(out var enabled) && enabled };
                    AutomationProperties.SetName(toggle, name); PreferenceToggleSwitch.Apply(toggle, design);
                    if (!string.IsNullOrEmpty(description)) ToolTipService.SetToolTip(toggle, description);
                    toggle.Toggled += async (_, _) => { if (!updatingAcpSettings && ownsState()) await model.SetAcpConfigAsync(id, JsonValue.Create(toggle.IsOn)!, lifetime.Token); };
                    acpConfigToggles[id] = toggle;
                    acpSettingControls.Add(toggle); addRow(name, toggle, description);
                }
                else if (NativeAcpSessionState.Text(option["type"]) == "select")
                    acpConfigSelectors[id] = addSelect(name, description, NativeAcpSessionState.Choices(option), NativeAcpSessionState.Text(option["currentValue"]),
                        value => model.SetAcpConfigAsync(id, JsonValue.Create(value)!, lifetime.Token));
            }
        }
        acpSettingsSection.Visibility = rows.Children.Count > 1 ? Visibility.Visible : Visibility.Collapsed;
    }
    private void SyncAcpValues(NativeAcpSessionState? state)
    {
        if (state == null || model.Busy) return;
        updatingAcpSettings = true;
        try
        {
            static void select(ComboBox? selector, string? value)
            {
                if (selector != null && !selector.IsDropDownOpen)
                    selector.SelectedItem = selector.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == value);
            }
            select(acpModeSelector, state.CurrentMode);
            if (acpModeValue != null) acpModeValue.Text = NativeComposerModelMenu.ModeLabel(state);
            foreach (var option in state.ConfigOptions ?? [])
            {
                var id = NativeAcpSessionState.Text(option["id"]) ?? "";
                select(acpConfigSelectors.GetValueOrDefault(id), NativeAcpSessionState.Text(option["currentValue"]));
                if (acpConfigToggles.TryGetValue(id, out var toggle) && option["currentValue"] is JsonValue boolean
                    && boolean.TryGetValue<bool>(out var enabled)) toggle.IsOn = enabled;
            }
        }
        finally { updatingAcpSettings = false; }
    }
    public bool HandleAcpEscape()
    {
        if (acpSelectors.LastOrDefault(selector => selector.IsDropDownOpen) is { } selector)
        { selector.IsDropDownOpen = false; selector.Focus(FocusState.Programmatic); return true; }
        return false;
    }
    private void RenderAcpPermissions()
    {
        foreach (var permission in model.Snapshot?.Acp?.Permissions ?? [])
        {
            var card = new StackPanel { Spacing = 8, Padding = new Thickness(16) };
            var title = design.Text(permission.Title, 14); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            card.Children.Add(title);
            var caption = design.Text(design.L("ACP 权限确认", "ACP permission"), 11); caption.Foreground = design.Brush("text-faint");
            card.Children.Add(caption);
            card.Children.Add(new TextBox { AcceptsReturn = true, Text = permission.Preview, IsReadOnly = true,
                TextWrapping = TextWrapping.Wrap, MaxHeight = 140 });
            var enabled = !model.Busy && model.ConnectionError == null && model.Snapshot is { ReadOnly: false };
            var actions = new NativeActionWrap();
            foreach (var option in permission.Options)
            {
                var choice = new Button { Content = option.Name, IsEnabled = enabled };
                design.ActionButton(choice, option.Kind.StartsWith("allow", StringComparison.Ordinal));
                ToolTipService.SetToolTip(choice, option.Kind);
                choice.Click += async (_, _) => await model.RespondAcpPermissionAsync(permission, option.Id, lifetime.Token);
                actions.Children.Add(choice);
            }
            var cancel = new Button { Content = design.L("取消此请求", "Cancel request"), IsEnabled = enabled };
            design.ActionButton(cancel);
            cancel.Click += async (_, _) => await model.RespondAcpPermissionAsync(permission, null, lifetime.Token);
            actions.Children.Add(cancel); card.Children.Add(actions);
            approvals.Children.Add(new Border { Child = card, BorderThickness = new Thickness(1), BorderBrush = design.Brush("clay"),
                Background = design.Brush("bg-elev"), CornerRadius = new CornerRadius(12) });
        }
    }

    private bool RenderAcpQuestion(StackPanel card, JsonObject question)
    {
        var presentation = NativeAcpQuestion.Parse(question);
        if (presentation == null) return false;
        var request = presentation.RequestId;
        var project = model.CurrentProjectId; var session = model.CurrentSessionId;
        bool ownsRequest() => model.CurrentProjectId == project && model.CurrentSessionId == session && model.CanAnswerAcp(request);
        card.Padding = new(0); card.Spacing = 0;
        var head = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 9 };
        head.Children.Add(design.Icon("chat", 16));
        var title = design.Text(design.T("plan.question.title"), 11.5); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        head.Children.Add(title);
        card.Children.Add(new Border { Child = head, Padding = new(12, 10, 12, 10), Background = design.Brush("clay-soft"),
            BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) });
        var text = TranscriptView.RenderMarkdown(presentation.Text, design);
        card.Children.Add(new Border { Child = text, Padding = new(16, 12, 16, 4) });
        if (!presentation.Pending(model.Snapshot?.Acp?.QuestionIds))
        {
            var note = design.Text(design.T(presentation.NoteKey), 10.5); note.Foreground = design.Brush("text-faint");
            card.Children.Add(new Border { Child = note, Padding = new(12, 8, 12, 8), Margin = new(0, 8, 0, 0),
                Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new(0, 1, 0, 0) });
            return true;
        }
        var enabled = ownsRequest();
        var choices = new StackPanel { Spacing = 7, Padding = new(16, 10, 16, 10) };
        foreach (var option in presentation.Choices)
        {
            var content = new StackPanel { Spacing = 2 };
            var label = design.Text(option.Label, 12); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            content.Children.Add(label);
            if (option.Description.Length > 0)
            { var detail = design.Text(option.Description, 10.5); detail.Foreground = design.Brush("text-faint"); content.Children.Add(detail); }
            var choice = new Button { Content = content, IsEnabled = enabled, HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch };
            design.ActionButton(choice); choice.Padding = new(10, 8, 10, 8);
            AutomationProperties.SetName(choice, option.Label);
            AutomationProperties.SetHelpText(choice, option.Description);
            choice.Click += async (_, _) => { if (ownsRequest()) await model.AnswerAcpAsync(request, option.Label, lifetime.Token); };
            choices.Children.Add(choice);
        }
        if (choices.Children.Count > 0) card.Children.Add(choices);
        if (!presentation.AllowsFreeform) return true;
        var key = presentation.DraftKey(project ?? "", session ?? "");
        var answer = new TextBox { Text = acpAnswers.GetValueOrDefault(key, ""), PlaceholderText = design.T("plan.question.placeholder"),
            IsEnabled = enabled, MinWidth = 0, Padding = new(10, 6, 10, 6), Background = design.Brush("bg-sunken"),
            BorderBrush = design.Brush("border-strong"), CornerRadius = new(6), FontSize = design.FontSize(12), FontFamily = design.Font() };
        AutomationProperties.SetName(answer, design.T("plan.question.placeholder"));
        var submit = new Button { Content = design.T("plan.question.send"), IsEnabled = enabled && !string.IsNullOrWhiteSpace(answer.Text) };
        design.ActionButton(submit, true); submit.Padding = new(10, 6, 10, 6);
        answer.TextChanged += (_, _) => { acpAnswers[key] = answer.Text; submit.IsEnabled = ownsRequest() && !string.IsNullOrWhiteSpace(answer.Text); };
        async Task send() { if (ownsRequest()) await model.AnswerAcpAsync(request, answer.Text, lifetime.Token); }
        submit.Click += async (_, _) => await send();
        var composingAnswer = false;
        answer.TextCompositionStarted += (_, _) => composingAnswer = true;
        answer.TextCompositionEnded += (_, _) => { composingAnswer = true; answer.DispatcherQueue.TryEnqueue(() => composingAnswer = false); };
        answer.KeyDown += async (_, e) =>
        {
            if (e.Key != Windows.System.VirtualKey.Enter || composingAnswer
                || (Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Shift) & Windows.UI.Core.CoreVirtualKeyStates.Down) != 0) return;
            e.Handled = true; await send();
        };
        var freeform = new Grid { ColumnSpacing = 7, Padding = new(16, 10, 16, 12) };
        freeform.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); freeform.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        freeform.Children.Add(answer); Grid.SetColumn(submit, 1); freeform.Children.Add(submit);
        card.Children.Add(freeform); return true;
    }
}
