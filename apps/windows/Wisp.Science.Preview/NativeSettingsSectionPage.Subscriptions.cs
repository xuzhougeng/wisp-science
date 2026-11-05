using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private NativeSubscriptionSettingsModel? subscriptionSettings;
    private DispatcherTimer? subscriptionPoll;
    private TextBlock? subscriptionFeedback;
    private Button? subscriptionSave;
    private Expander? subscriptionManual;
    private async Task LoadSubscriptions()
    {
        if (subscriptionSettings == null || Busy) return;
        await subscriptionSettings.LoadAsync();
        if (model.Closed || !showSubscriptions) return;
        if (subscriptionRoutes != null) await subscriptionRoutes.LoadAsync();
        if (model.Closed || !showSubscriptions) return;
        Render();
        if (subscriptionSettings.Accounts.Count > 0) { await subscriptionSettings.RefreshUsageAsync(); if (!model.Closed && showSubscriptions && !subscriptionSettings.FormOpen) Render(); }
    }
    private async Task OpenSubscription(string provider, bool accountOnly)
    {
        if (subscriptionSettings == null || Busy) return;
        await subscriptionSettings.OpenAsync(provider, accountOnly);
        if (!model.Closed) { Render(); BodyScroll.ChangeView(null, 0, null, true); }
    }
    private async Task SubscriptionAccountAction(string command, string? id = null)
    {
        if (subscriptionSettings == null || Busy) return;
        if (await subscriptionSettings.AccountActionAsync(command, id))
        {
            await model.LoadAsync(Reads);
            if (!model.Closed) { Render(); await subscriptionSettings.RefreshUsageAsync(); }
        }
        if (!model.Closed) Render();
    }
    private void Subscriptions()
    {
        if (subscriptionSettings is not { } state) return;
        Form.Spacing = 20; Form.Children.Add(ModelHint(T("subscriptions.desc")));
        subscriptionFeedback = ModelHint(""); Form.Children.Add(subscriptionFeedback);
        foreach (var provider in NativeSubscriptionSettingsModel.Providers)
        {
            var card = new StackPanel { Spacing = 18 };
            var heading = new Grid { ColumnSpacing = 24, RowSpacing = 10 };
            heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            heading.RowDefinitions.Add(new() { Height = GridLength.Auto }); heading.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var copy = new StackPanel { Spacing = 10 }; var title = Design.Text(provider.Title, 16); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            copy.Children.Add(title); copy.Children.Add(ModelHint(T(provider.Description))); heading.Children.Add(copy);
            var status = ModelHint(T(state.StatusErrors.ContainsKey(provider.Id) ? "subscriptions.status_error" : !state.Statuses.ContainsKey(provider.Id) ? "subscriptions.loading" : state.SignedIn(provider.Id) ? "subscriptions.signed_in" : "subscriptions.signed_out"));
            status.MaxWidth = 220; status.HorizontalAlignment = HorizontalAlignment.Left; Grid.SetColumn(status, 1); heading.Children.Add(status);
            heading.SizeChanged += (_, _) => { var narrow = NativeSettingsLayout.SubscriptionHeadingStacks(XamlRoot?.Size.Width ?? 0); Grid.SetColumn(status, narrow ? 0 : 1); Grid.SetRow(status, narrow ? 1 : 0); Grid.SetColumnSpan(copy, narrow ? 2 : 1); };
            card.Children.Add(heading);
            var actions = new NativeActionWrap { Gap = 10, RowGap = 8, RightAligned = true };
            var signIn = ModelButton(T(!state.SignedIn(provider.Id) ? "codex.login.start" : provider.Id == "codex" ? "subscriptions.add_account" : "subscriptions.manage"), () => OpenSubscription(provider.Id, true));
            StyleModelButton(signIn); actions.Children.Add(signIn);
            if (provider.Id == "codex")
            {
                var import = ModelButton(T("subscriptions.import_local"), () => SubscriptionAccountAction("import_local_codex_accounts")); StyleModelButton(import);
                ToolTipService.SetToolTip(import, T("subscriptions.import_local_hint")); actions.Children.Add(import);
            }
            var add = ModelButton(T("subscriptions.add_model"), () => OpenSubscription(provider.Id, false)); StyleSubscriptionPrimary(add);
            add.IsEnabled = state.SignedIn(provider.Id); add.Opacity = add.IsEnabled ? 1 : .55; actions.Children.Add(add); card.Children.Add(actions);
            SubscriptionProxyRoute(card, provider.Id);
            if (provider.Id == "codex") SubscriptionAccounts(card);
            var rows = Rows(model.Values["list_models"]).Where(row => NativeModelDrafts.ProviderValue(S(row, "provider")) == provider.Profile).ToArray();
            if (rows.Length == 0) card.Children.Add(ModelHint(T("subscriptions.empty")));
            foreach (var row in rows)
            {
                var line = new Grid { ColumnSpacing = 12 }; line.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                var edit = ModelButton(T("models.edit") + ": " + S(row, "label"), () => { EditModel(row); return Task.CompletedTask; });
                Design.QuietButton(edit); edit.HorizontalContentAlignment = HorizontalAlignment.Left; edit.Padding = new Thickness(6, 8, 6, 8);
                var labels = new StackPanel { Spacing = 5 }; var name = Design.Text(S(row, "label"), 14); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
                labels.Children.Add(name); labels.Children.Add(ModelHint(S(row, "model"))); edit.Content = labels; line.Children.Add(edit);
                var rowActions = new NativeActionWrap { Gap = 8, VerticalAlignment = VerticalAlignment.Center };
                if (B(row, "use_for_expert")) rowActions.Children.Add(ModelHint(T("models.capability.expert")));
                if (B(row, "active")) rowActions.Children.Add(ModelHint(T("models.default")));
                else
                {
                    var use = ModelButton(T("models.use"), () => Run("set_active_model", new() { ["id"] = S(row, "id") })); StyleModelButton(use); rowActions.Children.Add(use);
                    if (Rows(model.Values["list_models"]).Count() > 1)
                    {
                        var remove = Design.ToolButton(T("models.remove"), "close"); remove.Click += (_, _) => Confirm(Design.Localization.Format("models.remove_confirm", ("model", S(row, "label"))), () => _ = Run("remove_model", new() { ["id"] = S(row, "id") })); rowActions.Children.Add(remove);
                    }
                }
                Grid.SetColumn(rowActions, 1); line.Children.Add(rowActions);
                card.Children.Add(new Border { Child = line, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(0, 12, 0, 0) });
            }
            Form.Children.Add(new Border { Child = card, Padding = new Thickness(24), CornerRadius = new CornerRadius(14), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), Background = Design.Brush("bg-elev") });
        }
        UpdateSubscriptionFeedback();
    }
    private void SubscriptionAccounts(StackPanel card)
    {
        if (subscriptionSettings is not { } state) return;
        if (state.AccountsError != null) card.Children.Add(ModelHint(state.AccountsError));
        if (state.Imported is { } imported) card.Children.Add(ModelHint(Design.Localization.Format("subscriptions.imported", ("n", imported.ToString()))));
        if (state.Accounts.Count == 0) return;
        var heading = new NativeActionWrap { Gap = 12 }; var label = Design.Text(Design.Localization.Format("subscriptions.accounts", ("n", state.Accounts.Count.ToString())), 13); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; heading.Children.Add(label);
        var refresh = ModelButton(T("subscriptions.usage_refresh"), async () => { await state.RefreshUsageAsync(); if (!model.Closed) Render(); }); StyleModelButton(refresh, 12); heading.Children.Add(refresh); card.Children.Add(heading);
        foreach (var account in state.Accounts.OfType<JsonObject>())
        {
            var id = S(account, "account_id"); var row = new StackPanel { Spacing = 6 };
            var line = new Grid { ColumnSpacing = 12, RowSpacing = 12 };
            line.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            line.RowDefinitions.Add(new() { Height = GridLength.Auto }); line.RowDefinitions.Add(new() { Height = GridLength.Auto }); line.Children.Add(row);
            var title = new NativeActionWrap { Gap = 8 }; var name = Design.Text(S(account, "email") is { Length: > 0 } email ? email : id, 13); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.Children.Add(name);
            if (S(account, "plan_type") is { Length: > 0 } plan) title.Children.Add(ModelHint(char.ToUpperInvariant(plan[0]) + plan[1..]));
            if (B(account, "active")) title.Children.Add(ModelHint(T("subscriptions.active_account"))); row.Children.Add(title);
            if (S(account, "email").Length > 0) row.Children.Add(ModelHint(id));
            if (state.UsageErrors.TryGetValue(id, out var error)) row.Children.Add(ModelHint(error));
            else if (state.Usage.TryGetValue(id, out var usage))
            {
                if (B(usage, "limit_reached")) row.Children.Add(ModelHint(T("subscriptions.limit_reached")));
                var windows = new[] { usage["primary"], usage["secondary"] }.OfType<JsonObject>().ToArray();
                if (windows.Length == 0) row.Children.Add(ModelHint(T("subscriptions.usage_none")));
                foreach (var window in windows) SubscriptionUsage(row, window);
            }
            else row.Children.Add(ModelHint(T("subscriptions.usage_loading")));
            if (!B(account, "active"))
            {
                var actions = new NativeActionWrap { Gap = 8, RightAligned = true };
                var use = ModelButton(T("subscriptions.switch_account"), () => SubscriptionAccountAction("switch_codex_account", id)); StyleModelButton(use, 12); actions.Children.Add(use);
                var remove = Design.ToolButton(T("subscriptions.remove_account"), "trash"); remove.Click += (_, _) => Confirm(Design.Localization.Format("subscriptions.remove_account_confirm", ("account", S(account, "email") is { Length: > 0 } email ? email : id)), () => _ = SubscriptionAccountAction("remove_codex_account", id)); actions.Children.Add(remove);
                Grid.SetColumn(actions, 1); line.Children.Add(actions);
                line.SizeChanged += (_, _) =>
                {
                    var wide = NativeSettingsLayout.ModelFormColumns(XamlRoot?.Size.Width ?? ActualWidth);
                    Grid.SetColumnSpan(row, wide ? 1 : 2); Grid.SetColumn(actions, wide ? 1 : 0); Grid.SetRow(actions, wide ? 0 : 1); Grid.SetColumnSpan(actions, wide ? 1 : 2);
                };
            }
            else Grid.SetColumnSpan(row, 2);
            card.Children.Add(new Border { Child = line, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(0, 14, 0, 0) });
        }
    }
    private void SubscriptionUsage(StackPanel row, JsonObject window)
    {
        var seconds = window["window_seconds"]?.GetValue<long>() ?? 0;
        var used = Math.Clamp(Math.Round(window["used_percent"]?.GetValue<double>() ?? 0, MidpointRounding.AwayFromZero), 0, 100);
        var label = seconds <= 0 ? T("subscriptions.usage_limit") : Design.Localization.Format(seconds % 86400 == 0 ? "subscriptions.usage_days" : "subscriptions.usage_hours", ("n", (seconds % 86400 == 0 ? seconds / 86400 : Math.Max(1, (seconds + 1799) / 3600)).ToString()));
        var text = Design.Localization.Format("subscriptions.usage_used", ("n", used.ToString("0")));
        var reset = window["reset_at"]?.GetValue<long>() ?? 0;
        if (reset > 0)
        {
            var minutes = (Math.Max(0, reset - DateTimeOffset.UtcNow.ToUnixTimeSeconds()) + 59) / 60;
            var time = minutes >= 1440 ? L($"{minutes / 1440}天{minutes % 1440 / 60}小时", $"{minutes / 1440}d {minutes % 1440 / 60}h") : minutes >= 60 ? L($"{minutes / 60}小时{minutes % 60}分钟", $"{minutes / 60}h {minutes % 60}m") : L($"{minutes}分钟", $"{minutes}m");
            text += " · " + Design.Localization.Format("subscriptions.usage_resets", ("time", time));
        }
        var group = new Grid { ColumnSpacing = 10, RowSpacing = 4 };
        group.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); group.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); group.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        for (var i = 0; i < 3; i++) group.RowDefinitions.Add(new() { Height = GridLength.Auto });
        group.Children.Add(ModelHint(label));
        var bar = new ProgressBar { Minimum = 0, Maximum = 100, Value = used, Height = 6, Foreground = Design.Brush(used >= 100 ? "err" : used >= 80 ? "warn" : "clay") };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(bar, label); group.Children.Add(bar); group.Children.Add(ModelHint(text));
        void Arrange()
        {
            var wide = NativeSettingsLayout.ModelFormColumns(XamlRoot?.Size.Width ?? ActualWidth);
            for (var i = 0; i < group.Children.Count; i++) { var child = (FrameworkElement)group.Children[i]; Grid.SetColumn(child, wide ? i : 0); Grid.SetRow(child, wide ? 0 : i); Grid.SetColumnSpan(child, wide ? 1 : 3); child.VerticalAlignment = VerticalAlignment.Center; }
        }
        group.Loaded += (_, _) => Arrange(); group.SizeChanged += (_, _) => Arrange(); row.Children.Add(group);
    }
    private void SubscriptionForm()
    {
        if (subscriptionSettings is not { Provider: { } provider } state) return;
        var title = NativeSubscriptionSettingsModel.Providers.First(p => p.Id == provider).Title;
        SettingsContentWidth(1040); Body.Padding = new Thickness(24, 16, 24, 22);
        SettingsBreadcrumb(SectionTitle(Design, "models"), title, () => RequestLeave(Render));
        Form.Spacing = 10;
        string Key(string key) => NativeSubscriptionSettingsModel.LoginKey(provider, key);
        TextBlock Paragraph(string key)
        {
            var text = ModelHint(T(key)); text.Margin = new Thickness(0, 12, 0, 12); return text;
        }
        var heading = Design.Text(T(state.AccountOnly ? "subscriptions.sign_in" : "subscriptions.add_model"), 16); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        heading.Margin = new Thickness(0, 0, 0, 10); Form.Children.Add(heading);
        Form.Children.Add(Paragraph(state.AccountOnly ? Key("codex.login.desc") : "subscriptions.managed"));
        if (state.AccountOnly)
        {
            if (provider == "codex") Form.Children.Add(ModelFields(() =>
            {
                Choice(Form, T("codex.login.method"), state.Method, [("browser", T("codex.login.browser")), ("device", T("codex.login.device"))], value => { state.Set("method", value); Render(); });
                ((ComboBox)Form.Children.Last()).IsEnabled = !state.Pending;
            }));
            else Form.Children.Add(new Grid()); // The reference retains its empty method grid.
            Form.Children.Add(Paragraph(Key(state.Method == "device" ? "codex.login.device_hint" : "codex.login.browser_hint")));
            if (state.SavedAccount.Length > 0)
            {
                var account = ModelHint(T(provider == "codex" ? "subscriptions.active_account" : "subscriptions.signed_in") + " · " + state.SavedAccount);
                account.Margin = new Thickness(0, 12, 0, 12); Form.Children.Add(account);
            }
            if (provider == "codex") Form.Children.Add(Paragraph("subscriptions.add_account_hint"));
            var challenge = new StackPanel { Spacing = 12 }; Form.Children.Add(challenge);
            if (state.Pending && state.UserCode.Length > 0) challenge.Children.Add(Design.Text(T("codex.login.code") + "  " + state.UserCode, 14));
            if (state.Pending && state.LoginUrl.Length > 0)
            {
                var open = ModelButton(T("codex.login.url"), () => OpenSubscriptionUrl(state.LoginUrl)); Design.QuietButton(open); open.HorizontalAlignment = HorizontalAlignment.Left; challenge.Children.Add(open);
            }
            if (state.Method == "browser" && state.LoginId.Length > 0 && state.LoginStatus != "success")
            {
                var manual = ModelFields(() => { var redirect = Field(T(Key("codex.login.paste")), state.Redirect, value => state.Set("redirect", value)); redirect.PlaceholderText = T(Key("codex.login.paste_ph")); });
                var submit = ModelButton(T("codex.login.submit"), async () => { await state.SubmitRedirectAsync(); if (!model.Closed) Render(); }); StyleModelButton(submit); submit.HorizontalAlignment = HorizontalAlignment.Left; manual.Children.Add(submit);
                subscriptionManual = new Expander { Header = Design.Text(T("codex.login.manual"), 13), Content = manual, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch }; Form.Children.Add(subscriptionManual);
            }
        }
        else Form.Children.Add(ModelFields(() =>
        {
            Field(T("settings.model"), state.Model, value => state.Set("model", value));
            var alias = Field(T("settings.label"), state.Label, value => state.Set("label", value)); alias.PlaceholderText = T(Key("codex.login.label_ph"));
        }));
        subscriptionFeedback = ModelHint(""); Microsoft.UI.Xaml.Automation.AutomationProperties.SetLiveSetting(subscriptionFeedback, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite); Form.Children.Add(subscriptionFeedback);
        var actions = new NativeActionWrap { Gap = 10, RowGap = 8, RightAligned = true };
        var cancel = ModelButton(T("settings.cancel"), async () => { if (await state.CloseFormAsync() && !model.Closed) Render(); }); StyleModelButton(cancel); actions.Children.Add(cancel);
        if (state.AccountOnly)
        {
            var start = ModelButton(T(state.Pending ? Key("codex.login.waiting") : state.LoginStatus == "success" || provider != "codex" && state.SavedAccount.Length > 0 ? "codex.login.again" : "codex.login.start"), async () =>
            {
                if (await state.StartAsync() && !model.Closed && state.Pending && (state.Method == "browser" || provider == "xai")) await OpenSubscriptionUrl(state.LoginUrl);
                if (!model.Closed) Render();
            }); StyleModelButton(start); start.IsEnabled = !state.Pending; actions.Children.Add(start);
            if (state.Pending) { var stop = ModelButton(T("codex.login.cancel_attempt"), async () => { await state.CancelAttemptAsync(); if (!model.Closed) Render(); }); StyleModelButton(stop); actions.Children.Add(stop); }
        }
        if (!state.AccountOnly || state.LoginStatus == "success")
        {
            subscriptionSave = ModelButton(T(state.AccountOnly ? "subscriptions.save_account" : "codex.login.save"), async () =>
            {
                if (await state.SaveAsync()) { if (state.SavedModels != null) model.Values["list_models"] = state.SavedModels.DeepClone(); await Reload(); }
                if (!model.Closed) Render();
            }); StyleSubscriptionPrimary(subscriptionSave); actions.Children.Add(subscriptionSave);
        }
        foreach (var button in actions.Children.OfType<Button>()) { button.MinWidth = 84; button.MinHeight = 40; }
        Form.Children.Add(new Border { Child = actions, Margin = new Thickness(0, 4, 0, 0), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(0, 14, 0, 0) });
        var content = new StackPanel { Spacing = 10 };
        while (Form.Children.Count > 0) { var child = Form.Children[0]; Form.Children.RemoveAt(0); content.Children.Add(child); }
        var frame = new Border { Child = content, Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Padding = new Thickness(16, 14, 16, 14), Margin = new Thickness(0, 4, 0, 0), MaxWidth = 920, HorizontalAlignment = HorizontalAlignment.Stretch };
        Form.Children.Add(frame);
        PreferenceFieldAppearance.ApplyTree(frame, Design);
        UpdateSubscriptionFeedback();
    }
    private static async Task OpenSubscriptionUrl(string url)
    {
        if (Uri.TryCreate(url, UriKind.Absolute, out var uri) && uri.Scheme is "https" or "http") await Windows.System.Launcher.LaunchUriAsync(uri);
    }
    private void StyleSubscriptionPrimary(Button button)
    {
        StyleModelButton(button, primary: true);
        button.Resources["ButtonBackgroundDisabled"] = Design.Brush("clay");
        button.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay");
        if (button.Content is TextBlock text) { text.Foreground = Design.Brush("on-clay"); text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; }
    }
    private void UpdateSubscriptionFeedback()
    {
        if (subscriptionSettings is not { } state) return;
        if (subscriptionSave != null) { subscriptionSave.IsEnabled = state.CanSave && pendingConfirmation == null; subscriptionSave.Opacity = subscriptionSave.IsEnabled ? 1 : .55; }
        if (subscriptionFeedback == null) return;
        var text = state.Error ?? (state.FormOpen ? state.LoginStatus == "success" ? T("codex.login.ready") : state.Pending ? T("codex.login.pending") : state.LoginMessage : "");
        subscriptionFeedback.Text = text; subscriptionFeedback.Visibility = text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        subscriptionFeedback.Foreground = Design.Brush(state.Error != null || state.LoginStatus == "error" ? "err" : "text-muted");
    }
}
