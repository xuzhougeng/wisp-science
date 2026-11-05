using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private NativeNetworkSettingsModel? networkSettings;
    private Action? refreshNetworkFeedback;
    private Button? mirrorConfigure;
    private Border? networkFrame;
    private void CloseNetworkMirrors(bool cancel = false)
    {
        if (networkSettings is not { Busy: false, MirrorsOpen: true } network) return;
        network.CloseMirrors(cancel); Render();
        mirrorConfigure?.Focus(FocusState.Programmatic);
    }
    private void ResizeNetworkCard()
    {
        if (networkFrame != null) networkFrame.Padding = (XamlRoot?.Size.Width ?? ActualWidth) <= 850
            ? new Thickness(16, 18, 16, 18) : new Thickness(24, 22, 24, 22);
    }
    private void Network()
    {
        if (networkSettings is not { Loaded: true } network) return;
        var card = new StackPanel();
        var frame = Design.Card(card); frame.Padding = new Thickness(24, 22, 24, 22); Form.Children.Add(frame);
        networkFrame = frame; ResizeNetworkCard();
        if (network.MirrorsOpen)
        {
            mirrorConfigure = null;
            var breadcrumb = new Grid { ColumnSpacing = 8 };
            breadcrumb.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); breadcrumb.ColumnDefinitions.Add(new());
            var back = Design.ToolButton(T("settings.back"), "chevron-left"); StyleBreadcrumbBack(back);
            back.Click += (_, _) => CloseNetworkMirrors(); breadcrumb.Children.Add(back);
            var trail = BreadcrumbTrail(T("settings.nav.network"), T("network.mirrors"), () => CloseNetworkMirrors(), 16);
            Grid.SetColumn(trail, 1); breadcrumb.Children.Add(trail); card.Children.Add(breadcrumb);
        }
        var feedbackText = Design.Text("", 12);
        var feedback = new Border { Child = feedbackText, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8),
            Padding = new Thickness(10, 8, 10, 8), Visibility = Visibility.Collapsed };
        AutomationProperties.SetLiveSetting(feedbackText, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        card.Children.Add(feedback);
        refreshNetworkFeedback = () =>
        {
            var error = network.Error;
            if (error == null && !network.SaveConfirmed) { feedback.Visibility = Visibility.Collapsed; return; }
            var tone = Design.Brush(error == null ? "ok" : "err");
            feedbackText.Text = error switch { "network.address_required" => T("network.address_required"), null => T("network.saved"), _ => error };
            feedbackText.Foreground = tone;
            feedback.Background = new SolidColorBrush(Mix(Design.Brush("bg-input").Color, tone.Color, .08));
            feedback.BorderBrush = new SolidColorBrush(Mix(Design.Brush("border-strong").Color, tone.Color, .35));
            feedback.Visibility = Visibility.Visible;
        };
        TextBlock Description(string key)
        {
            var text = Mute(T(key)); Design.BindTypography(text, 12); Design.BindLineHeight(text, 12, 1.6); return text;
        }
        void Heading(string key)
        {
            var title = Design.Text(T(key), 13); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            title.Margin = new Thickness(0, 0, 0, 6);
            card.Children.Add(title);
        }
        async Task Save(string scope) { if (await network.SaveAsync(scope) && !model.Closed) Render(); }
        void PlainButton(Button button)
        {
            button.Background = Design.Brush("bg-input"); button.BorderBrush = Design.Brush("border-strong");
        }
        if (network.MirrorsOpen)
        {
            Heading("network.mirrors"); card.Children.Add(Description("network.mirrors_hint"));
            var fields = new StackPanel { Spacing = 26, Margin = new Thickness(0, 28, 0, 28) }; card.Children.Add(fields);
            foreach (var (key, label, placeholder) in new[] {
                ("conda_mirror_url", "network.conda", "https://mirror.example.com/anaconda/cloud/conda-forge"),
                ("pip_index_url", "network.pip", "https://mirror.example.com/pypi/simple"),
                ("ca_bundle_path", "network.ca", T("network.ca_placeholder")) })
            {
                var field = new StackPanel { Spacing = 10 }; fields.Children.Add(field); field.Children.Add(Design.Text(T(label), 14));
                var input = new TextBox { Text = NativeNetworkSettingsModel.Value(network.Draft, key),
                    PlaceholderText = placeholder, Padding = new Thickness(10, 8, 10, 8), CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-input") };
                AutomationProperties.SetName(input, T(label));
                Design.BindTypography(input); PreferenceFieldAppearance.UsePlainTextField(input); textMenus.Register(input);
                input.TextChanged += (_, _) => network.SetMirror(key, input.Text); field.Children.Add(input);
                if (key == "ca_bundle_path") field.Children.Add(Description("network.ca_hint"));
            }
            card.Children.Add(Description("network.credentials_hint"));
            var actions = new NativeActionWrap { Gap = 10, HorizontalAlignment = HorizontalAlignment.Right };
            var cancel = Button(T("settings.cancel"), () => { CloseNetworkMirrors(cancel: true); return Task.CompletedTask; });
            PlainButton(cancel);
            var save = Button(T("settings.save"), () => Save("mirrors"), primary: true);
            foreach (var action in new[] { cancel, save }) { action.MinWidth = 84; action.Padding = new Thickness(16, 9, 16, 9); actions.Children.Add(action); }
            card.Children.Add(new Border { Child = actions, Margin = new Thickness(0, 12, 0, 0), Padding = new Thickness(0, 14, 0, 0),
                BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) }); return;
        }
        Heading("network.connection");
        var connectionHint = Description("network.connection_hint"); connectionHint.Margin = new Thickness(0, 0, 0, 12); card.Children.Add(connectionHint);
        foreach (var scope in NativeNetworkSettingsModel.ProxyScopes)
        {
            var row = new Grid { ColumnSpacing = 20, Padding = new Thickness(0, 16, 0, 16) };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(.8, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = new GridLength(1.6, GridUnitType.Star) });
            row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var title = new NativeActionWrap { VerticalAlignment = VerticalAlignment.Center, VerticallyCentered = true };
            var label = Design.Text(T("network." + scope), 13); label.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            title.Children.Add(label);
            title.Children.Add(new Border { Child = Design.Text(T("network." + network.Mode(scope, saved: true)), 11, "text-muted"), Background = Design.Brush("bg-sunken"),
                Padding = new Thickness(6, 2, 6, 2), CornerRadius = new CornerRadius(4), HorizontalAlignment = HorizontalAlignment.Left });
            row.Children.Add(title);
            var mode = new ComboBox { MinWidth = 0, MaxWidth = 140, MinHeight = 36, Padding = new Thickness(11, 6, 0, 6),
                CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-input") };
            foreach (var value in new[] { "system", "direct", "custom" }) mode.Items.Add(new ComboBoxItem { Content = T("network." + value), Tag = value });
            mode.SelectedItem = mode.Items.Cast<ComboBoxItem>().First(i => (string)i.Tag == network.Mode(scope));
            AutomationProperties.SetName(mode, T("network." + scope)); Design.BindTypography(mode);
            PreferencePickerAppearance.SizeToOptions(mode, Design); choices.Add(mode);
            var address = new TextBox { MinWidth = 0, MinHeight = 36, Padding = new Thickness(11, 6, 11, 6),
                PlaceholderText = "http://127.0.0.1:7890", CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-input") };
            AutomationProperties.SetName(address, T("network." + scope) + " — " + T("network.address")); Design.BindTypography(address);
            PreferenceFieldAppearance.UsePlainTextField(address); textMenus.Register(address);
            void ShowAddress()
            {
                var custom = network.Mode(scope) == "custom";
                address.Visibility = custom ? Visibility.Visible : Visibility.Collapsed;
                var value = custom ? NativeNetworkSettingsModel.Value(network.Draft, scope + "_proxy_url") : "";
                if (address.Text != value) address.Text = value;
            }
            ShowAddress();
            address.TextChanged += (_, _) =>
            {
                if (address.Visibility == Visibility.Visible && address.Text != NativeNetworkSettingsModel.Value(network.Draft, scope + "_proxy_url"))
                    network.SetAddress(scope, address.Text);
            };
            mode.SelectionChanged += (_, _) =>
            {
                if (mode.SelectedItem is ComboBoxItem item) { network.SetMode(scope, (string)item.Tag); ShowAddress(); }
            };
            var clear = Button(T("network.clear"), () =>
            {
                if (mode.SelectedIndex != 0) mode.SelectedIndex = 0;
                else network.SetMode(scope, "system");
                return Task.CompletedTask;
            });
            Design.QuietButton(clear); clear.Foreground = Design.Brush("text-muted");
            // Set the intent once. Inferring a primary style from the Chinese
            // label and then replacing its resources can fail during rendering.
            var save = Button(T("settings.save"), () => Save(scope), primary: false);
            PlainButton(save);
            foreach (var action in new[] { clear, save })
            {
                action.MinWidth = action.MinHeight = 0;
                action.Padding = new Thickness(12, 7, 12, 7);
                Design.BindTypography(action, size: 12);
                if (action.Content is TextBlock caption) Design.BindTypography(caption, 12);
            }
            var controls = new NativeNetworkProxyControls(mode, address, clear, save);
            row.Children.Add(controls); Grid.SetColumn(controls, 1);
            var hint = Description("network." + scope + "_hint"); row.Children.Add(hint); Grid.SetColumnSpan(hint, 2); Grid.SetRow(hint, 2);
            row.SizeChanged += (_, _) =>
            {
                var narrow = (XamlRoot?.Size.Width ?? ActualWidth) <= 850;
                // Desktop has two occupied rows; the controls get their own
                // row only on compact windows, matching network-proxy's grid.
                row.RowSpacing = narrow ? 10 : 0;
                Grid.SetRow(hint, narrow ? 2 : 1);
                hint.Margin = narrow ? new Thickness(0) : new Thickness(0, 8, 0, 0);
                row.ColumnDefinitions[1].Width = narrow ? new GridLength(0) : new GridLength(1.6, GridUnitType.Star);
                Grid.SetColumn(controls, narrow ? 0 : 1); Grid.SetRow(controls, narrow ? 1 : 0);
                Grid.SetColumnSpan(controls, narrow ? 2 : 1); Grid.SetColumnSpan(title, narrow ? 2 : 1);
            };
            card.Children.Add(new Border { Child = row, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
        }
        var summary = new Grid { ColumnSpacing = 16, Margin = new Thickness(0, 18, 0, 0) };
        summary.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); summary.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var copy = new StackPanel { Spacing = 6 };
        var mirrorHeading = Design.Text(T("network.mirrors"), 13); mirrorHeading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; copy.Children.Add(mirrorHeading);
        copy.Children.Add(Description(NativeNetworkSettingsModel.MirrorKeys.Any(key => NativeNetworkSettingsModel.Value(network.Saved, key).Length > 0) ? "network.mirrors_configured" : "network.mirrors_empty"));
        summary.Children.Add(copy);
        var configure = Button(T("network.configure"), () => { network.OpenMirrors(); Render(); BodyScroll.ChangeView(null, 0, null, true); return Task.CompletedTask; });
        PlainButton(configure); configure.Padding = new Thickness(16, 9, 16, 9);
        mirrorConfigure = configure;
        Grid.SetColumn(configure, 1); summary.Children.Add(configure); card.Children.Add(summary);
    }
}
