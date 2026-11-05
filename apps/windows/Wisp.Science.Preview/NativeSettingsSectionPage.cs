using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// Native editors for the settings reached from the capability summary.
internal sealed partial class NativeSettingsSectionPage : NativeActionPage
{
    private readonly NativeSettingsEditorModel model;
    private readonly string section;
    private readonly bool projectScoped;
    private readonly string? settingsProjectId;
    private INativeSettingsClient? settingsClient;
    private readonly Func<bool, Task<string?>>? pickPath;
    private readonly List<ComboBox> choices = [];
    private readonly SettingsTextMenus textMenus = new();
    private readonly StackPanel confirmation = new() { Spacing = 8 };
    private Action? pendingConfirmation;
    private readonly Button cancelAuthorization = new() { Content = "取消 OAuth 授权", Visibility = Visibility.Collapsed };
    private readonly TextBlock saveFeedback = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Grid preferenceFooter = new() { ColumnSpacing = 12, RowSpacing = 8 };
    private readonly StackPanel editorActions = new() { Orientation = Orientation.Horizontal, Spacing = 12 };
    private readonly List<StackPanel> settingCards = [];
    public event Action<string>? WorkflowRequested;
    public event Action<string>? ProjectRequested;
    public event Action? LanguageChanged;
    private string T(string key) => Design.T(key);
    private string L(string chinese, string english) => Design.L(chinese, english);
    public NativeWindowPreferences WindowPreferences { private get; init; } = new();
    protected override bool ErrorHandledInline => section is "permissions" or "credentials" or "environments" || acpNotice != null
        || model?.EditingWindowPreferences == true && !model.LoadFailed && !showInlineValidation
            && preferenceFooter.Parent is Panel inline && inline != Footer;
    private void RestoreLanguage(bool includeWindowDraft = false)
    {
        var changed = includeWindowDraft && WindowPreferences.PreviewLocale is { } locale
            ? Design.Localization.Preview(locale) : Design.Localization.ApplyCommittedSettings(model.Values["get_settings"]);
        if (changed) LanguageChanged?.Invoke();
    }
    internal static string SectionTitle(WispDesign design, string section)
    {
        if (section == "project") return design.T("proj_settings.title");
        var key = "settings.nav." + section.Replace('-', '_');
        var title = design.T(key);
        return title == key ? Titles.GetValueOrDefault(section, section) : title;
    }
    private Button? quickActionSave;
    private Button? specialistSave;
    public bool HasChanges => model.HasChanges || model.HasCredentialChanges || ChannelHasChanges || localEnvironment?.HasChanges == true || networkSettings?.HasChanges == true || decisionSettings?.HasChanges == true || subscriptionSettings?.HasChanges == true || subscriptionRoutes?.HasChanges == true || model.Draft != null && invalidFields.Count > 0;
    private bool pageBusy => model.Busy || channelBinding?.Busy == true || syncJoin?.Busy == true || networkSettings?.Busy == true || specialistTransfer?.Busy == true || decisionSettings?.Busy == true || subscriptionSettings?.Busy == true || subscriptionRoutes?.Busy == true;
    public bool Busy => pageBusy || localEnvironment?.Saving == true;
    private static string S(JsonNode? row, string key) => row?[key]?.GetValue<string>() ?? "";
    private static bool B(JsonNode? row, string key) => row?[key]?.GetValue<bool>() == true;
    private static IEnumerable<JsonObject> Rows(JsonNode? value) => (value as JsonArray)?.OfType<JsonObject>() ?? [];
    public NativeSettingsSectionPage(INativeSettingsClient client, string? project, string section, WispDesign design, Action close, Func<bool, Task<string?>>? pickPath = null, string? sessionId = null,
        NativeLiveAppearanceModel? liveInputPreferences = null, Action<JsonObject>? applyInputPreferences = null)
        : this(new NativeSettingsEditorModel(client, project, sessionId), project, section, design, close, pickPath)
    {
        settingsClient = client;
        model.Localization = design.Localization;
        if (section == "usage") { usagePager = new(client, project); usagePager.Changed += UsageChanged; }
        if (section == "specialists") { specialistTransfer = new(client, project); specialistTransfer.Changed += RefreshSpecialistTransfer; }
        if (section == "general")
        {
            localEnvironment = new(client, project); localEnvironment.Changed += LockConfirmation;
            inputPreferences = liveInputPreferences ?? new(client, project);
            this.applyInputPreferences = applyInputPreferences;
            inputPreferences.Changed += RefreshInputPreferences;
            Notices.Children.Add(inputPreferenceNotice);
        }
        if (section == "network") { networkSettings = new(client, project); networkSettings.Changed += LockConfirmation; }
        if (section == "channels")
        {
            channelBinding = new(client, project, design.Localization); channelBinding.Changed += ChannelChanged;
            syncJoin = new(client, project, design.Localization); syncJoin.Changed += RefreshSyncJoin;
        }
        if (section == "models")
        {
            decisionSettings = new(client, project, JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", "decision-model.json")))!.AsObject());
            decisionSettings.Changed += LockConfirmation;
            subscriptionSettings = new(client, project); subscriptionSettings.Changed += LockConfirmation;
            subscriptionRoutes = new(client, project); subscriptionRoutes.Changed += LockConfirmation;
            subscriptionPoll = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(1500) };
            subscriptionPoll.Tick += async (_, _) =>
            {
                var before = subscriptionSettings.LoginStatus;
                await subscriptionSettings.PollAsync();
                if (!model.Closed && subscriptionSettings.FormOpen && before != subscriptionSettings.LoginStatus) Render();
            };
            subscriptionPoll.Start();
        }
        if (section is "general" or "network" or "session" or "pet" or "channels" or "permissions" or "credentials" or "environments" or "quick-actions" or "specialists" or "storage")
        {
            // These pages arrange the shared footer and need a mounted root
            // even when a cached/in-memory reply completes immediately.
            var started = false;
            Loaded += async (_, _) =>
            {
                if (started || model.Closed) return;
                started = true; await LoadInitialAsync();
            };
        }
        else _ = LoadInitialAsync();
    }
    private async Task LoadInitialAsync()
    {
        try { await Reload(); }
        catch (Exception ex) { if (!model.Closed) model.Fail(ex.Message); }
    }
    private NativeSettingsSectionPage(NativeSettingsEditorModel model, string? project, string section, WispDesign design, Action close, Func<bool, Task<string?>>? pickPath, bool modal = false)
        : base(design, SectionTitle(design, section), model, close, modal: modal)
    {
        this.model = model; this.projectScoped = project != null; settingsProjectId = project; this.section = section;
        this.pickPath = pickPath;
        UseSettingsLayout(section is "general" or "network", reserveScrollbar: section is "general" or "session", reserveOverflowScrollbar: NativeSettingsLayout.ReservesOverflowScrollbar(section)); Form.MaxWidth = double.PositiveInfinity; Form.Spacing = 18;
        if (section == "session")
        {
            // This page has one card and no results list. An empty results host
            // otherwise adds the body's 20px spacing after the card.
            HideResults();
            Body.Padding = new Thickness(24, 24, 24, 22);
        }
        SizeChanged += (_, _) => { ResizeGeneralGroups(); ResizeNetworkCard(); };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetLiveSetting(saveFeedback, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        preferenceFooter.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        preferenceFooter.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        preferenceFooter.RowDefinitions.Add(new() { Height = GridLength.Auto }); preferenceFooter.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var feedbackHost = new ScrollViewer { Content = saveFeedback, MaxHeight = 110, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalAlignment = VerticalAlignment.Center };
        preferenceFooter.Children.Add(feedbackHost); preferenceFooter.Children.Add(editorActions);
        preferenceFooter.SizeChanged += (_, e) =>
        {
            var stacked = e.NewSize.Width < 420;
            preferenceFooter.RowSpacing = stacked ? 8 : 0;
            Grid.SetColumn(editorActions, stacked ? 0 : 1); Grid.SetRow(editorActions, stacked ? 1 : 0);
            Grid.SetColumnSpan(feedbackHost, stacked ? 2 : 1);
            editorActions.Orientation = e.NewSize.Width < 180 ? Orientation.Vertical : Orientation.Horizontal;
        };
        Footer.Children.Add(preferenceFooter);
        cancelAuthorization.Click += async (_, _) => await model.CancelOAuthAsync();
        Notices.Children.Add(cancelAuthorization); Notices.Children.Add(confirmation); model.Changed += LockConfirmation;
    }
    private string[] Reads => section switch
    {
        "connections" => ["list_connectors", "list_mcp_connections"],
        "general" => ["get_settings", "get_bootstrap_status", "get_update_check_enabled"],
        "network" => ["get_network_settings"],
        "session" => ["get_settings"],
        "pet" => ["get_settings", "get_pet"],
        "browser" => ["get_browser_auto_launch", "get_browser_auto_close_tabs", "get_browser_url_filters"],
        "credentials" => ["credential_status", "list_custom_credentials"],
        "permissions" => ["list_approval_grants", "list_connectors"],
        "environments" => ["list_execution_contexts", "list_ssh_hosts", "get_default_execution_context", "list_ssh_trust_edges"],
        "usage" => ["get_token_usage"],
        "quick-actions" => ["list_quick_actions", "list_workflow_templates"],
        "specialists" => ["list_specialists", "list_models", "list_acp_agents", "list_skills", "get_subagent_model"],
        "workflows" or "workflow-conversion" => ["list_workflow_templates", "list_models", "list_skills"],
        "models" => ["list_models", "list_acp_agents"],
        "channels" => ["channels_status", "get_settings"],
        "project" => projectScoped ? ["get_project_settings", "get_project_run_retention"] : [],
        _ => throw new InvalidOperationException("Unknown settings section")
    };
    private readonly CancellationTokenSource sectionLifetime = new();
    private async Task Reload(bool reconnect = false)
    {
        if (section == "storage") { await RefreshStorageAsync(); return; }
        if (!reconnect && model.Draft != null && !model.InlinePreferences) return;
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(sectionLifetime.Token);
        deadline.CancelAfter(TimeSpan.FromSeconds(15));
        var loaded = reconnect ? await model.RetryLoadAsync(Reads, () => settingsClient is NativeSettingsClient host
            ? host.ReconnectAsync(deadline.Token) : Task.CompletedTask, deadline.Token) : await model.LoadAsync(Reads, deadline.Token);
        if (loaded && !model.Closed && inputPreferences?.Draft == null && inputPreferences != null)
        {
            try { await inputPreferences.LoadAsync(); }
            catch (Exception ex) { if (!model.Closed) model.Fail(model.ReadFailure(ex)); }
        }
        if (loaded && !model.Closed) localEnvironment?.AcceptSnapshot(model.Values["get_bootstrap_status"]);
        if (loaded && !model.Closed) networkSettings?.AcceptSnapshot(model.Values["get_network_settings"]);
        if (loaded && !model.Closed && !model.HasChanges) RestoreLanguage(includeWindowDraft: true);
        if (loaded && section == "general" && model.Values["get_settings"] is JsonObject prefs
            && prefs.ContainsKey("notification_sound_enabled")) await model.LoadNotificationSoundsAsync();
        if (loaded && section == "channels") await model.LoadChannelCapabilitiesAsync();
        if (loaded && section == "usage" && settingsClient != null)
        {
            // Optional display names must not hide valid usage when the catalog is unavailable.
            try { var profiles = await settingsClient.InvokeAsync("list_models", new(), settingsProjectId); if (!model.Closed) model.Values["list_models"] = profiles?.DeepClone(); }
            catch { /* Usage falls back to the recorded model IDs, like WebView. */ }
        }
        if (loaded && section == "models" && showDecision && decisionSettings != null) await decisionSettings.LoadAsync();
        if (loaded && section == "models" && showSubscriptions && subscriptionSettings != null && !subscriptionSettings.FormOpen) await LoadSubscriptions();
        if (!model.Closed) { Render(); if (loaded) OpenRequestedSpecialist(); }
    }
    private void Confirm(string text, Action action)
    {
        pendingConfirmation = action; confirmation.Children.Clear();
        confirmation.Children.Add(Warn(text));
        confirmation.Children.Add(Button(L("确认", "Confirm"), () => { var next = pendingConfirmation; ClearConfirmation(); next?.Invoke(); return Task.CompletedTask; }));
        confirmation.Children.Add(Button(T("settings.cancel"), () => { ClearConfirmation(); return Task.CompletedTask; }));
        LockConfirmation();
    }
    private void LockConfirmation()
    {
        UpdateAcpNotice();
        SetContentEnabled(!pageBusy && pendingConfirmation == null && specialistTransfer?.ModalOpen != true && syncJoin?.ModalOpen != true && model.EnvironmentEditor == null,
            resultsEnabled: localEnvironment?.Busy != true);
        RefreshEnvironmentModal();
        cancelAuthorization.Visibility = model.OAuthPending ? Visibility.Visible : Visibility.Collapsed;
        Notices.Visibility = pendingConfirmation != null || model.OAuthPending || inputPreferences?.Error != null ? Visibility.Visible : Visibility.Collapsed;
        Footer.Visibility = preferenceFooter.Parent == Footer && section is not ("quick-actions" or "specialists" or "channels" or "models") && (model.Draft != null && section != "connections" || model.SaveConfirmed) ? Visibility.Visible : Visibility.Collapsed;
        if (quickActionSave != null) quickActionSave.IsEnabled = !Busy && pendingConfirmation == null && model.Draft is { } action && NativeQuickActions.SaveBlockReason(action) == null;
        if (specialistSave != null) specialistSave.IsEnabled = !Busy && pendingConfirmation == null && model.Draft is { } specialist && NativeSpecialists.SaveBlockReason(specialist) == null;
        if (credentialAdd != null) credentialAdd.IsEnabled = model.CanAddCredential && pendingConfirmation == null;
        UpdateReviewerTest();
        refreshNetworkFeedback?.Invoke();
        UpdateDecisionFeedback();
        UpdateSubscriptionFeedback();
        UpdateSubscriptionRoutes();
        editorActions.Visibility = model.Draft != null ? Visibility.Visible : Visibility.Collapsed;
        foreach (var button in editorActions.Children.OfType<Button>()) button.IsEnabled = !Busy && pendingConfirmation == null;
        if (invalidFields.Count == 0) showInlineValidation = false;
        saveFeedback.Text = showInlineValidation ? L("请检查字段格式：", "Please check the field format: ") + string.Join(L("、", ", "), invalidFields)
            : model.Saving ? L("正在保存…", "Saving…") : model.SaveConfirmed
            ? model.Error == null ? L("已保存。", "Saved.") : L("上次设置已保存。请查看页面中的错误详情。", "Previous settings were saved. See the error above.")
            : model.Error ?? (GeneralInlineFooter ? T("settings.general.save_hint") : section is "session" or "pet" or "channels" ? "" : L("修改后点击保存。", "Save to apply your changes."));
    }
    private void ClearConfirmation() { pendingConfirmation = null; confirmation.Children.Clear(); LockConfirmation(); }
    public override void Dispose()
    {
        sectionLifetime.Cancel(); sectionLifetime.Dispose();
        storageScan?.Cancel();
        CloseUpdateDialog(restoreFocus: false);
        if (usagePager != null) { usagePager.Changed -= UsageChanged; usagePager.Dispose(); }
        petPreviewVersion++;
        connectionTransports.Clear(); discoveredTools.Clear();
        textMenus.Dispose();
        if (inputPreferences != null) inputPreferences.Changed -= RefreshInputPreferences;
        if (specialistTransfer != null) { specialistTransfer.Changed -= RefreshSpecialistTransfer; specialistTransfer.Dispose(); HideSpecialistTransfer(); }
        if (localEnvironment != null) { localEnvironment.Changed -= LockConfirmation; localEnvironment.Dispose(); }
        if (networkSettings != null) { networkSettings.Changed -= LockConfirmation; networkSettings.Dispose(); }
        if (decisionSettings != null) { decisionSettings.Changed -= LockConfirmation; decisionSettings.Dispose(); }
        subscriptionPoll?.Stop();
        if (subscriptionSettings != null) { subscriptionSettings.Changed -= LockConfirmation; subscriptionSettings.Dispose(); }
        if (subscriptionRoutes != null) { subscriptionRoutes.Changed -= LockConfirmation; subscriptionRoutes.Dispose(); }
        ClearCredentialControls();
        HideEnvironmentModal();
        ClearChannelSecrets(); authTerminal?.Dispose();
        if (channelBinding != null) { channelBinding.Changed -= ChannelChanged; channelBinding.Dispose(); }
        if (syncJoin != null) { syncJoin.Changed -= RefreshSyncJoin; syncJoin.Dispose(); HideSyncJoin(); }
        model.Changed -= LockConfirmation; base.Dispose();
    }
    public void RequestLeave(Action leave, bool switchingCategory = false, bool closingSettings = false)
    {
        if (updateDialog != null) { updateDialog.HandleEscape(); return; }
        if (model.EnvironmentEditor != null) { model.CancelEnvironmentEditor(); return; }
        if (syncJoin?.ModalOpen == true) { syncJoin.Dismiss(); return; }
        if (Busy || pendingConfirmation != null) return;
        if (specialistTransfer?.ModalOpen == true) { specialistTransfer.Cancel(); return; }
        var sharedForm = model.EditingWindowPreferences;
        var leavingSharedForm = sharedForm && (switchingCategory || closingSettings);
        async void Leave()
        {
            if (subscriptionSettings?.FormOpen == true && !await subscriptionSettings.CloseFormAsync()) return;
            if (model.Closed) return;
            if (switchingCategory && sharedForm && invalidFields.Count == 0) model.KeepWindowPreferences();
            model.DiscardCredentialInputs(); RestoreLanguage(includeWindowDraft: switchingCategory); localEnvironment?.Cancel(); networkSettings?.Discard(); decisionSettings?.Discard(); subscriptionRoutes?.Discard(); _ = LeaveChannelsAsync(leave);
        }
        if (leavingSharedForm && invalidFields.Count == 0 && localEnvironment?.HasChanges == true)
            Confirm(L("放弃尚未保存的路径修改？", "Discard unsaved path changes?"), Leave);
        else if (leavingSharedForm ? invalidFields.Count > 0 : HasChanges)
            Confirm(L("放弃尚未保存的修改？", "Discard unsaved changes?"), Leave);
        else Leave();
    }
    public override void HandleEscape()
    {
        if (updateDialog != null) { updateDialog.HandleEscape(); return; }
        if (textMenus.CloseTop()) return;
        if (syncJoin?.ModalOpen == true) { syncJoin.Dismiss(); return; }
        if (choices.LastOrDefault(c => c.IsDropDownOpen) is { } choice) { choice.IsDropDownOpen = false; return; }
        if (model.EnvironmentEditor != null) { model.CancelEnvironmentEditor(); return; }
        if (pendingConfirmation != null) { ClearConfirmation(); return; }
        if (section == "credentials" && credentialGroup != null) { BackToCredentials(); return; }
        if (section == "credentials") { RequestLeave(() => base.HandleEscape()); return; }
        if (section == "usage" && usagePager?.Selected != null) { BackToUsage(); return; }
        if (Busy) return;
        if (specialistTransfer?.ModalOpen == true) { specialistTransfer.Cancel(); return; }
        if (HandleChannelEscape()) return;
        if (subscriptionManual?.IsExpanded == true) { subscriptionManual.IsExpanded = false; return; }
        if (subscriptionSettings?.FormOpen == true) { RequestLeave(Render); return; }
        if (networkSettings?.MirrorsOpen == true) { CloseNetworkMirrors(); return; }
        if (section == "network") { RequestLeave(() => base.HandleEscape()); return; }
        if (localEnvironment?.Editing == true)
        {
            void CancelEnvironment() { localEnvironment.Cancel(); LocalEnvironment(); }
            if (localEnvironment.HasChanges) Confirm(L("放弃尚未保存的路径修改？", "Discard unsaved path changes?"), CancelEnvironment);
            else CancelEnvironment();
            return;
        }
        if (model.InlinePreferences) { RequestLeave(() => base.HandleEscape(), closingSettings: true); return; }
        if (section == "models" && (showDecision || showSubscriptions) && model.Draft == null) { RequestLeave(() => base.HandleEscape()); return; }
        if (model.Draft != null) { RequestLeave(() => { model.Discard(); Render(); }); return; }
        if (section == "connections" && connectionDetail != null) { BackToConnections(); return; }
        base.HandleEscape();
    }
    private async Task Run(string command, JsonObject args)
    {
        if (await model.InvokeAsync(command, args)) await Reload();
        else if (!model.Closed) Render();
    }
    private void Action(StackPanel target, string label, string command, JsonObject args, bool destructive = false)
        => target.Children.Add(Button(label, async () =>
        {
            if (destructive) Confirm(label + "？此操作不可撤销。", () => _ = Run(command, args));
            else await Run(command, args);
        }));
    private void Render()
    {
        if (model.Closed) return;
        acpNotice = null;
        petPreviewVersion++;
        MovePreferenceFooter(Footer);
        SettingsHeading(SectionTitle(Design, section));
        ClearCredentialControls();
        ClearChannelSecrets();
        authTerminal?.Dispose(); authTerminal = null;
        Form.Children.Clear(); Results.Children.Clear(); choices.Clear(); settingCards.Clear();
        refreshNetworkFeedback = null;
        inputShortcut = null; inputSelection = null;
        quickActionSave = null;
        specialistSave = null;
        reviewerTest = null; reviewerFeedback = null;
        decisionFeedback = null;
        subscriptionFeedback = null; subscriptionSave = null; subscriptionManual = null;
        subscriptionRouteSaves.Clear(); subscriptionRouteFeedback.Clear();
        transferNotice = null;
        editorActions.Children.Clear();
        HeaderActions.Children.Clear();
        if (model.LoadFailed)
        {
            Form.Children.Add(Button(L("重新连接并读取", "Reconnect and reload"), () => Reload(reconnect: true)));
            if (model.Values.Count == 0) { Update(); LockConfirmation(); return; }
        }
        var refresh = Design.ToolButton(L("刷新", "Refresh"), "refresh");
        refresh.Click += (_, _) => RequestLeave(() => { model.Discard(); _ = Reload(); });
        if (!model.LoadFailed && (section is not ("general" or "network" or "session" or "pet" or "browser" or "models" or "connections" or "storage" or "usage" or "permissions" or "credentials" or "environments" or "quick-actions" or "specialists") || model.Error != null || storageError != null))
            HeaderActions.Children.Add(refresh);
        switch (section)
        {
            case "connections": Connections(); break;
            case "general": General(); break;
            case "network": Network(); break;
            case "session": Session(); break;
            case "pet": Pet(); break;
            case "browser": Browser(); break;
            case "credentials": Credentials(); break;
            case "permissions": Permissions(); break;
            case "environments": Environments(); break;
            case "storage": Storage(); break;
            case "usage": Usage(); break;
            case "quick-actions": QuickActions(); break;
            case "specialists": Specialists(); break;
            case "workflows": Workflows(); break;
            case "workflow-conversion": WorkflowConversion(Rows(model.Values["list_workflow_templates"]).ToArray()); break;
            case "models": Models(); break;
            case "channels": Channels(); break;
            case "project": ProjectSettings(); break;
        }
        foreach (var card in settingCards) NativeActionWrap.GroupButtons(card);
        NativeActionWrap.GroupButtons(Form);
        PreferenceFieldAppearance.ApplyTree(Form, Design);
        PreferenceFieldAppearance.ApplyTree(Results, Design);
        Update();
        LockConfirmation();
    }
    private StackPanel Card(string title)
    {
        var card = new StackPanel { Spacing = 10 };
        var heading = Design.Text(title, 16); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        card.Children.Add(heading);
        var frame = Design.Card(card);
        frame.SetBinding(UIElement.VisibilityProperty, new Microsoft.UI.Xaml.Data.Binding {
            Source = card, Path = new PropertyPath("Visibility"), Mode = Microsoft.UI.Xaml.Data.BindingMode.OneWay });
        Results.Children.Add(frame); settingCards.Add(card); return card;
    }
    private void Toggle(StackPanel card, string label, bool value, string command, JsonObject args)
    {
        var control = new ToggleSwitch { Header = label, IsOn = value };
        control.Toggled += async (_, _) => { args["enabled"] = control.IsOn; await Run(command, args); };
        card.Children.Add(control);
    }
    private void Editor(string title, JsonObject draft, string command, string? parameter, Action<JsonObject> fields, JsonObject? extra = null, bool styleActions = true)
    {
        model.Edit(draft, command, parameter, extra);
        if (model.Draft == null) return;
        MovePreferenceFooter(Footer);
        ClearCredentialControls();
        ClearChannelSecrets();
        authTerminal?.Dispose(); authTerminal = null;
        invalidFields.Clear();
        Form.Children.Clear(); Results.Children.Clear(); choices.Clear();
        HeaderActions.Children.Clear();
        SettingsHeading(SectionTitle(Design, section) + " / " + title, () => RequestLeave(() => { model.Discard(); Render(); }));
        fields(model.Draft);
        editorActions.Children.Clear();
        editorActions.HorizontalAlignment = HorizontalAlignment.Right;
        Button ActionButton(string label, Func<Task> action) => styleActions ? Button(label, action) : ModelButton(label, action);
        editorActions.Children.Add(ActionButton(T("settings.cancel"), () => { RequestLeave(() => { model.Discard(); Render(); }); return Task.CompletedTask; }));
        editorActions.Children.Add(ActionButton(T("settings.save"), async () =>
        {
            if (invalidFields.Count > 0) { model.Fail("请检查字段格式：" + string.Join("、", invalidFields.Select(key => key.Contains(':') ? key[(key.IndexOf(':') + 1)..] : key).Distinct())); return; }
            if (model.Draft is { } proxyDraft && NativeProxyRoutes.ErrorKey(proxyDraft) is { } routeError) { model.Fail(T(routeError)); return; }
            if (await model.SaveAsync()) await Reload();
        }));
        BodyScroll.ChangeView(null, 0, null, true);
        Design.ApplyTypography(Form);
        LockConfirmation();
    }
    private void Text(JsonObject draft, string key, string title, bool multiline = false)
        => Field(title, S(draft, key), value => draft[key] = value, multiline);
    private void PathField(JsonObject draft, string key, string title, bool directory = false, bool either = false)
    {
        var owner = model.Draft;
        var input = Field(title, S(draft, key), value => draft[key] = value);
        if (pickPath == null) return;
        void Picker(bool folder) => Form.Children.Add(Button(folder ? "选择文件夹" : "选择文件", async () =>
        {
            var path = await pickPath(folder);
            if (!model.Closed && model.Draft == owner && path != null) input.Text = path;
        }));
        Picker(directory); if (either) Picker(!directory);
    }
    private void Lines(JsonObject draft, string key, string title)
        => Field(title + "（每行一项）", string.Join("\n", (draft[key] as JsonArray ?? []).Select(n => n?.GetValue<string>() ?? "")),
            value => draft[key] = new JsonArray(value.Split('\n').Select(s => s.Trim()).Where(s => s.Length > 0).Select(s => (JsonNode?)JsonValue.Create(s)).ToArray()), true);
    private void Choice(StackPanel target, string title, string value, (string Id, string Label)[] options, Action<string> changed)
    {
        var combo = new ComboBox { Header = title, HorizontalAlignment = HorizontalAlignment.Stretch };
        foreach (var option in options) combo.Items.Add(new ComboBoxItem { Content = option.Label, Tag = option.Id });
        combo.SelectedItem = combo.Items.Cast<ComboBoxItem>().FirstOrDefault(o => (string)o.Tag == value);
        combo.SelectionChanged += (_, _) => { if (combo.SelectedItem is ComboBoxItem item) changed((string)item.Tag); };
        Design.ApplyTypography(combo);
        choices.Add(combo); target.Children.Add(combo);
    }
}
