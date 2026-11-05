using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private bool standaloneEnvironmentHost;

    // Reuse the settings editor without loading/mounting the settings page.
    // Creating a host only needs a fresh local draft; opening/cancelling does no I/O.
    public static NativeSettingsSectionPage CreateEnvironmentHostDialog(INativeSettingsClient client, string? project, WispDesign design, Action close)
    {
        var state = new NativeSettingsEditorModel(client, project) { Localization = design.Localization };
        var page = new NativeSettingsSectionPage(state, project, "environments", design, close, null, modal: true)
        { standaloneEnvironmentHost = true };
        page.ModalChanged += overlay =>
        {
            if (overlay != null) page.Content = overlay;
            else page.DispatcherQueue.TryEnqueue(() =>
            {
                // Close after the current state notification has unwound. Disposal
                // also clears the overlay, but must not close a restored parent.
                if (!state.Closed && state.EnvironmentEditor == null) close();
            });
        };
        state.OpenEnvironmentHost();
        return page;
    }

    private Grid? environmentOverlay;
    private string? environmentModalKind;
    private Button? environmentSave, environmentTest;
    private TextBlock? environmentError;
    private StackPanel? environmentIdentity, environmentPassword;
    private readonly List<Control> environmentInputs = [];
    private PasswordBox? environmentSecret;
    private RoutedEventHandler? environmentSecretChanged;
    private void HideEnvironmentModal()
    {
        if (environmentOverlay != null) ModalChanged?.Invoke(null);
        if (environmentSecret != null) { environmentSecret.PasswordChanged -= environmentSecretChanged; environmentSecret.Password = ""; }
        environmentInputs.Clear(); environmentOverlay = null; environmentModalKind = null;
        environmentSave = environmentTest = null; environmentError = null; environmentIdentity = environmentPassword = null;
        environmentSecret = null; environmentSecretChanged = null;
    }
    private void RefreshEnvironmentModal()
    {
        if (section != "environments" || model.Closed) return;
        foreach (var (id, feedback) in environmentProbeFeedback) feedback.Visibility = model.ProbingEnvironment == id ? Visibility.Visible : Visibility.Collapsed;
        if (environmentModalKind != model.EnvironmentEditor)
        {
            HideEnvironmentModal();
            if (model.EnvironmentEditor != null) ShowEnvironmentModal();
        }
        foreach (var input in environmentInputs) input.IsEnabled = !Busy && !(input.Tag as string == "alias" && model.EditingEnvironmentHost);
        if (environmentSave != null) { environmentSave.IsEnabled = model.EnvironmentEditor == "remove" ? !Busy : model.CanSaveEnvironment; environmentSave.Opacity = environmentSave.IsEnabled ? 1 : .5; }
        if (environmentTest != null)
        {
            environmentTest.IsEnabled = model.CanTestEnvironmentHost;
            if (environmentTest.Content is TextBlock label) label.Text = T(model.EnvironmentTesting ? "hosts.testing" : "hosts.test");
        }
        if (environmentError != null)
        {
            environmentError.Text = model.Error ?? (model.EnvironmentTestPassed ? T("hosts.test_ok") : "");
            environmentError.Foreground = Design.Brush(model.Error != null ? "err" : "ok"); environmentError.Visibility = environmentError.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        }
        var password = NativeEnvironments.Text(model.EnvironmentDraft, "auth_method") == "password";
        if (environmentIdentity != null) environmentIdentity.Visibility = password ? Visibility.Collapsed : Visibility.Visible;
        if (environmentPassword != null) environmentPassword.Visibility = password ? Visibility.Visible : Visibility.Collapsed;
    }
    private TextBox EnvironmentInput(StackPanel parent, string field, string labelKey, string placeholder, bool runtime = false, bool multiline = false)
    {
        if (runtime) { var fieldBody = new StackPanel { Spacing = 6 }; parent.Children.Add(fieldBody); parent = fieldBody; }
        var label = EnvironmentText(T(labelKey), 12, "text"); label.Margin = runtime ? new Thickness(0) : new Thickness(0, 12, 0, 4); parent.Children.Add(label);
        var input = new TextBox { Tag = field, Text = NativeEnvironments.Text(model.EnvironmentDraft, field), PlaceholderText = placeholder,
            AcceptsReturn = multiline, TextWrapping = multiline ? TextWrapping.Wrap : TextWrapping.NoWrap, MinHeight = multiline ? 64 : 36,
            IsSpellCheckEnabled = false, IsTextPredictionEnabled = false, Padding = new Thickness(10, 8, 10, 8), CornerRadius = new CornerRadius(8), BorderThickness = new Thickness(1),
            Background = Design.Brush("bg-input"), Foreground = Design.Brush("text"), BorderBrush = Design.Brush("border-strong") };
        Design.BindTypography(input, code: runtime, size: runtime ? 12.5 : 13.5); AutomationProperties.SetLabeledBy(input, label); textMenus.Register(input);
        input.TextChanged += (_, _) => model.SetEnvironmentField(field, input.Text); environmentInputs.Add(input);
        var frame = NativeDialogVisuals.Input(input, Design);
        if (runtime && S(model.EnvironmentTarget, "kind") == "local" && pickPath != null)
        {
            var row = new Grid { ColumnSpacing = 8 }; row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.Children.Add(frame);
            var browse = EnvironmentButton(T("runtime_config.browse"), async () =>
            {
                var draft = model.EnvironmentDraft; var path = await pickPath(false);
                if (!model.Closed && ReferenceEquals(draft, model.EnvironmentDraft) && path != null) input.Text = path;
            }); browse.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(browse, 1); row.Children.Add(browse); environmentInputs.Add(browse); parent.Children.Add(row);
            browse.Padding = new Thickness(14, 8, 14, 8); if (browse.Content is TextBlock browseLabel) Design.BindTypography(browseLabel, 13.5);
        }
        else parent.Children.Add(frame);
        return input;
    }
    private void ShowEnvironmentModal()
    {
        var kind = model.EnvironmentEditor!; environmentModalKind = kind;
        var overlay = new Grid(); environmentOverlay = overlay; NativeDialogVisuals.Backdrop(overlay, Design);
        var body = new StackPanel { Padding = new Thickness(22), Spacing = 14 };
        var diagnosisKind = NativeEnvironments.FailureKind(NativeEnvironments.Text(model.EnvironmentTarget, "failure"));
        var diagnosisPrefix = diagnosisKind is "password" or "key" or "client" or "probe_output" ? diagnosisKind : "fail";
        var titleKey = kind switch { "host" => model.EditingEnvironmentHost ? "hosts.edit" : "hosts.add", "runtime" => "runtime_config.title", "diagnosis" => "ssh_check." + diagnosisPrefix + "_title", _ => "environments.remove" };
        var header = new Grid { ColumnSpacing = 12 }; header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = EnvironmentText(T(titleKey), 17, kind == "diagnosis" ? "err" : "text", true); title.VerticalAlignment = VerticalAlignment.Center; header.Children.Add(title);
        var close = Design.ToolButton(T("hosts.cancel"), "close"); close.Width = close.Height = 28; close.MinHeight = close.MinWidth = 0; close.Padding = new Thickness(0);
        close.Click += (_, _) => model.CancelEnvironmentEditor(); environmentInputs.Add(close); Grid.SetColumn(close, 1); header.Children.Add(close); body.Children.Add(header);
        AutomationProperties.SetName(overlay, title.Text);
        Control? focus = null;
        if (kind == "host")
        {
            focus = EnvironmentInput(body, "alias", "hosts.name", T("hosts.name_ph"));
            var address = EnvironmentInput(body, "host_name", "hosts.hostname", T("hosts.hostname_ph")); if (model.EditingEnvironmentHost) focus = address;
            EnvironmentInput(body, "user", "hosts.user", T("hosts.user_ph")); EnvironmentInput(body, "port", "hosts.port", "22");
            var authLabel = EnvironmentText(T("hosts.auth_method"), 12, "text"); authLabel.Margin = new Thickness(0, 12, 0, 4); body.Children.Add(authLabel);
            var auth = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch, MinHeight = 36, Padding = new Thickness(10, 8, 10, 8),
                Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), CornerRadius = new CornerRadius(8) };
            auth.Items.Add(T("hosts.auth_key")); auth.Items.Add(T("hosts.auth_password")); auth.SelectedIndex = S(model.EnvironmentDraft, "auth_method") == "password" ? 1 : 0;
            Design.BindTypography(auth, size: 13.5); AutomationProperties.SetLabeledBy(auth, authLabel); choices.Add(auth); environmentInputs.Add(auth); body.Children.Add(auth);
            auth.SelectionChanged += (_, _) => model.SetEnvironmentField("auth_method", auth.SelectedIndex == 1 ? "password" : "key");
            environmentIdentity = new StackPanel { Spacing = 14 }; EnvironmentInput(environmentIdentity, "identity_file", "hosts.identity", T("hosts.identity_ph")); body.Children.Add(environmentIdentity);
            environmentPassword = new StackPanel { Spacing = 14 }; var passwordLabel = EnvironmentText(T("hosts.password"), 12, "text"); passwordLabel.Margin = new Thickness(0, 12, 0, 4); environmentPassword.Children.Add(passwordLabel);
            var secret = new PasswordBox { Password = S(model.EnvironmentDraft, "password"), PlaceholderText = T(B(model.EnvironmentDraft, "has_password") ? "hosts.password_keep" : "hosts.password_ph"), PasswordRevealMode = PasswordRevealMode.Hidden,
                Padding = new Thickness(10, 8, 10, 8), MinHeight = 36, CornerRadius = new CornerRadius(8), BorderThickness = new Thickness(1), Background = Design.Brush("bg-input"), Foreground = Design.Brush("text"), BorderBrush = Design.Brush("border-strong") };
            Design.BindTypography(secret, size: 13.5); AutomationProperties.SetLabeledBy(secret, passwordLabel);
            environmentSecret = secret; environmentSecretChanged = (_, _) => model.SetEnvironmentField("password", secret.Password); secret.PasswordChanged += environmentSecretChanged;
            environmentInputs.Add(secret); environmentPassword.Children.Add(NativeDialogVisuals.Input(secret, Design)); environmentPassword.Children.Add(EnvironmentText(T("hosts.password_hint"))); body.Children.Add(environmentPassword);
            EnvironmentInput(body, "notes", "hosts.notes", T("hosts.notes_ph"), multiline: true); body.Children.Add(EnvironmentText(T("hosts.openssh_hint")));
        }
        else if (kind == "runtime")
        {
            body.Children.Add(EnvironmentText(Design.Localization.Format("runtime_config.scope", ("context", NativeEnvironments.Label(model.EnvironmentTarget!)))));
            focus = EnvironmentInput(body, "python_executable", "runtime_config.python", T("runtime_config.python_placeholder"), runtime: true);
            EnvironmentInput(body, "rscript_executable", "runtime_config.r", T("runtime_config.r_placeholder"), runtime: true); body.Children.Add(EnvironmentText(T("runtime_config.hint"), 11.5));
        }
        else if (kind == "remove")
        {
            body.Children.Add(EnvironmentText(Design.Localization.Format("hosts.remove_confirm", ("host", S(model.EnvironmentTarget, "alias"))), 14, "text"));
            var report = model.EnvironmentTarget!["report"]!;
            body.Children.Add(EnvironmentText(Design.Localization.Format("hosts.disposal_detail", ("refs", NativeEnvironments.Text(report, "external_references")),
                ("files", NativeEnvironments.Text(report, "staged_files")), ("runs", NativeEnvironments.Text(report, "active_runs")))));
        }
        else
        {
            body.Children.Add(EnvironmentText(Design.Localization.Format("ssh_check." + diagnosisPrefix + "_body", ("host", NativeEnvironments.Label(model.EnvironmentTarget!))), 13));
            body.Children.Add(new Border { Padding = new Thickness(12), Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(8),
                Child = EnvironmentText(Design.Localization.Format("ssh_check.detail", ("detail", NativeEnvironments.Text(model.EnvironmentTarget, "failure"))), 12, "err") });
            body.Children.Add(EnvironmentText(T("ssh_check.causes_title"), 13, "text", true));
            foreach (var cause in NativeEnvironments.FailureCauses(diagnosisKind)) body.Children.Add(EnvironmentText("• " + T(cause), 12));
        }
        if (kind != "diagnosis")
        {
            environmentError = EnvironmentText("", 12, "err"); AutomationProperties.SetLiveSetting(environmentError, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive); body.Children.Add(environmentError);
        }
        var actions = new NativeActionWrap { Gap = 8, HorizontalAlignment = HorizontalAlignment.Right };
        if (kind == "host")
        {
            environmentTest = EnvironmentButton(T("hosts.test"), async () => { await model.TestEnvironmentHostAsync(); }); actions.Children.Add(environmentTest);
        }
        var cancel = EnvironmentButton(T(kind == "diagnosis" ? "ssh_check.cancel" : "hosts.cancel"), () => { model.CancelEnvironmentEditor(); return Task.CompletedTask; }); environmentInputs.Add(cancel); actions.Children.Add(cancel);
        if (kind == "diagnosis")
        {
            var id = S(model.EnvironmentTarget, "id");
            actions.Children.Add(EnvironmentButton(T("ssh_check.jump"), () => { model.CancelEnvironmentEditor(); Render(); return Task.CompletedTask; }));
            actions.Children.Add(EnvironmentButton(T("ssh_check.fix_host"), () => { model.CancelEnvironmentEditor(); model.OpenEnvironmentHost(id[4..]); return Task.CompletedTask; }, primary: true));
            actions.Children.Add(EnvironmentButton(T("ssh_check.reprobe_after_fix"), () => { model.CancelEnvironmentEditor(); return EnvironmentAction(() => model.ProbeEnvironmentAsync(id)); }));
        }
        else
        {
            var saveKey = kind == "host" ? model.EditingEnvironmentHost ? "hosts.update" : "hosts.save" : kind == "remove" ? "environments.remove" : "settings.save";
            environmentSave = EnvironmentButton(T(saveKey), () => EnvironmentAction(kind == "remove" ? model.RemoveEnvironmentHostAsync : model.SaveEnvironmentAsync, kind == "host"), primary: true); actions.Children.Add(environmentSave);
        }
        foreach (var button in actions.Children.OfType<Button>())
        {
            button.Padding = new Thickness(14, 8, 14, 8);
            if (button.Content is TextBlock label) Design.BindTypography(label, 13.5);
        }
        body.Children.Add(actions);
        var scroll = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var card = new Border { Child = scroll, Background = Design.Brush("bg-elev"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(16),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24), TabFocusNavigation = KeyboardNavigationMode.Cycle };
        overlay.Children.Add(card);
        void Resize() { card.Width = Math.Min(kind == "host" ? 460 : kind == "runtime" ? 560 : 520, Math.Max(0, overlay.ActualWidth - 32)); card.MaxHeight = Math.Max(0, overlay.ActualHeight - 40); body.Width = Math.Max(0, card.Width - 2); }
        overlay.SizeChanged += (_, _) => Resize(); overlay.Loaded += (_, _) =>
        {
            Resize(); DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
            { if (ReferenceEquals(environmentOverlay, overlay)) (focus ?? cancel).Focus(FocusState.Programmatic); });
        };
        ModalChanged?.Invoke(overlay); Design.ApplyTypography(body);
    }
}
