using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private static readonly JsonObject[] CredentialGroups = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", "credential-groups.json")))!.AsArray().OfType<JsonObject>().ToArray();
    private JsonObject? credentialGroup;
    private Button? credentialAdd;
    private Flyout? credentialHelp;
    private readonly List<(PasswordBox Box, RoutedEventHandler Changed)> credentialSecrets = [];
    private void ClearCredentialControls()
    {
        credentialHelp?.Hide();
        credentialHelp = null;
        // Detach before clearing: WinUI can deliver PasswordChanged after this
        // render returns. An old empty box must never erase the retained draft.
        foreach (var (box, changed) in credentialSecrets) { box.PasswordChanged -= changed; box.Password = ""; }
        credentialSecrets.Clear(); credentialAdd = null;
    }
    private TextBlock CredentialText(string text, double size = 12, string color = "text-muted", bool strong = false)
    {
        var block = Design.Text(text, size); block.Foreground = Design.Brush(color);
        if (strong) block.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        block.LineHeight = Design.FontSize(size) * (size == 15 ? 1.2 : 1.45); block.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        return block;
    }
    private Button CredentialLink(string label, Func<Task> action, bool danger = false)
    {
        var button = Button(label, action); Design.QuietButton(button); button.MinHeight = 0; button.Padding = new Thickness(0);
        var text = CredentialText(label, 11, danger ? "err" : "text-faint"); text.TextDecorations = Windows.UI.Text.TextDecorations.Underline;
        button.Content = text; return button;
    }
    private void BackToCredentials()
    {
        if (Busy) return;
        model.DiscardCredentialInputs(CredentialGroups.SelectMany(group => Rows(group["fields"]).Select(field => S(field, "id"))));
        credentialGroup = null; Render(); BodyScroll.ChangeView(null, 0, null, true);
    }
    private async Task CredentialAction(Func<Task<bool>> action)
    {
        await action(); if (!model.Closed) Render();
    }
    private Control CredentialInput(string label, string value, string placeholder, bool secret, Action<string> changed)
    {
        Control input;
        if (secret)
        {
            var box = new PasswordBox { Password = value, PlaceholderText = placeholder, PasswordRevealMode = PasswordRevealMode.Hidden };
            RoutedEventHandler passwordChanged = (_, _) => changed(box.Password);
            box.PasswordChanged += passwordChanged;
            credentialSecrets.Add((box, passwordChanged)); input = box;
        }
        else
        {
            var box = new TextBox { Text = value, PlaceholderText = placeholder };
            box.TextChanged += (_, _) => changed(box.Text); textMenus.Register(box); input = box;
        }
        Design.BindTypography(input, size: 14); input.Padding = new Thickness(11, 8, 11, 8); input.MinHeight = 36;
        input.CornerRadius = new CornerRadius(8); input.BorderThickness = new Thickness(1);
        input.BorderBrush = Design.Brush("border-strong"); input.Background = Design.Brush("bg-input"); input.Foreground = Design.Brush("text");
        AutomationProperties.SetName(input, label); return input;
    }
    private void Credentials()
    {
        Body.Padding = new Thickness(24, 18, 24, 22); Body.Spacing = 0; Form.Spacing = 0;
        if (credentialGroup != null) SettingsBreadcrumb(T("settings.nav.credentials"), T(S(credentialGroup, "name_key")), BackToCredentials);
        Form.Children.Add(CredentialText(T("cred.desc")));
        if (credentialGroup == null) CredentialDirectory(); else CredentialDetail(credentialGroup);
        var message = CredentialText(model.Error ?? (model.CredentialFeedbackKey is { } key ? T(key) : ""), 12, model.Error != null ? "err" : "ok");
        message.Margin = new Thickness(0, 12, 0, 0); message.Visibility = message.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        AutomationProperties.SetLiveSetting(message, model.Error != null ? Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive : Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        Form.Children.Add(message);
        var actions = new NativeActionWrap { Gap = 10, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 24, 0, 0) };
        if (credentialGroup != null) actions.Children.Add(Button(T("settings.cancel"), () => { BackToCredentials(); return Task.CompletedTask; }));
        var save = Button(T("settings.save"), () => CredentialAction(() => model.SaveCredentialsAsync(credentialGroup == null
            ? model.CustomCredentials.Select(row => S(row, "id")) : Rows(credentialGroup["fields"]).Select(row => S(row, "id")))));
        StyleGeneralButton(save, primary: true); actions.Children.Add(save); Form.Children.Add(actions);
        foreach (var button in actions.Children.OfType<Button>()) { if (button != save) StyleGeneralButton(button); button.MinWidth = 84; }
    }
    private void CredentialDirectory()
    {
        var list = new StackPanel();
        for (var i = 0; i < CredentialGroups.Length; i++)
        {
            var group = CredentialGroups[i]; var fields = Rows(group["fields"]).ToArray(); var count = fields.Count(field => model.CredentialPresent(S(field, "id")));
            var copy = new StackPanel { Spacing = 6 }; copy.Children.Add(CredentialText(T(S(group, "name_key")), 15, "text", true));
            var about = CredentialText(T(S(group, "about_key"))); about.LineHeight = Design.FontSize(12) * 1.6; copy.Children.Add(about);
            var trailing = new Grid { ColumnSpacing = 18, VerticalAlignment = VerticalAlignment.Center };
            trailing.ColumnDefinitions.Add(new()); trailing.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            trailing.Children.Add(CredentialText(count == 0 ? T("cred.not_stored") : $"{T("cred.stored")} {count}/{fields.Length}"));
            var chevron = Design.Icon("chevron-right", 18, Design.ColorText("text-faint")); Grid.SetColumn(chevron, 1); trailing.Children.Add(chevron);
            var row = new Grid { ColumnSpacing = 18 };
            row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
            row.Children.Add(copy); Grid.SetColumn(trailing, 1); row.Children.Add(trailing);
            var button = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                Padding = new Thickness(20), MinHeight = 0, BorderThickness = new Thickness(0, 0, 0, i == CredentialGroups.Length - 1 ? 0 : 1),
                BorderBrush = Design.Brush("border"), Background = Design.Brush("bg-app"), CornerRadius = new CornerRadius(0) };
            void Arrange()
            {
                var narrow = (XamlRoot?.Size.Width ?? ActualWidth) <= 700;
                Grid.SetColumnSpan(copy, narrow ? 2 : 1); Grid.SetColumnSpan(trailing, narrow ? 2 : 1);
                Grid.SetColumn(trailing, narrow ? 0 : 1); Grid.SetRow(trailing, narrow ? 1 : 0); row.RowSpacing = narrow ? 10 : 0;
                button.Padding = narrow ? new Thickness(12, 16, 12, 16) : new Thickness(20);
            }
            button.Loaded += (_, _) => Arrange(); button.SizeChanged += (_, _) => Arrange();
            AutomationProperties.SetName(button, T(S(group, "name_key")) + ". " + T(S(group, "about_key")) + ". " + (count == 0 ? T("cred.not_stored") : $"{T("cred.stored")} {count}/{fields.Length}"));
            button.Click += (_, _) => { credentialGroup = group; model.DiscardCredentialInputs(fields.Select(field => S(field, "id"))); Render(); BodyScroll.ChangeView(null, 0, null, true); };
            list.Children.Add(button);
        }
        Form.Children.Add(new Border { Child = list, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Margin = new Thickness(0, 32, 0, 30) });
        var label = CredentialText(T("cred.custom.name").ToUpperInvariant(), 11, "text-faint", true); label.CharacterSpacing = 60;
        label.Margin = new Thickness(2, 30, 2, 6); Form.Children.Add(label);
        var hint = CredentialText(T("cred.custom.hint")); hint.Margin = new Thickness(0, 12, 0, 8); Form.Children.Add(hint);
        foreach (var row in model.CustomCredentials) CredentialCustomRow(row);
        CredentialAddForm();
    }
    private void CredentialDetail(JsonObject group)
    {
        var name = T(S(group, "name_key")); var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, Margin = new Thickness(2, 30, 2, 6) };
        var caption = CredentialText(name.ToUpperInvariant(), 11, "text-faint", true); caption.CharacterSpacing = 60; caption.VerticalAlignment = VerticalAlignment.Center; heading.Children.Add(caption);
        var help = Design.ToolButton(name + ": " + T("cred.help.aria"), "circle-help"); help.Width = help.Height = 20; help.MinWidth = help.MinHeight = 0; help.Padding = new Thickness(0);
        var helpBody = new StackPanel { Spacing = 10, MaxWidth = 340 };
        foreach (var (title, key) in new[] { ("cred.help.what", "about_key"), ("cred.help.configured", "configured_key"), ("cred.help.unconfigured", "unconfigured_key") })
        {
            var section = new StackPanel { Spacing = 2 }; section.Children.Add(CredentialText(T(title), 11.5, "text", true)); section.Children.Add(CredentialText(T(S(group, key)))); helpBody.Children.Add(section);
        }
        // Own one help surface so Escape cannot close a flyout only to reveal
        // a second automatically opened tooltip under the unchanged pointer.
        ToolTipService.SetToolTip(help, null);
        var flyout = new Flyout { Content = helpBody }; credentialHelp = flyout; textMenus.Register(flyout);
        // A transient native flyout supports hover and keyboard focus without
        // stealing focus from the form. The window Escape stack owns dismissal.
        void ShowHelp() => flyout.ShowAt(help, new Microsoft.UI.Xaml.Controls.Primitives.FlyoutShowOptions { ShowMode = Microsoft.UI.Xaml.Controls.Primitives.FlyoutShowMode.Transient });
        help.PointerEntered += (_, _) => ShowHelp();
        help.GotFocus += (_, _) => { if (help.FocusState == FocusState.Keyboard) ShowHelp(); };
        help.Click += (_, _) => ShowHelp();
        heading.Children.Add(help); Form.Children.Add(heading);
        foreach (var field in Rows(group["fields"]))
        {
            var id = S(field, "id"); var stored = model.CredentialPresent(id); var label = T(S(field, "label_key"));
            var fieldStack = new StackPanel { Spacing = 5, Margin = new Thickness(0, 12, 0, 0) };
            var fieldHead = CredentialText(label + " — " + T(stored ? "cred.stored" : "cred.not_stored"));
            fieldStack.Children.Add(stored ? PermissionRow(fieldHead, CredentialLink(T("cred.clear"), () => CredentialAction(() => model.ClearCredentialAsync(id))), 160) : fieldHead);
            fieldStack.Children.Add(CredentialInput(label, model.CredentialInput(id), stored ? T("settings.stored_key") : "", B(field, "secret"), v => model.SetCredentialInput(id, v)));
            Form.Children.Add(fieldStack);
        }
        var setup = new NativeActionWrap { Gap = 10, Margin = new Thickness(0, 14, 0, 0) };
        setup.Children.Add(CredentialText(T(S(group, "hint_key")), 12, "text-faint"));
        foreach (var link in Rows(group["links"]))
        {
            var url = S(link, "url"); var button = CredentialLink(T(S(link, "label_key")), async () => await Windows.System.Launcher.LaunchUriAsync(new Uri(url)));
            var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 3 }; var text = CredentialText(T(S(link, "label_key")), 12, "clay"); text.TextDecorations = Windows.UI.Text.TextDecorations.Underline;
            content.Children.Add(text); content.Children.Add(Design.Icon("external-link", 12, Design.ColorText("clay"))); button.Content = content; setup.Children.Add(button);
        }
        Form.Children.Add(setup);
    }
    private Border CredentialFrame(StackPanel content, double padding = 14) => new() { Child = content, Padding = new Thickness(padding), Margin = new Thickness(0, 10, 0, 0),
        Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8) };
    private void CredentialCustomRow(JsonObject row)
    {
        var id = S(row, "id"); var stored = model.CredentialPresent(id); var card = new StackPanel { Spacing = 10 };
        var meta = new NativeActionWrap { Gap = 10 }; meta.Children.Add(CredentialText(S(row, "name"), 13.5, "text", true));
        var env = CredentialText(S(row, "env_var"), 11.5); env.FontFamily = Design.CodeFont; meta.Children.Add(env);
        meta.Children.Add(CredentialText(T(stored ? "cred.stored" : "cred.not_stored"), 11, "text-faint"));
        var actions = new NativeActionWrap { Gap = 10 };
        if (stored) actions.Children.Add(CredentialLink(T("cred.clear"), () => CredentialAction(() => model.ClearCredentialAsync(id))));
        actions.Children.Add(CredentialLink(T("specialists.remove"), () => CredentialAction(() => model.RemoveCustomCredentialAsync(id)), true));
        card.Children.Add(PermissionRow(meta, actions, 250)); card.Children.Add(CredentialInput(S(row, "name"), model.CredentialInput(id), T(stored ? "settings.stored_key" : "cred.custom.value_placeholder"), true, v => model.SetCredentialInput(id, v)));
        Form.Children.Add(CredentialFrame(card));
    }
    private void CredentialAddForm()
    {
        var card = new StackPanel { Spacing = 12 }; card.Children.Add(CredentialText(T("cred.custom.add"), 14, "text", true));
        StackPanel Field(string label, string value, string placeholder, bool secret, Action<string> changed)
        {
            var field = new StackPanel { Spacing = 5 }; field.Children.Add(CredentialText(label));
            field.Children.Add(CredentialInput(label, value, placeholder, secret, v => { changed(v); if (credentialAdd != null) credentialAdd.IsEnabled = model.CanAddCredential; })); return field;
        }
        var name = Field(T("cred.custom.service_name"), model.CustomCredentialName, T("cred.custom.service_placeholder"), false, v => model.CustomCredentialName = v);
        var env = Field(T("cred.custom.env_var"), model.CustomCredentialEnvironment, "METASO_API_KEY", false, v => model.CustomCredentialEnvironment = v);
        Design.BindTypography((Control)env.Children[1], code: true, size: 14);
        var pair = new Grid { ColumnSpacing = 14 }; pair.ColumnDefinitions.Add(new()); pair.ColumnDefinitions.Add(new()); pair.RowDefinitions.Add(new() { Height = GridLength.Auto }); pair.RowDefinitions.Add(new() { Height = GridLength.Auto });
        pair.Children.Add(name); pair.Children.Add(env); Grid.SetColumn(env, 1);
        pair.SizeChanged += (_, e) => { var narrow = e.NewSize.Width < 500; Grid.SetColumnSpan(name, narrow ? 2 : 1); Grid.SetColumnSpan(env, narrow ? 2 : 1); Grid.SetColumn(env, narrow ? 0 : 1); Grid.SetRow(env, narrow ? 1 : 0); pair.RowSpacing = narrow ? 12 : 0; };
        card.Children.Add(pair); card.Children.Add(Field(T("cred.custom.value"), model.CustomCredentialValue, T("cred.custom.value_placeholder"), true, v => model.CustomCredentialValue = v));
        card.Children.Add(CredentialText(T("cred.custom.env_hint"), 12, "text-faint"));
        credentialAdd = Button(T("cred.custom.add"), () => CredentialAction(model.AddCustomCredentialAsync)); StyleGeneralButton(credentialAdd);
        credentialAdd.IsEnabled = model.CanAddCredential; credentialAdd.HorizontalAlignment = HorizontalAlignment.Right; card.Children.Add(credentialAdd);
        var frame = CredentialFrame(card, 16); frame.Margin = new Thickness(0, 14, 0, 0); Form.Children.Add(frame);
    }
}
