using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Dictionary<string, string> acpAnswers = [];
    private readonly Button newAcp = new() { Content = "新建 ACP 对话", HorizontalAlignment = HorizontalAlignment.Left,
        Visibility = Visibility.Collapsed };
    private MenuFlyout? acpCreationMenu;
    private bool acpCreationOpen;
    private string? acpCreationFingerprint;
    private readonly Button acpSettings = new() { Content = "ACP 会话选项", Visibility = Visibility.Collapsed };
    private readonly Flyout acpSettingsFlyout = new();
    private readonly ScrollViewer acpSettingsScroll = new() { MinWidth = 240, MaxWidth = 320, MaxHeight = 320,
        HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private bool acpSettingsOpen;
    private readonly List<ComboBox> acpSelectors = [];
    private readonly List<Control> acpSettingControls = [];
    private ComboBox? acpModeSelector;
    private readonly Dictionary<string, ComboBox> acpConfigSelectors = [];
    private readonly Dictionary<string, ToggleSwitch> acpConfigToggles = [];
    private bool updatingAcpSettings;
    // Do not disable a ComboBox synchronously from SelectionChanged: WinUI is
    // still closing its child popup and may dismiss the containing Flyout.
    // The model's Busy guard serializes writes; the next snapshot reconciles values.
    private bool AcpControlsEnabled => !model.ShowingHistory && !model.UncertainSend && model.ConnectionError == null
        && model.Snapshot is { ReadOnly: false, Running: false };
    private string? acpSettingsFingerprint;
    private void SetupAcpSettings()
    {
        acpSettings.Flyout = acpSettingsFlyout;
        acpSettingsFlyout.Content = acpSettingsScroll;
        acpSettingsFlyout.Opened += (_, _) => acpSettingsOpen = true;
        acpSettingsFlyout.Closed += (_, _) => acpSettingsOpen = false;
        design.QuietButton(acpSettings); design.QuietButton(newAcp);
    }
    private void RefreshAcpSettings()
    {
        var state = model.Snapshot?.AcpState;
        foreach (var control in acpSettingControls) control.IsEnabled = AcpControlsEnabled;
        // Preserve the actual selectors while a response updates their values:
        // replacing a focused ComboBox inside an open Flyout breaks light-dismiss.
        var shape = System.Text.Json.JsonSerializer.SerializeToNode(state) as JsonObject;
        if (shape?["modes"] is JsonObject modes) modes.Remove("currentModeId");
        if (shape?["configOptions"] is JsonArray configs)
            foreach (var config in configs.OfType<JsonObject>()) config.Remove("currentValue");
        var fingerprint = shape?.ToJsonString() + $"/{model.ShowingHistory}";
        if (fingerprint == acpSettingsFingerprint) { SyncAcpValues(state); return; }
        acpSettingsFingerprint = fingerprint;
        foreach (var selector in acpSelectors) selector.IsDropDownOpen = false;
        acpSelectors.Clear();
        acpSettingControls.Clear();
        acpModeSelector = null; acpConfigSelectors.Clear(); acpConfigToggles.Clear();
        var rows = new StackPanel { Spacing = 10, HorizontalAlignment = HorizontalAlignment.Stretch };
        acpSettingsScroll.Content = rows;
        if (state != null && !model.ShowingHistory)
        {
            bool ownsState() => model.Snapshot?.SessionId == state.FrameId && model.CanChangeAcpSettings;
            ComboBox addSelect(string name, string? description, NativeAcpChoice[] choices, string? current, Func<string, Task> change)
            {
                var selector = new ComboBox { PlaceholderText = "选择…", HorizontalAlignment = HorizontalAlignment.Stretch,
                    IsEnabled = AcpControlsEnabled, Header = name };
                foreach (var choice in choices) selector.Items.Add(new ComboBoxItem { Content = choice.Label, Tag = choice.Id });
                selector.SelectedItem = selector.Items.Cast<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == current);
                if (!string.IsNullOrEmpty(description)) ToolTipService.SetToolTip(selector, description);
                selector.SelectionChanged += async (_, _) =>
                {
                    if (!updatingAcpSettings && ownsState() && selector.SelectedItem is ComboBoxItem item && item.Tag is string id) await change(id);
                };
                acpSelectors.Add(selector); acpSettingControls.Add(selector); rows.Children.Add(selector);
                return selector;
            }
            if (!state.HasModeConfig && state.ModeChoices.Length > 0)
                acpModeSelector = addSelect("模式", null, state.ModeChoices, state.CurrentMode, id => model.SetAcpModeAsync(id, lifetime.Token));
            foreach (var option in state.ConfigOptions ?? [])
            {
                var id = NativeAcpSessionState.Text(option["id"]);
                if (string.IsNullOrEmpty(id)) continue;
                var name = NativeAcpSessionState.Text(option["name"]) ?? id;
                var description = NativeAcpSessionState.Text(option["description"]);
                if (NativeAcpSessionState.Text(option["type"]) == "boolean")
                {
                    var toggle = new ToggleSwitch { Header = name, IsEnabled = AcpControlsEnabled,
                        IsOn = option["currentValue"] is JsonValue boolean && boolean.TryGetValue<bool>(out var enabled) && enabled };
                    if (!string.IsNullOrEmpty(description)) ToolTipService.SetToolTip(toggle, description);
                    toggle.Toggled += async (_, _) => { if (!updatingAcpSettings && ownsState()) await model.SetAcpConfigAsync(id, JsonValue.Create(toggle.IsOn)!, lifetime.Token); };
                    acpConfigToggles[id] = toggle;
                    acpSettingControls.Add(toggle); rows.Children.Add(toggle);
                }
                else if (NativeAcpSessionState.Text(option["type"]) == "select")
                    acpConfigSelectors[id] = addSelect(name, description, NativeAcpSessionState.Choices(option), NativeAcpSessionState.Text(option["currentValue"]),
                        value => model.SetAcpConfigAsync(id, JsonValue.Create(value)!, lifetime.Token));
            }
        }
        acpSettings.Visibility = rows.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        if (rows.Children.Count == 0) acpSettingsFlyout.Hide();
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
    private void RefreshAcpCreation()
    {
        newAcp.Visibility = createAcp != null && model.Agents.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        newAcp.IsEnabled = !model.Busy;
        var fingerprint = System.Text.Json.JsonSerializer.Serialize(model.Agents);
        if (fingerprint == acpCreationFingerprint) return;
        acpCreationFingerprint = fingerprint;
        acpCreationMenu?.Hide();
        var menu = acpCreationMenu = new MenuFlyout();
        menu.Opened += (_, _) => acpCreationOpen = true;
        menu.Closed += (_, _) => acpCreationOpen = false;
        foreach (var agent in model.Agents)
        {
            var choice = new MenuFlyoutItem { Text = agent.Label };
            choice.Click += async (_, _) => { if (createAcp != null) await createAcp(agent.Id); };
            menu.Items.Add(choice);
        }
        newAcp.Flyout = menu;
    }

    public bool HandleAcpEscape()
    {
        if (acpSelectors.LastOrDefault(selector => selector.IsDropDownOpen) is { } selector)
        { selector.IsDropDownOpen = false; return true; }
        if (acpCreationOpen) { acpCreationMenu?.Hide(); return true; }
        if (acpSettingsOpen) { acpSettingsFlyout.Hide(); return true; }
        return false;
    }
    private void RenderAcpPermissions()
    {
        foreach (var permission in model.Snapshot?.Acp?.Permissions ?? [])
        {
            var card = new StackPanel { Spacing = 8, Padding = new Thickness(16) };
            card.Children.Add(design.Text("ACP 需要确认 · " + permission.Title, 14));
            card.Children.Add(new TextBox { Text = permission.Preview, IsReadOnly = true, AcceptsReturn = true,
                TextWrapping = TextWrapping.Wrap, MaxHeight = 140 });
            var enabled = !model.Busy && model.ConnectionError == null && model.Snapshot is { ReadOnly: false };
            foreach (var option in permission.Options)
            {
                var choice = new Button { Content = option.Name, IsEnabled = enabled };
                ToolTipService.SetToolTip(choice, option.Kind);
                choice.Click += async (_, _) => await model.RespondAcpPermissionAsync(permission, option.Id, lifetime.Token);
                card.Children.Add(choice);
            }
            var cancel = new Button { Content = "取消此请求", IsEnabled = enabled };
            cancel.Click += async (_, _) => await model.RespondAcpPermissionAsync(permission, null, lifetime.Token);
            card.Children.Add(cancel);
            approvals.Children.Add(new Border { Child = card, BorderThickness = new Thickness(1), BorderBrush = design.Brush("clay"),
                Background = design.Brush("bg-elev"), CornerRadius = new CornerRadius(12) });
        }
    }

    private bool RenderAcpQuestion(StackPanel card, JsonObject question)
    {
        var request = question["request_id"]?.GetValue<string>();
        if (request == null) return false;
        if (model.Snapshot?.Acp?.QuestionIds.Contains(request) != true)
        { card.Children.Add(design.Text(question["status"]?.GetValue<string>() == "answered" ? "已回答" : "此请求已结束", 12)); return true; }
        var enabled = model.CanAnswerAcp(request);
        foreach (var option in question["options"]?.AsArray() ?? [])
        {
            var label = option?["label"]?.GetValue<string>() ?? "";
            var choice = new Button { Content = label, IsEnabled = enabled };
            choice.Click += async (_, _) => await model.AnswerAcpAsync(request, label, lifetime.Token);
            card.Children.Add(choice);
        }
        var answer = new TextBox { Text = acpAnswers.GetValueOrDefault(request, ""), PlaceholderText = "输入回答…",
            AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, IsEnabled = enabled };
        var submit = new Button { Content = "提交回答", IsEnabled = enabled && !string.IsNullOrWhiteSpace(answer.Text) };
        answer.TextChanged += (_, _) => { acpAnswers[request] = answer.Text; submit.IsEnabled = model.CanAnswerAcp(request) && !string.IsNullOrWhiteSpace(answer.Text); };
        submit.Click += async (_, _) => await model.AnswerAcpAsync(request, answer.Text, lifetime.Token);
        card.Children.Add(answer); card.Children.Add(submit); return true;
    }
}
