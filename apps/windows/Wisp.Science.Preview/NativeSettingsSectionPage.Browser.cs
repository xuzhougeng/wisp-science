using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private readonly Dictionary<string, (string Host, string Reason)> browserInputs = new() { ["block"] = ("", ""), ["prefer"] = ("", "") };
    private string? browserFeedback;

    private void Browser()
    {
        Body.Padding = new Thickness(24, 14, 24, 22); Form.Spacing = 12;
        var behavior = new StackPanel(); var behaviorCard = Design.Card(behavior); ResponsiveSettingsCard(behaviorCard); Form.Children.Add(behaviorCard);
        Behavior("get_browser_auto_launch", T("browser.auto_launch"), T("browser.auto_launch_hint"), "browser.auto_launch_saved");
        Behavior("get_browser_auto_close_tabs", T("browser.auto_close_tabs"), T("browser.auto_close_tabs_hint"), "browser.auto_close_tabs_saved");
        Form.Children.Add(ModelHint(T("browser.filters.hint")));
        if (browserFeedback != null) Form.Children.Add(Mute(browserFeedback));
        if (model.Values["get_browser_url_filters"] is not JsonObject filters)
        { Form.Children.Add(Mute(L("网址过滤尚未载入，请刷新重试。", "URL filters have not loaded. Refresh to try again."))); return; }

        var collections = new Grid { ColumnSpacing = 20, RowSpacing = 20 };
        collections.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); collections.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        collections.RowDefinitions.Add(new() { Height = GridLength.Auto }); collections.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var blocked = RuleCard("block", T("browser.filters.block"), T("browser.filters.reason_block_placeholder"), T("browser.filters.block_empty"));
        var preferred = RuleCard("prefer", T("browser.filters.prefer"), T("browser.filters.reason_prefer_placeholder"), T("browser.filters.prefer_empty"));
        collections.Children.Add(blocked); collections.Children.Add(preferred);
        void Arrange()
        {
            // The WebView breakpoint is based on the whole settings window, including navigation.
            var single = (XamlRoot?.Size.Width ?? ActualWidth) <= 1100;
            collections.RowSpacing = single ? 20 : 0;
            Grid.SetColumnSpan(blocked, single ? 2 : 1); Grid.SetColumnSpan(preferred, single ? 2 : 1);
            Grid.SetColumn(preferred, single ? 0 : 1); Grid.SetRow(preferred, single ? 1 : 0);
        }
        collections.SizeChanged += (_, _) => Arrange(); collections.Loaded += (_, _) => Arrange(); Form.Children.Add(collections);

        void Behavior(string read, string title, string hint, string savedKey)
        {
            var available = model.Values[read] is JsonValue value && value.TryGetValue<bool>(out _);
            var toggle = new ToggleSwitch { IsOn = available && model.Values[read]!.GetValue<bool>(), OnContent = null, OffContent = null, MinWidth = 0, IsEnabled = available };
            StylePreferenceToggle(toggle);
            toggle.Toggled += async (_, _) => { browserFeedback = null; if (await model.SetBrowserBehaviorAsync(read, toggle.IsOn)) browserFeedback = T(savedKey); if (!model.Closed) Render(); };
            InlineRow(behavior, title, hint, toggle, compact: true, emphasized: true, divider: read == "get_browser_auto_launch");
        }
        Border RuleCard(string kind, string title, string reasonPlaceholder, string emptyText)
        {
            var body = new StackPanel(); var heading = Design.Text(Design.Localization.Locale == "en" ? title.ToUpperInvariant() : title, 14);
            heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; heading.CharacterSpacing = 60; heading.Margin = new Thickness(2, 8, 2, 8); body.Children.Add(heading);
            var inputRow = new Grid { ColumnSpacing = 8, RowSpacing = 8, Margin = new Thickness(0, 0, 0, 12) };
            inputRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); inputRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); inputRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            inputRow.RowDefinitions.Add(new() { Height = GridLength.Auto }); inputRow.RowDefinitions.Add(new() { Height = GridLength.Auto }); inputRow.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var host = RuleInput(T("browser.filters.host_placeholder"), title + " — " + T("browser.filters.host"), browserInputs[kind].Host);
            var reason = RuleInput(reasonPlaceholder, title + " — " + T("browser.filters.reason"), browserInputs[kind].Reason);
            var add = Button(T("browser.filters.add"), async () =>
            {
                browserFeedback = null;
                if (await model.AddBrowserRuleAsync(kind, host.Text, reason.Text)) { browserInputs[kind] = ("", ""); browserFeedback = T("browser.filters.saved"); }
                if (!model.Closed) Render();
            });
            StyleModelButton(add); AutomationProperties.SetName(add, T("browser.filters.add") + " — " + title); add.IsEnabled = !string.IsNullOrWhiteSpace(host.Text);
            host.TextChanged += (_, _) => { browserInputs[kind] = (host.Text, reason.Text); add.IsEnabled = !string.IsNullOrWhiteSpace(host.Text); };
            reason.TextChanged += (_, _) => browserInputs[kind] = (host.Text, reason.Text);
            // Settings fields already own their border and focus ring. Applying
            // the dialog decorator first resolves the same resource keys twice
            // and can abort Render with 0x800F0902 on the first field.
            var hostField = host; var reasonField = reason;
            inputRow.Children.Add(hostField); Grid.SetColumn(reasonField, 1); inputRow.Children.Add(reasonField); Grid.SetColumn(add, 2); inputRow.Children.Add(add);
            inputRow.SizeChanged += (_, e) =>
            {
                var narrow = NativeSettingsLayout.BrowserFieldsStack(e.NewSize.Width);
                inputRow.RowSpacing = narrow ? 8 : 0;
                Grid.SetColumnSpan(hostField, narrow ? 3 : 1); Grid.SetColumn(reasonField, narrow ? 0 : 1); Grid.SetColumnSpan(reasonField, narrow ? 3 : 1); Grid.SetRow(reasonField, narrow ? 1 : 0);
                Grid.SetColumn(add, narrow ? 0 : 2); Grid.SetColumnSpan(add, narrow ? 3 : 1); Grid.SetRow(add, narrow ? 2 : 0);
            };
            body.Children.Add(inputRow);
            var rows = Rows(filters[kind]).ToArray();
            if (rows.Length == 0)
            {
                body.Children.Add(new Border { Child = Design.Text(emptyText, 13), BorderBrush = Design.Brush("border"),
                    BorderThickness = new Thickness(0, 1, 0, 0) });
            }
            foreach (var row in rows)
            {
                var line = new Grid { ColumnSpacing = 16, Padding = new Thickness(0, 10, 0, 0) };
                line.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                var copy = new StackPanel { Spacing = 4 }; copy.Children.Add(Design.Text(S(row, "host"), 14)); if (S(row, "reason").Length > 0) copy.Children.Add(Mute(S(row, "reason"))); line.Children.Add(copy);
                var remove = Design.ToolButton(T("browser.filters.remove") + " — " + title + ": " + S(row, "host"), "close");
                remove.Click += async (_, _) => { browserFeedback = null; if (await model.RemoveBrowserRuleAsync(kind, S(row, "host"))) browserFeedback = T("browser.filters.saved"); if (!model.Closed) Render(); };
                Grid.SetColumn(remove, 1); line.Children.Add(remove); body.Children.Add(new Border { Child = line, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
            }
            var card = Design.Card(body); ResponsiveSettingsCard(card); card.VerticalAlignment = VerticalAlignment.Top; return card;
        }
        TextBox RuleInput(string placeholder, string label, string value)
        {
            var input = new TextBox { PlaceholderText = placeholder, Text = value, MinWidth = 0, CornerRadius = new CornerRadius(10), Padding = new Thickness(10, 8, 10, 8), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
            AutomationProperties.SetName(input, label); textMenus.Register(input);
            PreferenceFieldAppearance.Apply(input, Design); return input;
        }
    }
}
