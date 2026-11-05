using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private NativeSubscriptionRoutesModel? subscriptionRoutes;
    private readonly Dictionary<string, Button> subscriptionRouteSaves = [];
    private readonly Dictionary<string, TextBlock> subscriptionRouteFeedback = [];
    private FrameworkElement ProxyRouteField(Func<string> value, Func<bool> custom, Action<string, bool> changed, string category, string hint, out Action refresh)
    {
        var panel = new StackPanel { Spacing = 8, Margin = new Thickness(0, 8, 0, 0) };
        var label = Design.Text(T("network.route"), 12); label.Foreground = Design.Brush("text-muted"); panel.Children.Add(label);
        var controls = new Grid { ColumnSpacing = 10, RowSpacing = 8 };
        controls.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); controls.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        controls.RowDefinitions.Add(new() { Height = GridLength.Auto }); controls.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var mode = new ComboBox { MinWidth = 0, MinHeight = 36, CornerRadius = new CornerRadius(8), Padding = new Thickness(10, 8, 0, 8), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong") };
        foreach (var id in new[] { "inherit", "direct", "custom" }) mode.Items.Add(new ComboBoxItem { Content = T("network." + id), Tag = id });
        choices.Add(mode); Design.BindTypography(mode, size: 13); controls.Children.Add(mode);
        var address = new TextBox { MinWidth = 0, MinHeight = 36, PlaceholderText = "http://127.0.0.1:7890", CornerRadius = new CornerRadius(8), Padding = new Thickness(10, 8, 10, 8), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong") };
        textMenus.Register(address); Design.BindTypography(address, size: 13); Grid.SetColumn(address, 1); controls.Children.Add(address); panel.Children.Add(controls);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(mode, T("network.route"));
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(address, T("network.route") + " — " + T("network.address"));
        panel.Children.Add(ModelHint(T(hint) + (category.Length == 0 ? "" : " " + Design.Localization.Format("network.inherit_hint", ("category", T(category))))));
        var syncing = false;
        void Update()
        {
            syncing = true; var current = NativeProxyRoutes.Mode(value(), custom());
            mode.SelectedItem = mode.Items.Cast<ComboBoxItem>().First(item => (string)item.Tag == current);
            address.Visibility = current == "custom" ? Visibility.Visible : Visibility.Collapsed;
            var text = value() == "none" ? "" : value(); if (address.Text != text) address.Text = text;
            syncing = false; Arrange();
        }
        mode.SelectionChanged += (_, _) => { if (!syncing && mode.SelectedItem is ComboBoxItem item) { var id = (string)item.Tag; changed(id == "direct" ? "none" : "", id == "custom"); Update(); LockConfirmation(); } };
        address.TextChanged += (_, _) => { if (!syncing) { changed(address.Text, true); Update(); LockConfirmation(); } };
        void Arrange()
        {
            var narrow = (XamlRoot?.Size.Width ?? ActualWidth) <= 850 || controls.ActualWidth < 440;
            controls.RowSpacing = narrow && address.Visibility == Visibility.Visible ? 8 : 0;
            Grid.SetRow(address, narrow ? 1 : 0); Grid.SetColumn(address, narrow ? 0 : 1); Grid.SetColumnSpan(address, narrow ? 2 : 1);
            Grid.SetColumnSpan(mode, narrow ? 2 : 1);
            mode.MinWidth = narrow ? 0 : Math.Min(200, controls.ActualWidth);
            mode.MaxWidth = narrow ? double.PositiveInfinity : 220; mode.HorizontalAlignment = narrow ? HorizontalAlignment.Stretch : HorizontalAlignment.Left;
        }
        controls.SizeChanged += (_, _) => Arrange(); controls.Loaded += (_, _) => Arrange();
        refresh = Update; Update(); return panel;
    }
    private FrameworkElement ModelProxyRoute(JsonObject draft, out Action refresh) => ProxyRouteField(
        () => NativeProxyRoutes.ModelValue(draft, Rows(model.Values["list_models"])), () => NativeProxyRoutes.Custom(draft),
        (value, custom) => NativeProxyRoutes.Set(draft, value, custom), "network.model", "models.proxy_hint", out refresh);

    private void SubscriptionProxyRoute(StackPanel card, string provider)
    {
        if (subscriptionRoutes is not { } state) return;
        var content = new StackPanel { Spacing = 8 };
        if (state.Saved != null && state.Error != null && state.StatusProvider == null) content.Children.Add(ModelHint(state.Error));
        if (state.Drafts.TryGetValue(provider, out var draft))
        {
            content.Children.Add(ProxyRouteField(() => S(draft, "proxy_url"), () => NativeProxyRoutes.Custom(draft),
                (value, custom) => state.Set(provider, value, custom), "", "subscriptions.proxy_hint", out _));
            var actions = new NativeActionWrap { RightAligned = true, Gap = 12 };
            var feedback = ModelHint(""); feedback.VerticalAlignment = VerticalAlignment.Center;
            subscriptionRouteFeedback[provider] = feedback; actions.Children.Add(feedback);
            var save = ModelButton(T("settings.save"), async () => { await state.SaveAsync(provider); if (!model.Closed) Render(); });
            StyleModelButton(save); subscriptionRouteSaves[provider] = save; actions.Children.Add(save); content.Children.Add(actions);
        }
        else
        {
            content.Children.Add(ModelHint(state.Error == null ? T("subscriptions.loading") : state.Error));
            if (state.Error != null)
            {
                var retry = ModelButton(L("重试", "Retry"), async () => { await state.LoadAsync(); if (!model.Closed) Render(); });
                StyleModelButton(retry); content.Children.Add(retry);
            }
        }
        card.Children.Add(new Border { Child = content, Padding = new Thickness(0, 14, 0, 0), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
    }
    private void UpdateSubscriptionRoutes()
    {
        if (subscriptionRoutes is not { } state) return;
        foreach (var (provider, save) in subscriptionRouteSaves)
            save.IsEnabled = !Busy && pendingConfirmation == null && state.ProviderHasChanges(provider);
        foreach (var (provider, feedback) in subscriptionRouteFeedback)
            feedback.Text = state.StatusProvider == provider ? state.Error is { } error ? T(error) : state.SaveConfirmed ? T("network.saved") : "" : "";
    }
}
