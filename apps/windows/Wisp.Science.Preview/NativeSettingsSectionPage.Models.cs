using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private bool showAgents;
    private bool showDecision;
    private bool showSubscriptions;
    private NativeSettingsAuthTerminal? authTerminal;
    private string? authTerminalId;
    private readonly NativeAcpChecks agentChecks = new();
    private NativeAcpInfo? authenticatedCheck;
    private Border? acpNotice;
    private bool acpSaved;
    private string? draggedModelId;
    private void Models()
    {
        SettingsContentWidth(920);
        Body.Padding = new Thickness(24, 14, 24, 22); Body.Spacing = 12; Form.Spacing = 12;
        if (subscriptionSettings?.FormOpen == true) { SubscriptionForm(); return; }
        var rows = Rows(model.Values["list_models"]).Where(row => !NativeSubscriptionSettingsModel.IsSubscription(row)).ToArray();
        var toolbar = new Grid { ColumnSpacing = 12 };
        toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto }); toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var tabs = new NativeActionWrap { Gap = 6 };
        void Tab(string title, bool agents, bool decision = false, bool subscriptions = false)
        {
            var tab = ModelButton(title, () =>
            {
                if (showAgents == agents && showDecision == decision && showSubscriptions == subscriptions) return Task.CompletedTask;
                RequestLeave(() =>
                {
                    acpSaved = false; model.Discard(); showAgents = agents; showDecision = decision; showSubscriptions = subscriptions; Render();
                    if (decision) _ = LoadDecisionSettings();
                    if (subscriptions) _ = LoadSubscriptions();
                });
                return Task.CompletedTask;
            });
            StyleModelButton(tab, 13, new Thickness(12, 7, 12, 7));
            if (showAgents != agents || showDecision != decision || showSubscriptions != subscriptions)
            {
                Design.QuietButton(tab); tab.Foreground = Design.Brush("text-muted");
                // QuietButton has smaller generic insets. Category tabs keep
                // identical geometry when selection moves between them.
                tab.Padding = new Thickness(12, 7, 12, 7); tab.MinHeight = 0;
                tab.BorderThickness = new Thickness(1); tab.BorderBrush = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
            }
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(tab, title);
            tabs.Children.Add(tab);
        }
        Tab($"{T("models.category.http")} ({rows.Length})", false);
        Tab($"{T("models.acp_open")} ({Rows(model.Values["list_acp_agents"]).Count()})", true);
        Tab(T("settings.nav.subscriptions"), false, subscriptions: true);
        Tab(T("models.category.decision"), false, true);
        toolbar.Children.Add(tabs);
        var add = ModelButton(T(showAgents ? "models.add_acp" : "models.add"), () =>
        {
            if (showAgents) EditAgent(new() { ["id"] = "", ["label"] = "", ["command"] = "", ["args"] = new JsonArray() });
            else AddProvider();
            return Task.CompletedTask;
        });
        StyleModelButton(add); Grid.SetColumn(add, 1); toolbar.Children.Add(add);
        add.Visibility = showDecision || showSubscriptions ? Visibility.Collapsed : Visibility.Visible;
        toolbar.SizeChanged += (_, e) =>
        {
            var narrow = e.NewSize.Width < 460;
            toolbar.RowSpacing = narrow ? 12 : 0;
            Grid.SetColumn(add, narrow ? 0 : 1); Grid.SetRow(add, narrow ? 1 : 0);
            Grid.SetColumnSpan(tabs, narrow || showDecision || showSubscriptions ? 2 : 1);
            add.HorizontalAlignment = HorizontalAlignment.Right;
        };
        Form.Children.Add(toolbar);
        if (showDecision) { DecisionSettings(); return; }
        if (showSubscriptions) { Subscriptions(); return; }
        var hint = ModelHint(T(showAgents ? "models.acp_subpage_hint" : "models.acp_hint")); hint.Margin = new Thickness(0, 12, 0, 8); Form.Children.Add(hint);
        if (showAgents) { if (!model.LoadFailed) Form.Children.Add(AcpNotice(list: true)); Agents(); return; }
        var presets = Rows(JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", "model-presets.json"))));
        var presetButtons = new NativeActionWrap { Margin = new Thickness(0, 2, 0, 12) };
        var presetLabel = ModelHint(T("models.quick_add")); presetLabel.VerticalAlignment = VerticalAlignment.Center;
        presetButtons.Children.Add(presetLabel);
        foreach (var preset in presets)
        {
            var button = ModelButton(S(preset, "label"), () =>
            { AddProvider(S(preset, "url")); return Task.CompletedTask; });
            StyleModelButton(button, 12.5, new Thickness(12, 4, 12, 4)); button.CornerRadius = new CornerRadius(14);
            presetButtons.Children.Add(button);
        }
        Form.Children.Add(presetButtons);
        var list = new StackPanel(); Form.Children.Add(new Border { Child = list, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
        foreach (var (row, index) in rows.Select((row, index) => (row, index)))
        {
            var line = new Grid { ColumnSpacing = 6, MinHeight = 48, Padding = new Thickness(2, 13, 2, 13), CanDrag = true, AllowDrop = true };
            line.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var main = new Grid { ColumnSpacing = 12 };
            main.ColumnDefinitions.Add(new() { Width = new GridLength(14) }); main.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            var grip = Design.Icon("grip-vertical", 14, Design.ColorText("text-faint")); grip.Opacity = 0; main.Children.Add(grip);
            var copy = new StackPanel { Spacing = 4 }; Grid.SetColumn(copy, 1); main.Children.Add(copy);
            var titleLine = new NativeActionWrap();
            var title = Design.Text(NativeModelPresentation.Label(row), 14); title.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            if (B(row, "active")) title.Foreground = Design.Brush("clay");
            titleLine.Children.Add(title);
            foreach (var (key, label) in new[] { ("use_for_vision", "models.capability.vision"), ("use_for_expert", "models.capability.expert"),
                ("use_for_image_generation", "models.capability.image"), ("use_for_video_generation", "models.capability.video") })
                if (B(row, key))
                {
                    var text = Design.Text(T(label), 11); text.Foreground = Design.Brush("text-muted");
                    titleLine.Children.Add(new Border { Child = text, Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(14), Padding = new Thickness(7, 1, 7, 1) });
                }
            copy.Children.Add(titleLine);
            var subtitle = NativeModelPresentation.Subtitle(row);
            if (subtitle.Length > 0) copy.Children.Add(ModelHint(subtitle));
            var edit = ModelButton(T("models.edit") + ": " + NativeModelPresentation.Label(row), () => { EditModel(row); return Task.CompletedTask; });
            edit.Content = main; Design.QuietButton(edit); edit.Padding = new Thickness(0); edit.MinHeight = 0;
            edit.HorizontalAlignment = HorizontalAlignment.Stretch; edit.HorizontalContentAlignment = HorizontalAlignment.Stretch; line.Children.Add(edit);
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
            Grid.SetColumn(actions, 1); line.Children.Add(actions);
            if (B(row, "active"))
            {
                var selected = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
                selected.Children.Add(Design.Icon("check", 14, Design.ColorText("clay"))); var text = Design.Text(T("models.default"), 12); text.Foreground = Design.Brush("clay"); selected.Children.Add(text);
                actions.Children.Add(new Border { Child = selected, Background = Design.Brush("clay-soft"), CornerRadius = new CornerRadius(14), Padding = new Thickness(10, 4, 10, 4) });
            }
            else
            {
                if (NativeModelPresentation.CanDelete(row, Rows(model.Values["list_models"])))
                {
                    var remove = Design.ToolButton(T("models.remove"), "close"); remove.Padding = new Thickness(4); remove.MinHeight = 0;
                    remove.Click += (_, _) => Confirm(Design.Localization.Format("models.remove_confirm", ("model", NativeModelPresentation.Label(row))),
                        () => _ = Run("remove_model", new() { ["id"] = S(row, "id") }));
                    actions.Children.Add(remove);
                }
                if (NativeModelPresentation.IsChat(row))
                {
                    var use = ModelButton(T("models.use"), () => Run("set_active_model", new() { ["id"] = S(row, "id") }));
                    StyleModelButton(use, 12, new Thickness(10, 6, 10, 6)); actions.Children.Add(use);
                }
            }
            var open = Design.ToolButton(T("models.edit") + ": " + NativeModelPresentation.Label(row), "chevron-right");
            open.Padding = new Thickness(0); open.MinHeight = 0; open.Click += (_, _) => EditModel(row); actions.Children.Add(open);
            line.PointerEntered += (_, _) => grip.Opacity = .55; line.PointerExited += (_, _) => grip.Opacity = 0;
            var frame = new Border { Child = line, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, index + 1 < rows.Length ? 1 : 0) };
            line.DragStarting += (_, e) =>
            {
                if (Busy) { e.Cancel = true; return; }
                draggedModelId = S(row, "id"); e.Data.SetText(draggedModelId); e.Data.RequestedOperation = Windows.ApplicationModel.DataTransfer.DataPackageOperation.Move;
            };
            line.DropCompleted += (_, _) => { draggedModelId = null; frame.Background = null; };
            line.DragOver += (_, e) =>
            {
                if (draggedModelId == null || Busy) return;
                e.AcceptedOperation = Windows.ApplicationModel.DataTransfer.DataPackageOperation.Move; frame.Background = Design.Brush("clay-soft"); e.Handled = true;
            };
            line.DragLeave += (_, _) => frame.Background = null;
            line.Drop += async (_, e) =>
            {
                var from = draggedModelId; draggedModelId = null; frame.Background = null; e.Handled = true;
                if (from == null || from == S(row, "id") || Busy) return;
                await Run("reorder_models", new() { ["ids"] = NativeModelDrafts.ReorderTo(Rows(model.Values["list_models"]), from, S(row, "id")) });
            };
            list.Children.Add(frame);
        }
        if (rows.Length == 0) Form.Children.Add(ModelHint(T("models.empty")));
    }
    private TextBlock ModelHint(string text)
    {
        var block = Design.Text(text, 12); block.Foreground = Design.Brush("text-faint");
        block.LineHeight = 17.4 * Design.Typography.UiSize / 14; return block;
    }
    private Button ModelButton(string title, Func<Task> action)
    {
        // Model pages choose their final style explicitly. The generic helper
        // infers a primary style from Chinese "保存", then a second style pass
        // replaces already-resolved resources and can throw 0x800F0902.
        var button = new Button { Content = Design.Text(title, 13) };
        Design.BindTypography(button);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, title);
        button.Click += async (_, _) => await action(); return button;
    }
    private void StyleModelButton(Button button, double size = 13.5, Thickness? padding = null, bool primary = false)
    {
        Design.ActionButton(button, primary); button.MinHeight = 0; button.Padding = padding ?? new Thickness(14, 8, 14, 8);
        button.Background = Design.Brush(primary ? "clay" : "bg-input");
        button.BorderBrush = Design.Brush(primary ? "clay" : "border-strong");
        if (button.Content is TextBlock text) Design.BindTypography(text, size);
    }
    private void EditModel(JsonObject source)
    {
        if (NativeSubscriptionSettingsModel.IsSubscription(source))
        {
            ModelEditor(T("models.edit"), (JsonObject)source.DeepClone(), "save_model", d =>
            {
                Form.Children.Add(ModelHint(T("subscriptions.managed")));
                ModelColumns(() => Text(d, "model", T("settings.model")), () => Text(d, "label", T("settings.label")));
                Form.Children.Add(ModelCheck(T("settings.use_for_expert"), B(d, "use_for_expert"), value => d["use_for_expert"] = value));
                Form.Children.Add(ModelHint(T("settings.expert_hint")));
            });
            return;
        }
        Action? refreshCatalog = null;
        ModelEditor(T(S(source, "id").Length == 0 ? "models.add" : "models.edit"), NativeModelDrafts.EditDraft(source), "save_model", d =>
        {
            var ready = false; Action refreshRoles = () => { }; Action refreshFast = () => { }; Action refreshHeaders = () => { }; Action refreshProxy = () => { };
            void IdentityChanged() { if (ready) { refreshRoles(); refreshFast(); refreshHeaders(); refreshProxy(); refreshCatalog?.Invoke(); } }
            Field(T("settings.api_url"), S(d, "api_url"), value => { d["api_url"] = value; IdentityChanged(); });
            Form.Children.Add(ModelHint(T("settings.tip")));
            Secure(d, "key", T("settings.api_key"));
            if (Form.Children.Last() is PasswordBox secret && B(d, "has_api_key")) secret.PlaceholderText = T("settings.stored_key");
            ModelColumns(
                () => Choice(Form, T("settings.provider"), NativeModelDrafts.ProviderValue(S(d, "provider")), [("openai", T("settings.provider.openai")),
                    ("openai_responses", T("settings.provider.openai_responses")), ("anthropic", T("settings.provider.anthropic"))],
                    value => { d["provider"] = value; IdentityChanged(); }),
                () => { var input = Field(T("settings.model"), S(d, "model"), value => { NativeModelDrafts.SelectModel(d, value); IdentityChanged(); }); input.PlaceholderText = T("settings.model_ph"); });
            ModelColumns(
                () => { var input = Field(T("settings.endpoint_suffix"), S(d, "endpoint_suffix"), value => { d["endpoint_suffix"] = value; IdentityChanged(); }); input.PlaceholderText = T("settings.endpoint_suffix_ph"); },
                () => { var input = Field(T("settings.label"), S(d, "label"), value => d["label"] = value); input.PlaceholderText = T("settings.label_ph"); });
            TextBox output = null!, context = null!; ComboBox effort = null!;
            var reasoningHint = ModelHint(""); var limitsHint = ModelHint(""); limitsHint.Visibility = Visibility.Collapsed;
            var chat = ModelFields(() =>
            {
                ModelColumns(
                    () => { Integer(d, "max_tokens", T("settings.max_tokens")); output = (TextBox)Form.Children.Last(); },
                    () => { Integer(d, "context_window", T("settings.context_window")); context = (TextBox)Form.Children.Last(); });
                Form.Children.Add(limitsHint);
                ModelColumns(() =>
                {
                    Choice(Form, T("settings.reasoning_effort"), S(d, "reasoning_effort"), [], value => d["reasoning_effort"] = value);
                    effort = (ComboBox)Form.Children.Last(); Form.Children.Add(reasoningHint);
                }, () => { });
            });
            Form.Children.Add(chat);
            var media = new StackPanel { Spacing = 12 }; Form.Children.Add(media);
            NativeModelMediaKind? previousMediaKind = null; var mediaChoices = new List<ComboBox>();
            var imageRoleHint = ModelHint(T("settings.image_role_hint")); Form.Children.Add(imageRoleHint);
            var restoreChat = ModelButton(T("settings.restore_chat_model"), () =>
            {
                NativeModelMedia.RestoreChat(d); IdentityChanged(); return Task.CompletedTask;
            }); StyleModelButton(restoreChat, 13, new Thickness(12, 8, 12, 8)); Form.Children.Add(restoreChat);
            var fastCopy = new StackPanel { Spacing = 3 }; var fastLabel = Design.Text(T("settings.service_tier"), 13);
            fastLabel.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; fastCopy.Children.Add(fastLabel);
            var fastState = Design.Text("", 11); fastState.Foreground = Design.Brush("text-faint"); fastCopy.Children.Add(fastState);
            var fastToggle = new ToggleSwitch { IsOn = S(d, "service_tier") is "priority" or "fast", OnContent = "", OffContent = "", MinWidth = 0 };
            StylePreferenceToggle(fastToggle);
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(fastToggle, T("settings.service_tier"));
            var fastGrid = new Grid { ColumnSpacing = 10, MinHeight = 58, Padding = new Thickness(12, 10, 12, 10) };
            fastGrid.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); fastGrid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); fastGrid.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            fastGrid.Children.Add(Design.Icon("bolt", 18)); Grid.SetColumn(fastCopy, 1); fastCopy.VerticalAlignment = VerticalAlignment.Center; fastGrid.Children.Add(fastCopy);
            Grid.SetColumn(fastToggle, 2); fastToggle.VerticalAlignment = VerticalAlignment.Center; fastGrid.Children.Add(fastToggle);
            var fastFrame = new Border { Child = fastGrid, CornerRadius = new CornerRadius(8), BorderThickness = new Thickness(1) };
            var fastHint = ModelHint(T("settings.service_tier.hint")); Form.Children.Add(fastFrame); Form.Children.Add(fastHint);
            refreshFast = () =>
            {
                var visible = NativeModelDrafts.ProviderValue(S(d, "provider")) is "openai" or "openai_responses" or "openai_codex";
                fastFrame.Visibility = fastHint.Visibility = visible && !NativeModelPresentation.IsImage(d) && !NativeModelPresentation.IsVideo(d) ? Visibility.Visible : Visibility.Collapsed;
                fastState.Text = T(fastToggle.IsOn ? "settings.service_tier.fast" : "settings.service_tier.default");
                fastFrame.Background = fastToggle.IsOn ? new Microsoft.UI.Xaml.Media.SolidColorBrush(Mix(Design.Brush("bg-elev").Color, Design.Brush("warn").Color, .1)) : Design.Brush("bg-elev");
                fastFrame.BorderBrush = fastToggle.IsOn ? Design.Brush("warn") : Design.Brush("border");
            };
            fastToggle.Toggled += (_, _) => { d["service_tier"] = fastToggle.IsOn ? "priority" : ""; refreshFast(); };
            var checks = new List<(string Key, CheckBox Check)>(); var syncing = false;
            CheckBox Role(string key, string label)
            {
                var check = ModelCheck(T(label), B(d, key), enabled =>
                {
                    if (syncing) return;
                    NativeModelDrafts.SetRole(d, key, enabled); refreshRoles(); refreshFast();
                }); checks.Add((key, check)); return check;
            }
            Border Capability(string hintKey, params CheckBox[] controls)
            {
                var content = new StackPanel { Spacing = 8 };
                var row = new NativeActionWrap(); foreach (var check in controls) row.Children.Add(check); content.Children.Add(row);
                var hint = ModelHint(T(hintKey)); hint.Margin = new Thickness(24, 0, 0, 0); content.Children.Add(hint);
                return new Border { Child = content, Padding = new Thickness(16, 14, 16, 14), Background = Design.Brush("bg-elev"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8) };
            }
            var expert = Capability("settings.expert_hint", Role("use_for_expert", "settings.use_for_expert")); Form.Children.Add(expert);
            var vision = Capability("settings.vision_hint", Role("supports_vision", "settings.supports_vision"), Role("use_for_vision", "settings.use_for_vision")); Form.Children.Add(vision);
            var chatRoles = ModelFields(() => ModelColumns(() => Form.Children.Add(Capability("settings.image_generation_hint", Role("use_for_image_generation", "settings.use_for_image_generation"))),
                () => Form.Children.Add(Capability("settings.video_generation_hint", Role("use_for_video_generation", "settings.use_for_video_generation"))))); Form.Children.Add(chatRoles);
            var imageRole = ModelFields(() => { Form.Children.Add(Role("use_for_image_generation", "settings.use_for_image_generation")); Form.Children.Add(ModelHint(T("settings.image_generation_hint"))); }); Form.Children.Add(imageRole);
            var videoRole = ModelFields(() => { Form.Children.Add(Role("use_for_video_generation", "settings.use_for_video_generation")); Form.Children.Add(ModelHint(T("settings.video_generation_hint"))); }); Form.Children.Add(videoRole);
            refreshRoles = () =>
            {
                syncing = true; foreach (var (key, check) in checks) check.IsChecked = B(d, key); syncing = false;
                var kind = NativeModelMedia.Kind(d); var isVideo = kind == NativeModelMediaKind.Video;
                var isImage = kind is NativeModelMediaKind.Image or NativeModelMediaKind.GrokImage;
                chat.Visibility = vision.Visibility = chatRoles.Visibility = isImage || isVideo ? Visibility.Collapsed : Visibility.Visible;
                expert.Visibility = isImage || isVideo || B(d, "use_for_video_generation") ? Visibility.Collapsed : Visibility.Visible;
                imageRoleHint.Visibility = imageRole.Visibility = isImage ? Visibility.Visible : Visibility.Collapsed;
                videoRole.Visibility = isVideo ? Visibility.Visible : Visibility.Collapsed;
                restoreChat.Visibility = NativeModelMedia.CanRestoreChat(d) ? Visibility.Visible : Visibility.Collapsed;
                if (kind != previousMediaKind)
                {
                    foreach (var choice in mediaChoices) choices.Remove(choice); mediaChoices.Clear(); media.Children.Clear();
                    var start = choices.Count; media.Children.Add(ModelMediaFields(d, kind)); mediaChoices.AddRange(choices.Skip(start)); previousMediaKind = kind;
                }
                media.Visibility = isImage || isVideo ? Visibility.Visible : Visibility.Collapsed;
                if (isImage || isVideo) { invalidFields.Remove(T("settings.max_tokens")); invalidFields.Remove(T("settings.context_window")); }
            };
            Form.Children.Add(ModelProxyRoute(d, out refreshProxy));
            Form.Children.Add(ModelRequestHeaders(d, out refreshHeaders));
            var restoreHint = ModelHint(T("settings.restore_chat_hint")); Form.Children.Add(restoreHint);
            var refreshMediaRoles = refreshRoles;
            refreshRoles = () => { refreshMediaRoles(); restoreHint.Visibility = B(d, "restore_chat_model") ? Visibility.Visible : Visibility.Collapsed; };
            refreshCatalog = () => _ = model.ReadModelCatalogAsync(catalog =>
            {
                var current = S(d, "reasoning_effort");
                var values = (catalog?["efforts"] as JsonArray)?.Select(value => value?.GetValue<string>() ?? "").Where(value => value.Length > 0).Distinct().ToArray() ?? [];
                effort.Items.Clear(); effort.Items.Add(new ComboBoxItem { Content = T("settings.reasoning_effort.default"), Tag = "" });
                if (current.Length > 0 && !values.Contains(current)) effort.Items.Add(new ComboBoxItem { Content = current + " (" + T("composer.effort.unverified") + ")", Tag = current, IsEnabled = false });
                foreach (var value in values) { var key = "composer.effort." + value; var label = T(key); effort.Items.Add(new ComboBoxItem { Content = label == key ? value : label, Tag = value }); }
                effort.SelectedItem = effort.Items.Cast<ComboBoxItem>().First(item => (string)item.Tag == current);
                reasoningHint.Text = catalog == null ? T("settings.reasoning_effort.unknown_hint") : values.Length == 0 ? T("settings.reasoning_effort.unsupported_hint")
                    : Design.Localization.Format("settings.reasoning_effort.known_hint", ("list", string.Join(" / ", values)));
                limitsHint.Visibility = catalog == null ? Visibility.Collapsed : Visibility.Visible;
                if (catalog != null)
                {
                    output.Text = catalog["max_tokens"]?.ToString() ?? output.Text; context.Text = catalog["context_window"]?.ToString() ?? context.Text;
                    limitsHint.Text = Design.Localization.Format("settings.catalog_limits_hint", ("context", context.Text), ("output", output.Text));
                }
            });
            ready = true; refreshRoles(); refreshFast();
        }, test: true);
        refreshCatalog?.Invoke();
    }
    private CheckBox ModelCheck(string label, bool value, Action<bool> changed)
    {
        var check = new CheckBox { Content = new Border { Child = Design.Text(label, 13), Margin = new Thickness(8, 0, 0, 0) }, IsChecked = value, MinWidth = 0, MinHeight = 0, Padding = new Thickness(0) };
        check.CornerRadius = new CornerRadius(2);
        check.VerticalContentAlignment = VerticalAlignment.Center;
        // The SDK template captures CheckBoxSize as a StaticResource, so a
        // per-control resource override does not resize it. Keep the native
        // checkbox semantics and states; only size its existing indicator.
        check.Loaded += (_, _) =>
        {
            check.ApplyTemplate();
            if (FindIndicator(check) is { Parent: Grid indicator } rectangle)
            {
                rectangle.Width = rectangle.Height = 16;
                indicator.Width = indicator.Height = 16;
            }
        };
        static FrameworkElement? FindIndicator(DependencyObject parent)
        {
            if (parent is FrameworkElement { Name: "NormalRectangle" } element) return element;
            for (var index = 0; index < Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChildrenCount(parent); index++)
                if (FindIndicator(Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChild(parent, index)) is { } found) return found;
            return null;
        }
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(check, label);
        check.Checked += (_, _) => changed(true); check.Unchecked += (_, _) => changed(false); return check;
    }
    private void ModelEditor(string title, JsonObject source, string command, Action<JsonObject> fields, bool test = false)
    {
        acpNotice = null; acpSaved = false;
        Editor(title, source, command, "profile", fields);
        if (model.Draft == null) return;
        SettingsContentWidth(1040); Body.Padding = new Thickness(24, 16, 24, 22); Body.Spacing = 12;
        SettingsBreadcrumb(SectionTitle(Design, "models"), title, () => RequestLeave(() => { model.Discard(); Render(); }));
        var content = ModelFields(() => { }, takeAll: true); content.Spacing = 10;
        if (command == "save_acp_agent") content.Children.Add(AcpNotice(list: false));
        var feedback = new StackPanel { Spacing = 6, Visibility = Visibility.Collapsed }; content.Children.Add(feedback);
        var actions = new NativeActionWrap { RightAligned = true };
        if (test) actions.Children.Add(ModelButton(T("settings.validate"), async () =>
        {
            if (invalidFields.Count > 0) { model.Fail(L("请先修正无效字段。", "Please correct the invalid fields.")); return; }
            if (model.ProviderErrorKey(save: false) is { } errorKey) { model.Fail(T(errorKey)); return; }
            if (model.Draft is { } proxyDraft && NativeProxyRoutes.ErrorKey(proxyDraft) is { } routeError) { model.Fail(T(routeError)); return; }
            if (await model.TestModelAsync(value => { feedback.Children.Clear(); Summary(feedback, value); feedback.Visibility = Visibility.Visible; })) LockConfirmation();
        }));
        actions.Children.Add(ModelButton(T("settings.cancel"), () => { model.Discard(); Render(); return Task.CompletedTask; }));
        var save = ModelButton(T(command == "save_acp_agent" ? "models.acp_save" : "settings.save"), async () =>
        {
            if (invalidFields.Count > 0) { model.Fail(L("请先修正无效字段。", "Please correct the invalid fields.")); return; }
            if (command == "save_acp_agent" && model.Draft is { } draft && !NativeModelDrafts.AcpValid(draft)) { model.Fail(T("models.acp_required")); return; }
            if (command == "save_model" && model.Draft is { } subscription && NativeSubscriptionSettingsModel.IsSubscription(subscription) && string.IsNullOrWhiteSpace(S(subscription, "model"))) { model.Fail(T("settings.model_ph")); return; }
            if (model.ProviderErrorKey() is { } errorKey) { model.Fail(T(errorKey)); return; }
            if (model.Draft is { } proxyDraft && NativeProxyRoutes.ErrorKey(proxyDraft) is { } routeError) { model.Fail(T(routeError)); return; }
            if (!await model.SaveAsync()) return;
            if (command == "save_acp_agent") { acpSaved = true; Render(); }
            else await Reload();
        }); actions.Children.Add(save);
        foreach (var button in actions.Children.OfType<Button>()) { StyleModelButton(button, primary: button == save); button.MinWidth = 84; }
        content.Children.Add(new Border { Child = actions, Margin = new Thickness(0, 4, 0, 0), Padding = new Thickness(0, 14, 0, 0), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
        Form.Children.Add(new Border { Child = content, Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Padding = new Thickness(16, 14, 16, 14), Margin = new Thickness(0, 4, 0, 0) });
        PreferenceFieldAppearance.ApplyTree(Form, Design);
        editorActions.Children.Clear(); LockConfirmation();
    }
    private StackPanel ModelFields(Action fields, bool takeAll = false, bool resizeArguments = false)
    {
        var start = takeAll ? 0 : Form.Children.Count; fields();
        var group = new StackPanel { Spacing = 12 };
        while (Form.Children.Count > start)
        {
            var field = Form.Children[start]; Form.Children.RemoveAt(start);
            StackPanel? labeled = null;
            object? header = field switch { TextBox text => text.Header, PasswordBox password => password.Header, ComboBox combo => combo.Header, _ => null };
            if (header is string caption)
            {
                var label = Design.Text(caption, 12); label.Foreground = Design.Brush("text-muted");
                // Authored labels share the CSS five-pixel gap rather than
                // accumulating the native template's larger header margins.
                switch (field) { case TextBox text: text.Header = null; break; case PasswordBox password: password.Header = null; break; case ComboBox combo: combo.Header = null; break; }
                labeled = new StackPanel { Spacing = 5 }; labeled.Children.Add(label);
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(field, caption);
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetLabeledBy(field, label);
            }
            if (field is TextBox input) { textMenus.Register(input); if (!input.AcceptsReturn) input.TextWrapping = TextWrapping.NoWrap; }
            if (field is TextBox or PasswordBox or ComboBox)
            {
                var control = (Control)field; Design.BindTypography(control, size: 14);
                control.CornerRadius = new CornerRadius(8); control.BorderBrush = Design.Brush("border-strong");
                control.Background = Design.Brush("bg-input"); control.BorderThickness = new Thickness(1); control.MinWidth = 0;
                control.Padding = new Thickness(11, 9, field is ComboBox ? 0 : 11, 9);
                if (field is TextBox { AcceptsReturn: true }) { control.CornerRadius = new CornerRadius(2); control.Padding = new Thickness(2); }
            }
            if (resizeArguments && field is TextBox { AcceptsReturn: true } args) field = ResizableAcpArguments(args);
            if (labeled != null) { labeled.Children.Add(field); group.Children.Add(labeled); }
            else group.Children.Add(field);
        }
        return group;
    }
    private void ModelColumns(params Action[] fields)
    {
        var grid = new Grid { ColumnSpacing = 14 };
        for (var i = 0; i < fields.Length; i++)
        {
            grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var group = ModelFields(fields[i]); FrameworkElement column = group;
            if (group.Children.Count == 1 && group.Children[0] is Border card) { group.Children.Clear(); column = card; }
            Grid.SetColumn(column, i); grid.Children.Add(column);
        }
        void Arrange()
        {
            var narrow = !NativeSettingsLayout.ModelFormColumns(XamlRoot?.Size.Width ?? ActualWidth);
            grid.RowSpacing = narrow ? 12 : 0;
            for (var i = 0; i < grid.Children.Count; i++)
            { var child = (FrameworkElement)grid.Children[i]; Grid.SetColumn(child, narrow ? 0 : i); Grid.SetRow(child, narrow ? i : 0); Grid.SetColumnSpan(child, narrow ? fields.Length : 1); }
        }
        grid.SizeChanged += (_, _) => Arrange(); grid.Loaded += (_, _) => Arrange(); Form.Children.Add(grid);
    }
    private void Agents()
    {
        var agents = Rows(model.Values["list_acp_agents"]).ToArray();
        agentChecks.Retain(agents);
        if (agents.Length == 0)
        {
            var empty = ModelHint(T("models.empty_acp")); Design.BindTypography(empty, size: 13);
            empty.LineHeight = 19.5 * Design.Typography.UiSize / 14;
            Form.Children.Add(new Border { Child = empty, Margin = new Thickness(0, -12, 0, 0), Padding = new Thickness(12, 16, 12, 16), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
        }
        var list = new StackPanel { Margin = new Thickness(0, -12, 0, 0) }; Form.Children.Add(list);
        foreach (var agent in agents)
        {
            var id = S(agent, "id"); var line = new Grid { ColumnSpacing = 12, Padding = new Thickness(2, 12, 2, 10), MinHeight = 48 };
            line.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var copy = new StackPanel { Spacing = 2 };
            var title = Design.Text(S(agent, "label"), 14); title.FontWeight = Microsoft.UI.Text.FontWeights.Medium; copy.Children.Add(title);
            copy.Children.Add(ModelHint(S(agent, "command")));
            var edit = ModelButton(T("models.edit_acp") + ": " + S(agent, "label"), () => { EditAgent(agent); return Task.CompletedTask; });
            Design.QuietButton(edit); edit.Content = copy; edit.Padding = new Thickness(0); edit.HorizontalAlignment = HorizontalAlignment.Stretch; edit.HorizontalContentAlignment = HorizontalAlignment.Stretch;
            line.Children.Add(edit);
            var actions = new NativeActionWrap { VerticalAlignment = VerticalAlignment.Center, Gap = 6 };
            Grid.SetColumn(actions, 1); line.Children.Add(actions);
            var details = new NativeActionWrap { Gap = 8, RowGap = 8, VerticallyCentered = true, Margin = new Thickness(2, 0, 2, 12) };
            var test = ModelButton(T("models.acp_test"), async () =>
            {
                acpSaved = false;
                if (await model.InvokeAsync("test_acp_agent", new() { ["id"] = id }, reply => agentChecks.Store(agent, reply))) RefreshDetails();
            }); StyleAcpButton(test); actions.Children.Add(test);
            var remove = Design.ToolButton(T("models.remove"), "close"); remove.Padding = new Thickness(4); remove.MinHeight = 0;
            remove.Click += (_, _) => Confirm(Design.Localization.Format("models.remove_confirm", ("model", S(agent, "label"))), () => _ = Run("remove_acp_agent", new() { ["id"] = id })); actions.Children.Add(remove);
            var open = Design.ToolButton(T("models.edit_acp"), "chevron-right"); open.Padding = new Thickness(0); open.MinHeight = 0; open.Click += (_, _) => EditAgent(agent); actions.Children.Add(open);
            var clear = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
            var entry = new StackPanel { Background = clear }; entry.Children.Add(line);
            bool over = false, focused = false;
            void Highlight() { remove.Opacity = over || focused ? 1 : 0; entry.Background = over ? Design.Brush("bg-elev") : clear; }
            remove.Opacity = 0;
            entry.PointerEntered += (_, _) => { over = true; Highlight(); };
            entry.PointerExited += (_, _) => { over = false; Highlight(); };
            entry.GotFocus += (_, _) => { focused = true; Highlight(); };
            entry.LostFocus += (_, _) => { focused = false; Highlight(); };
            list.Children.Add(new Border { Child = entry, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) });
            entry.Children.Add(details); RefreshDetails();
            void RefreshDetails()
            {
                details.Children.Clear();
                var info = agentChecks.For(agent);
                details.Visibility = info == null ? Visibility.Collapsed : Visibility.Visible;
                if (info == null) return;
                details.Children.Add(Design.Text(info.Version, 12, "text-muted"));
                AddAcpMethods(details, id, info);
            }
        }
        if (authTerminalId != null && settingsClient != null && settingsProjectId != null)
        {
            authTerminal = new NativeSettingsAuthTerminal(new NativeAuthTerminalModel(settingsClient, settingsProjectId, authTerminalId), Design, () =>
            { authTerminalId = null; authTerminal?.Dispose(); authTerminal = null; Render(); });
            Results.Children.Add(authTerminal);
        }
    }

    private void StyleAcpButton(Button button)
    {
        StyleModelButton(button, 12, new Thickness(11, 5, 11, 5));
        // WinUI stretches oversized radii into an ellipse; use half the rendered
        // height so wide labels keep the straight edges of the WebView pill.
        button.SizeChanged += (_, e) => button.CornerRadius = new CornerRadius(e.NewSize.Height / 2);
        button.Resources["ButtonBackgroundPointerOver"] = Design.Brush("clay-soft");
        button.Resources["ButtonBorderBrushPointerOver"] = Design.Brush("clay");
        if (button.Content is TextBlock caption)
        {
            caption.Foreground = Design.Brush("text-muted");
            button.PointerEntered += (_, _) => caption.Foreground = Design.Brush("clay");
            button.PointerExited += (_, _) => caption.Foreground = Design.Brush("text-muted");
        }
    }

    private void AddAcpMethods(NativeActionWrap details, string id, NativeAcpInfo info)
    {
        foreach (var method in info.Methods)
        {
            var authorize = ModelButton(method.Name, async () =>
            {
                JsonNode? result = null;
                if (!await model.InvokeAsync("authenticate_acp_agent", new() { ["id"] = id, ["methodId"] = method.Id }, value => result = value)) return;
                if (result == null) { authenticatedCheck = info; Render(); return; }
                var terminalId = S(result, "id");
                if (terminalId.Length == 0) { model.Fail(Design.L("认证终端未返回有效 ID。", "The authentication terminal did not return an ID.")); return; }
                authTerminalId = terminalId; Render();
            });
            StyleAcpButton(authorize);
            authorize.IsEnabled = method.Kind != "env_var" && authTerminalId == null && (method.Kind != "terminal" || projectScoped);
            var hint = method.Kind == "env_var" ? Design.L("此认证方式需要环境变量。", "This method requires an environment variable.")
                : method.Kind == "terminal" && !projectScoped ? Design.L("请选择项目后打开认证终端。", "Select a project to open an authentication terminal.") : method.Description;
            if (hint.Length > 0) ToolTipService.SetToolTip(authorize, hint);
            details.Children.Add(authorize);
        }
        if (ReferenceEquals(authenticatedCheck, info)) details.Children.Add(ModelHint(T("models.acp_auth_ok")));
    }
    private Border AcpNotice(bool list)
    {
        var note = Design.Text("", 12);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetLiveSetting(note, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        acpNotice = new Border { Child = note, BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8),
            Padding = new Thickness(10, 8, 10, 8), Margin = new Thickness(0, list ? -12 : 0, 0, 0), Visibility = Visibility.Collapsed };
        UpdateAcpNotice();
        return acpNotice;
    }
    private void UpdateAcpNotice()
    {
        if (acpNotice?.Child is not TextBlock note) return;
        var message = model.Error ?? (acpSaved ? T("models.acp_saved") : "");
        acpNotice.Visibility = message.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        if (message.Length == 0) return;
        var color = Design.Brush(model.Error == null ? "ok" : "err");
        Microsoft.UI.Xaml.Media.SolidColorBrush Mix(string background, double weight)
        {
            var a = color.Color; var b = Design.Brush(background).Color;
            byte Channel(byte x, byte y) => (byte)Math.Round(x * weight + y * (1 - weight));
            return new(Windows.UI.Color.FromArgb(255, Channel(a.R,b.R), Channel(a.G,b.G), Channel(a.B,b.B)));
        }
        note.Text = message; note.Foreground = color;
        acpNotice.BorderBrush = Mix("border-strong", .35); acpNotice.Background = Mix("bg-input", .08);
    }
    private void EditAgent(JsonObject agent) => ModelEditor(T(S(agent, "id").Length == 0 ? "models.add_acp" : "models.edit_acp"), agent, "save_acp_agent", d =>
    {
        var fields = ModelFields(() =>
        {
            Text(d, "label", T("models.acp_label")); Text(d, "command", T("models.acp_command"));
            var args = Field(T("models.acp_args"), string.Join("\n", (d["args"] as JsonArray ?? []).Select(value => value?.GetValue<string>() ?? "")),
                value => d["args"] = NativeModelDrafts.AcpArguments(value), multiline: true);
            args.MinHeight = 32; args.Height = 76;
        }, resizeArguments: true);
        fields.Spacing = 10;
        Form.Children.Add(fields);
        Form.Children.Add(ModelHint(T("models.acp_subpage_hint")));
    });
    private Grid ResizableAcpArguments(TextBox input)
    {
        var slot = new Grid();
        var box = new Grid(); slot.Children.Add(box); box.Children.Add(input);
        var grip = Design.Icon("expand", 10);
        grip.HorizontalAlignment = HorizontalAlignment.Right; grip.VerticalAlignment = VerticalAlignment.Bottom;
        grip.Margin = new Thickness(3); grip.Opacity = .55; grip.IsHitTestVisible = false; box.Children.Add(grip);
        var thumb = new Microsoft.UI.Xaml.Controls.Primitives.Thumb { Width = 16, Height = 16, IsTabStop = true,
            HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Bottom,
            Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(thumb, L("调整参数框大小；使用方向键调整", "Resize arguments; use the arrow keys"));
        box.Children.Add(thumb);
        double? wantedWidth = null;
        void Layout()
        {
            box.HorizontalAlignment = wantedWidth == null ? HorizontalAlignment.Stretch : HorizontalAlignment.Left;
            box.Width = wantedWidth == null ? double.NaN : Math.Min(wantedWidth.Value, slot.ActualWidth);
        }
        void Resize(double x, double y)
        {
            if (slot.ActualWidth <= 0) return;
            var next = NativeSettingsLayout.ResizeArgumentBox(box.ActualWidth, input.Height, x, y, slot.ActualWidth);
            if (x != 0) wantedWidth = next.Width >= slot.ActualWidth ? null : next.Width;
            input.Height = next.Height; Layout();
        }
        thumb.DragDelta += (_, e) => Resize(e.HorizontalChange, e.VerticalChange);
        thumb.KeyDown += (_, e) =>
        {
            switch (e.Key)
            {
                case Windows.System.VirtualKey.Left: Resize(-16, 0); break;
                case Windows.System.VirtualKey.Right: Resize(16, 0); break;
                case Windows.System.VirtualKey.Up: Resize(0, -16); break;
                case Windows.System.VirtualKey.Down: Resize(0, 16); break;
                default: return;
            }
            e.Handled = true;
        };
        slot.SizeChanged += (_, _) => Layout();
        return slot;
    }
}
