using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private NativeDecisionSettingsModel? decisionSettings;
    private TextBlock? decisionFeedback;
    private async Task LoadDecisionSettings()
    {
        if (decisionSettings == null || Busy) return;
        await decisionSettings.LoadAsync();
        if (!model.Closed && showDecision) Render();
    }
    private void DecisionSettings()
    {
        if (decisionSettings is not { } state) return;
        Form.Spacing = 12;
        var contentStart = Form.Children.Count;
        Form.Children.Add(ModelHint(T("decision.desc")));
        if (state.Draft is not { } draft || state.Saved is not { } saved)
        {
            decisionFeedback = ModelHint(L("正在加载…", "Loading…")); Form.Children.Add(decisionFeedback);
            var retry = ModelButton(L("重新加载", "Reload"), LoadDecisionSettings); StyleModelButton(retry);
            retry.HorizontalAlignment = HorizontalAlignment.Left; Form.Children.Add(retry); UpdateDecisionFeedback(); return;
        }
        var status = Design.Text(Design.Localization.Format(state.SavedStatusKey,
            ("preset", T("decision.preset." + S(saved, "preset"))),
            ("model", S(saved, "model").Length > 0 ? S(saved, "model") : S(saved, "base_url"))), 13);
        status.Foreground = Design.Brush(state.SavedUsable ? "text" : "text-muted");
        Form.Children.Add(new Border { Child = status, Padding = new Thickness(12, 10, 12, 10), CornerRadius = new CornerRadius(8),
            Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush(state.SavedUsable ? "clay-soft" : "border"), BorderThickness = new Thickness(1) });
        var enabled = new ToggleSwitch { IsOn = B(draft, "enabled"), OnContent = "", OffContent = "", MinWidth = 0,
            HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center };
        StylePreferenceToggle(enabled);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(enabled, T("decision.enabled"));
        enabled.Toggled += (_, _) => state.Set("enabled", System.Text.Json.Nodes.JsonValue.Create(enabled.IsOn));
        var enableRow = new Grid { ColumnSpacing = 12 }; enableRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        enableRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var enableLabel = Design.Text(T("decision.enabled"), 13); enableLabel.VerticalAlignment = VerticalAlignment.Center;
        enableRow.Children.Add(enableLabel); Grid.SetColumn(enabled, 1); enableRow.Children.Add(enabled); Form.Children.Add(enableRow);

        var fieldsStart = Form.Children.Count;
        var service = ModelFields(() => Choice(Form, T("decision.preset"), S(draft, "preset"),
            state.Presets.Select(p => (S(p, "id"), T("decision.preset." + S(p, "id")))).ToArray(), value =>
            { if (state.SelectPreset(value)) Render(); }));
        var preset = state.SelectedPreset;
        var hint = new NativeActionWrap { Gap = 10, RowGap = 4 };
        hint.Children.Add(ModelHint(T("decision.preset_hint." + S(draft, "preset"))));
        if (S(preset, "key_url") is { Length: > 0 } keyUrl)
        {
            var link = ModelButton(T("decision.link." + S(draft, "preset")), async () => { await Windows.System.Launcher.LaunchUriAsync(new Uri(keyUrl)); });
            Design.QuietButton(link); link.Padding = new Thickness(0); link.MinHeight = 0;
            var copy = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
            var label = Design.Text(T("decision.link." + S(draft, "preset")), 12); label.Foreground = Design.Brush("clay");
            copy.Children.Add(label); copy.Children.Add(Design.Icon("external-link", 12, Design.ColorText("clay"))); link.Content = copy; hint.Children.Add(link);
        }
        service.Spacing = 5; service.Children.Add(hint); Form.Children.Add(service);
        Form.Children.Add(ModelFields(() =>
        {
            var url = Field(T("decision.base_url"), S(draft, "base_url"), value => state.Set("base_url", System.Text.Json.Nodes.JsonValue.Create(value)));
            url.PlaceholderText = S(preset, "base_url") is { Length: > 0 } defaultUrl ? defaultUrl : "https://decide.example.org";
        }));
        var fields = new Grid { ColumnSpacing = 14 };
        fields.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); fields.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        for (var i = 0; i < 3; i++) fields.RowDefinitions.Add(new() { Height = GridLength.Auto });
        fields.Children.Add(ModelFields(() =>
        {
            var input = Field(T("decision.model"), S(draft, "model"), value => state.Set("model", System.Text.Json.Nodes.JsonValue.Create(value)));
            input.PlaceholderText = T("decision.model_placeholder");
        }));
        fields.Children.Add(ModelFields(() => Choice(Form, T("decision.think"), S(draft, "think_mode"),
            state.ThinkModes.Select(mode => (mode, T("decision.think." + (mode.Length == 0 ? "default" : mode)))).ToArray(),
            value => state.Set("think_mode", System.Text.Json.Nodes.JsonValue.Create(value)))));
        fields.Children.Add(ModelFields(() => Field(T("decision.timeout"), state.TimeoutText, state.SetTimeout)));
        void Arrange()
        {
            var wide = NativeSettingsLayout.ModelFormColumns(XamlRoot?.Size.Width ?? ActualWidth);
            fields.RowSpacing = 0;
            for (var i = 0; i < fields.Children.Count; i++)
            {
                var child = (FrameworkElement)fields.Children[i]; var row = wide ? i / 2 : i;
                Grid.SetRow(child, row); Grid.SetColumn(child, wide ? i % 2 : 0); Grid.SetColumnSpan(child, wide ? 1 : 2);
                child.Margin = new Thickness(0, row > 0 ? 12 : 0, 0, 0);
            }
        }
        fields.Loaded += (_, _) => Arrange(); fields.SizeChanged += (_, _) => Arrange(); Form.Children.Add(fields);
        Form.Children.Add(ModelHint(T("decision.think_hint")));
        var keyHead = new Grid { ColumnSpacing = 12 };
        keyHead.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); keyHead.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var stored = NativeDecisionSettingsModel.HasKey(draft);
        keyHead.Children.Add(ModelHint(T("decision.api_key") + " — " + T(stored ? "cred.stored" : B(preset, "key_required") ? "cred.not_stored" : "decision.key_optional")));
        if (stored)
        {
            var clear = ModelButton(T("cred.clear"), async () => { if (await state.ClearKeyAsync()) Render(); });
            Design.QuietButton(clear); clear.Padding = new Thickness(0); clear.MinHeight = 0; Grid.SetColumn(clear, 1); keyHead.Children.Add(clear);
        }
        var keyGroup = new StackPanel { Spacing = 5 }; keyGroup.Children.Add(keyHead);
        var key = new PasswordBox { Password = state.ApiKey, PlaceholderText = stored ? T("settings.stored_key") : "" };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(key, T("decision.api_key")); key.PasswordChanged += (_, _) => state.SetKey(key.Password);
        var styledKey = ModelFields(() => Form.Children.Add(key)); keyGroup.Children.Add(styledKey); Form.Children.Add(keyGroup);
        // The web form grid uses twelve-pixel row gaps inside a fourteen-pixel
        // section grid. Keep those boundaries instead of flattening every row.
        var fieldGroup = new StackPanel { Spacing = 12 };
        while (Form.Children.Count > fieldsStart) { var child = Form.Children[fieldsStart]; Form.Children.RemoveAt(fieldsStart); fieldGroup.Children.Add(child); }
        Form.Children.Add(fieldGroup);
        Form.Children.Add(ModelHint(T("decision.egress")));
        decisionFeedback = ModelHint(""); Microsoft.UI.Xaml.Automation.AutomationProperties.SetLiveSetting(decisionFeedback, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        Form.Children.Add(decisionFeedback);
        var actions = new NativeActionWrap { RightAligned = true, Gap = 10 };
        var test = ModelButton(T("decision.test"), async () => { await state.TestAsync(); }); StyleModelButton(test); test.MinWidth = 84; actions.Children.Add(test);
        var save = ModelButton(T("settings.save"), async () => { if (await state.SaveAsync()) Render(); }); StyleModelButton(save, primary: true); save.MinWidth = 84; actions.Children.Add(save);
        Form.Children.Add(new Border { Child = actions, Padding = new Thickness(0, 14, 0, 0), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
        var content = new StackPanel { Spacing = 14 };
        while (Form.Children.Count > contentStart) { var child = Form.Children[contentStart]; Form.Children.RemoveAt(contentStart); content.Children.Add(child); }
        Form.Children.Add(content);
        UpdateDecisionFeedback();
    }
    private void UpdateDecisionFeedback()
    {
        if (decisionFeedback == null || decisionSettings is not { } state) return;
        var error = state.Error;
        var text = state.ValidationKey == "timeout" ? L("超时请填写 0–600 的整数；0 使用服务默认值。", "Enter a timeout from 0 to 600; 0 uses the service default.")
            : state.ValidationKey != null ? T(state.ValidationKey)
            : error != null ? error
            : state.MessageKey == "decision.test_ok" ? Design.Localization.Format("decision.test_ok", ("model", S(state.Probe, "model") is { Length: > 0 } name ? name : "?"), ("ms", state.Probe?["elapsed_ms"]?.ToString() ?? "0"))
            : state.MessageKey != null ? T(state.MessageKey)
            : state.Saved == null ? L("正在加载…", "Loading…") : "";
        decisionFeedback.Text = text; decisionFeedback.Visibility = text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        decisionFeedback.Foreground = Design.Brush(error != null || state.ValidationKey != null ? "err" : "text-muted");
    }
}
