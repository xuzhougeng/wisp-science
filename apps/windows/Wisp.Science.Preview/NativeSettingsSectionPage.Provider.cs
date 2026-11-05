using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private void AddProvider(string url = "https://api.deepseek.com") => ModelEditor(T("models.add"), NativeProviderDrafts.Create(url), "save_provider", d =>
    {
        var rows = new StackPanel { Spacing = 10 };
        var keyHint = ModelHint(T("models.separate_key_hint")); PasswordBox key = null!;
        Action refreshHeaders = () => { }; Action refreshProxy = () => { };
        var rowChoices = new List<ComboBox>();
        void KeyHint()
        {
            var stored = NativeProviderDrafts.StoredKeyProfile(d, Rows(model.Values["list_models"]));
            keyHint.Visibility = stored == null ? Visibility.Collapsed : Visibility.Visible;
            var host = Uri.TryCreate(S(d, "api_url"), UriKind.Absolute, out var endpoint) ? endpoint.Host : S(d, "api_url");
            if (key != null) key.PlaceholderText = stored == null ? "" : Design.Localization.Format("models.reuse_key", ("host", host));
        }
        void RenderRows()
        {
            rows.Children.Clear(); foreach (var choice in rowChoices) choices.Remove(choice); rowChoices.Clear();
            var start = choices.Count;
            foreach (var entry in NativeProviderDrafts.Entries(d)) rows.Children.Add(ProviderRow(d, entry, RenderRows));
            rowChoices.AddRange(choices.Skip(start));
        }
        var intro = ModelHint(T("models.byok_hint")); intro.Margin = new Thickness(0, 12, 0, 12); Form.Children.Add(intro);
        Field(T("settings.api_url"), S(d, "api_url"), value =>
        {
            if (NativeProviderDrafts.ChangeUrl(d, value)) RenderRows();
            KeyHint(); refreshHeaders(); refreshProxy();
        });
        Form.Children.Add(ModelHint(T("settings.tip")));
        Secure(d, "key", T("settings.api_key")); key = (PasswordBox)Form.Children.Last(); Form.Children.Add(keyHint); KeyHint();
        // Like the WebView's Add API access and preset buttons, focus only once
        // on entry. Editing the URL later must not move the caret out of it.
        if (NativeProviderDrafts.NeedsKey(d, Rows(model.Values["list_models"])))
        {
            void FocusKey(object sender, RoutedEventArgs args)
            {
                key.Loaded -= FocusKey;
                key.DispatcherQueue.TryEnqueue(() => {
                    if (key.IsLoaded && ReferenceEquals(model.Draft, d)) key.Focus(FocusState.Programmatic);
                });
            }
            key.Loaded += FocusKey;
        }
        var models = new StackPanel { Spacing = 10, Margin = new Thickness(0, 16, 0, 0) };
        var expert = ModelHint(T("settings.expert_hint"));
        // Match the standalone explanatory paragraph before the Models heading.
        expert.Margin = new Thickness(0, Design.FontSize(12), 0, Design.FontSize(10));
        models.Children.Add(expert);
        var heading = new StackPanel { Spacing = 2 }; var title = Design.Text(T("models.entries"), 13); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        heading.Children.Add(title); heading.Children.Add(ModelHint(T("models.entries_hint"))); models.Children.Add(heading);
        models.Children.Add(rows); RenderRows();
        var add = ModelButton(T("models.add_entry"), () =>
        { (d["entries"] as JsonArray)!.Add(NativeProviderDrafts.Entry()); RenderRows(); return Task.CompletedTask; });
        StyleModelButton(add); add.HorizontalAlignment = HorizontalAlignment.Left;
        var addContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 }; addContent.Children.Add(Design.Icon("plus", 14)); addContent.Children.Add(Design.Text(T("models.add_entry"), 13)); add.Content = addContent;
        models.Children.Add(add); Form.Children.Add(models);
        Form.Children.Add(ModelProxyRoute(d, out refreshProxy));
        Form.Children.Add(ModelRequestHeaders(d, out refreshHeaders));
    }, test: true);

    private Border ProviderRow(JsonObject draft, JsonObject entry, Action rebuild)
    {
        var content = new StackPanel { Spacing = 8 };
        var fields = new Grid { ColumnSpacing = 10, RowSpacing = 8 };
        fields.ColumnDefinitions.Add(new() { Width = new GridLength(.85, GridUnitType.Star), MinWidth = 140 });
        fields.ColumnDefinitions.Add(new() { Width = new GridLength(1.15, GridUnitType.Star) });
        fields.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); fields.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        for (var index = 0; index < 5; index++) fields.RowDefinitions.Add(new() { Height = GridLength.Auto });
        Action refreshRoles = () => { };
        var protocol = ModelFields(() => Choice(Form, T("settings.provider"), S(entry, "provider"),
            [("openai", T("settings.provider.openai")), ("openai_responses", T("settings.provider.openai_responses")), ("anthropic", T("settings.provider.anthropic"))], value => entry["provider"] = value));
        var modelId = ModelFields(() =>
        {
            var input = Field(T("settings.model"), S(entry, "model"), value => { NativeProviderDrafts.SelectModel(entry, value); refreshRoles(); });
            input.PlaceholderText = T("settings.model_ph");
        });
        var label = ModelFields(() => { var input = Field(T("settings.label"), S(entry, "label"), value => entry["label"] = value); input.PlaceholderText = T("settings.label_ph"); });
        var suffix = ModelFields(() => { var input = Field(T("settings.endpoint_suffix"), S(entry, "endpoint_suffix"), value => entry["endpoint_suffix"] = value); input.PlaceholderText = T("settings.endpoint_suffix_ph"); });
        var remove = Design.ToolButton(T("models.remove_entry"), "close"); remove.MinWidth = 24; remove.MinHeight = 24; remove.Padding = new Thickness(4);
        remove.VerticalAlignment = VerticalAlignment.Bottom; remove.Margin = new Thickness(0, 0, 0, 2);
        remove.IsEnabled = NativeProviderDrafts.Entries(draft).Count() > 1;
        remove.Click += (_, _) =>
        {
            if (Busy || NativeProviderDrafts.Entries(draft).Count() < 2) return;
            (draft["entries"] as JsonArray)!.Remove(entry); rebuild();
        };
        fields.Children.Add(protocol); fields.Children.Add(modelId); fields.Children.Add(label); fields.Children.Add(suffix); fields.Children.Add(remove);
        void Arrange()
        {
            var wide = NativeSettingsLayout.ModelFormColumns(XamlRoot?.Size.Width ?? ActualWidth);
            fields.ColumnDefinitions[0].MinWidth = wide ? 140 : 0;
            fields.RowSpacing = wide ? 0 : 8;
            suffix.Margin = wide ? new Thickness(0, 8, 0, 0) : new Thickness(0);
            for (var index = 0; index < fields.Children.Count; index++)
            {
                var child = (FrameworkElement)fields.Children[index];
                Grid.SetColumn(child, wide ? index < 3 ? index : index == 3 ? 0 : 3 : 0);
                Grid.SetRow(child, wide ? index == 3 ? 1 : 0 : index);
                Grid.SetColumnSpan(child, wide ? index == 3 ? 3 : 1 : 4);
            }
            remove.HorizontalAlignment = wide ? HorizontalAlignment.Stretch : HorizontalAlignment.Left;
        }
        fields.SizeChanged += (_, _) => Arrange(); fields.Loaded += (_, _) => Arrange(); content.Children.Add(fields);
        var roles = new NativeActionWrap { Gap = 14, RowGap = 8 }; content.Children.Add(roles);
        var checks = new Dictionary<string, CheckBox>(); var syncing = false;
        foreach (var (key, title) in new[] { ("use_for_expert", "settings.use_for_expert"), ("supports_vision", "settings.supports_vision"),
            ("use_for_vision", "settings.use_for_vision"), ("use_for_image_generation", "settings.use_for_image_generation"), ("use_for_video_generation", "settings.use_for_video_generation") })
        {
            var check = ModelCheck(T(title), B(entry, key), value => { if (!syncing) { NativeProviderDrafts.SetRole(entry, key, value); refreshRoles(); } });
            checks[key] = check; roles.Children.Add(check);
        }
        refreshRoles = () =>
        {
            var image = NativeModelPresentation.IsImage(entry); var video = !image && NativeModelPresentation.IsVideo(entry);
            syncing = true;
            foreach (var (key, check) in checks)
            {
                check.IsChecked = B(entry, key);
                check.Visibility = !image && !video || image && key == "use_for_image_generation" || video && key == "use_for_video_generation" ? Visibility.Visible : Visibility.Collapsed;
            }
            syncing = false;
        }; refreshRoles();
        // Added/removed entries rebuild outside Render(), so decorate each row
        // here as well as during the initial settings-page traversal.
        PreferenceFieldAppearance.ApplyTree(content, Design);
        return new Border { Child = content, Padding = new Thickness(12, 10, 12, 10), CornerRadius = new CornerRadius(10), Background = Design.Brush("bg-input"),
            BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1) };
    }

    private UIElement ModelRequestHeaders(JsonObject draft, out Action refresh)
    {
        var cards = new StackPanel { Spacing = 14, Visibility = Visibility.Collapsed };
        var intro = ModelHint(T("models.request_headers_hint")); intro.Margin = new Thickness(0, 4, 0, 2); cards.Children.Add(intro);
        var syncing = false; var update = new List<Action>();
        foreach (var session in new[] { false, true })
        {
            var toggleKey = session ? "send_session_id" : "send_user_agent"; var valueKey = session ? "session_header_name" : "user_agent";
            bool Enabled() => session ? NativeModelDrafts.SessionIdentityEnabled(draft) : B(draft, toggleKey);
            TextBox input = null!; TextBlock preview = null!; CheckBox check = null!;
            Action updateRow = () => { };
            var card = ModelFields(() =>
            {
                check = ModelCheck(T(session ? "models.send_session_id" : "models.send_user_agent"), Enabled(), value =>
                { if (!syncing) { draft[toggleKey] = value; updateRow(); } }); Form.Children.Add(check);
                Form.Children.Add(ModelHint(T(session ? "models.session_identity_purpose" : "models.user_agent_purpose")));
                input = Field(T(session ? "models.session_header_name" : "models.user_agent_value"), S(draft, valueKey), value => { draft[valueKey] = value; updateRow(); });
                input.PlaceholderText = session ? "x-opencode-session" : "wisp-science";
                Form.Children.Add(ModelHint(T(session ? "models.session_identity_hint" : "models.user_agent_hint")));
                var sample = new StackPanel { Spacing = 4 }; sample.Children.Add(ModelHint(T("models.request_header_preview")));
                preview = Design.Text("", 12); preview.FontFamily = Design.Font(true); preview.Foreground = Design.Brush("text-muted"); sample.Children.Add(preview);
                Form.Children.Add(new Border { Child = sample, Padding = new Thickness(12, 10, 12, 10), Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(8), Margin = new Thickness(0, 4, 0, 0) });
            }); card.Spacing = 8;
            updateRow = () =>
            {
                syncing = true; var enabled = Enabled(); check.IsChecked = enabled; input.IsEnabled = enabled; syncing = false;
                var value = S(draft, valueKey).Trim(); if (value.Length == 0) value = session ? "x-opencode-session" : "wisp-science";
                preview.Text = !enabled ? T("models.request_header_disabled") : session ? value + ": " + T("models.session_id_generated") : "User-Agent: " + value;
            }; update.Add(updateRow); updateRow();
            cards.Children.Add(new Border { Child = card, Padding = new Thickness(16), Background = Design.Brush("bg-elev"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8) });
        }
        var panel = new StackPanel { Spacing = 4 }; var summary = ModelButton(T("models.advanced_options"), () => Task.CompletedTask);
        Design.QuietButton(summary); summary.MinHeight = 0; summary.Padding = new Thickness(0, 8, 0, 8); summary.HorizontalAlignment = HorizontalAlignment.Left;
        void UpdateSummary()
        {
            var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            content.Children.Add(Design.Icon(cards.Visibility == Visibility.Visible ? "chevron-down" : "chevron-right", 14)); content.Children.Add(Design.Text(T("models.advanced_options"), 13)); summary.Content = content;
        }
        summary.Click += (_, _) => { cards.Visibility = cards.Visibility == Visibility.Visible ? Visibility.Collapsed : Visibility.Visible; UpdateSummary(); };
        UpdateSummary(); panel.Children.Add(summary); panel.Children.Add(cards);
        refresh = () => { foreach (var action in update) action(); }; return panel;
    }
}
