using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    internal static IReadOnlyDictionary<string, string> Titles { get; } = new Dictionary<string, string>
    {
        ["general"] = "常规", ["network"] = "网络", ["session"] = "对话", ["pet"] = "宠物",
        ["skills"] = "技能", ["connections"] = "连接", ["memory"] = "记忆", ["hooks"] = "钩子",
        ["skill-store"] = "社区技能", ["skill-github"] = "从 GitHub 添加",
        ["plugins"] = "插件", ["browser"] = "浏览器", ["credentials"] = "凭据", ["permissions"] = "权限",
        ["environments"] = "环境", ["storage"] = "存储", ["usage"] = "用量",
        ["quick-actions"] = "快捷动作", ["specialists"] = "专家", ["workflows"] = "工作流", ["workflow-conversion"] = "从技能创建工作流", ["models"] = "模型", ["channels"] = "远程接入", ["project"] = "项目设置"
    };
    private readonly HashSet<string> invalidFields = [];
    private void Boolean(JsonObject draft, string key, string label)
    {
        var toggle = new ToggleSwitch { Header = label, IsOn = B(draft, key) };
        toggle.Toggled += (_, _) => draft[key] = toggle.IsOn; Form.Children.Add(toggle);
    }
    private void Secure(JsonObject draft, string key, string label)
    {
        var input = new PasswordBox { Header = label, Password = S(draft, key) };
        input.PasswordChanged += (_, _) => draft[key] = input.Password; Form.Children.Add(input);
    }
    private void Integer(JsonObject draft, string key, string label, long minimum = 0, long maximum = int.MaxValue, bool nullable = false)
    {
        Field(label, draft[key]?.ToString() ?? "", value =>
        {
            if (nullable && string.IsNullOrWhiteSpace(value)) { draft[key] = null; invalidFields.Remove(label); }
            else if (long.TryParse(value, out var number) && number >= minimum && number <= maximum)
            { draft[key] = number; invalidFields.Remove(label); }
            else invalidFields.Add(label);
        });
    }
    private void Preference(string title, string read, string write, string? parameter, Action<JsonObject> fields)
    {
        var card = Card(title);
        if (model.Values[read] is not JsonObject prefs) { card.Children.Add(Mute("尚未读取，点击刷新重试。")); return; }
        var description = title switch
        {
            "工作区与通知" => $"{(S(prefs, "locale") == "en" ? "English" : "简体中文")} · 桌面通知{(B(prefs, "notifications_enabled") ? "已开启" : "已关闭")} · {(B(prefs, "resume_last_session") ? "启动时恢复最近会话" : "启动时显示项目")}",
            "输入与选择" => "配置消息发送快捷键和选中文本后的快捷菜单。",
            "网络与软件源" => "分别配置模型、订阅登录和工具代理，以及 Python / Conda 软件源。",
            "对话运行限制" => $"自动压缩上下文{(B(prefs, "auto_compact") ? "已开启" : "已关闭")} · 后续问题建议{(B(prefs, "follow_up_questions") ? "已开启" : "已关闭")}",
            "桌宠" => B(prefs, "pet_enabled") ? "桌宠已开启，可调整资源目录。" : "桌宠已关闭，可在编辑中启用并选择资源目录。",
            "URL 访问规则" => $"阻止访问 {(prefs["block"] as JsonArray)?.Count ?? 0} 项 · 优先访问 {(prefs["prefer"] as JsonArray)?.Count ?? 0} 项",
            "项目保留策略" => "设置成功运行、失败运行和孤立文件的保留期限。",
            _ => ""
        };
        if (description.Length > 0) card.Children.Add(Mute(description));
        card.Children.Add(Button(L("编辑", "Edit ") + title, () => { Editor(title, prefs, write, parameter, fields); return Task.CompletedTask; }));
    }
    private void Immediate(StackPanel card, string label, string read, string write)
    {
        if (model.Values[read] is JsonValue value && value.TryGetValue<bool>(out var enabled))
            Toggle(card, label, enabled, write, new());
    }
    private Grid? generalGroups;
    private Border? generalNotifications;
    private Border? generalPreferenceFrame, generalEnvironmentFrame;
    private NativeLocalEnvironmentModel? localEnvironment;
    private void LocalEnvironment()
    {
        if (localEnvironment is not { } environment) return;
        var local = new StackPanel { Spacing = 16 };
        var title = Design.Text(T("env.title"), 15); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        local.Children.Add(title);
        var description = Mute(T("env.optional")); Design.BindTypography(description, 12);
        description.Margin = new Thickness(0, -4, 0, 0); description.LineHeight = Design.Typography.Scale(12) * 1.6;
        local.Children.Add(description);
        var rows = new StackPanel();
        foreach (var (label, key) in NativeLocalEnvironmentModel.Fields)
        {
            var row = new Grid { ColumnSpacing = 8, Padding = new Thickness(12, 9, 12, 9) };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(70) }); row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            var caption = Mute(label); Design.BindTypography(caption, 12); caption.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            caption.VerticalAlignment = VerticalAlignment.Center; row.Children.Add(caption);
            FrameworkElement value;
            if (environment.Draft is { } draft)
            {
                var input = new TextBox { Text = S(draft, key), PlaceholderText = T("env.missing"), CornerRadius = new CornerRadius(10),
                    MinHeight = 0, Padding = new Thickness(8, 6, 8, 6),
                    Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong") };
                Design.BindTypography(input, code: true, size: 11);
                PreferenceFieldAppearance.UsePlainTextField(input);
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(input, label);
                textMenus.Register(input);
                input.TextChanged += (_, _) => { draft[key] = input.Text; LockConfirmation(); };
                value = input;
            }
            else
            {
                var text = Design.Text(S(environment.Paths, key) is { Length: > 0 } path ? path : T(environment.Paths == null ? "env.checking" : "env.missing"), 11);
                text.FontFamily = Design.Font(true); text.LineHeight = Design.Typography.Scale(11) * 1.6;
                text.IsTextSelectionEnabled = true; value = text;
            }
            Grid.SetColumn(value, 1); row.Children.Add(value);
            rows.Children.Add(new Border { Child = row, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, key == NativeLocalEnvironmentModel.Fields[^1].Key ? 0 : 1) });
        }
        local.Children.Add(new Border { Child = rows, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8) });
        if (environment.Warning is { Length: > 0 } warning) local.Children.Add(Mute(warning));
        if (environment.Error is { } error) local.Children.Add(Warn(error));
        if (environment.Saved) local.Children.Add(Mute(T("env.saved")));
        var actions = new NativeActionWrap();
        if (environment.Editing)
        {
            local.Children.Add(Mute(T("env.edit_hint")));
            actions.Children.Add(Button(T("env.save"), async () => { await environment.SaveAsync(); if (!model.Closed) LocalEnvironment(); }));
            actions.Children.Add(Button(T("settings.cancel"), () => { environment.Cancel(); LocalEnvironment(); return Task.CompletedTask; }));
        }
        else
        {
            actions.Children.Add(Button(T("env.check_again"), async () => { await environment.DetectAsync(); if (!model.Closed) LocalEnvironment(); }));
            var edit = Button(T("env.edit"), () => { environment.Edit(); LocalEnvironment(); return Task.CompletedTask; });
            edit.IsEnabled = edit.IsEnabled && environment.CanEdit; actions.Children.Add(edit);
        }
        foreach (var button in actions.Children.OfType<Button>())
        {
            button.MinHeight = 0; button.Padding = new Thickness(12, 6, 12, 6); button.CornerRadius = new CornerRadius(8);
            if (button.Content is TextBlock label) Design.BindTypography(label, 13.5);
        }
        local.Children.Add(actions);
        // The environment request owns only this card. Updating the whole page
        // would replace the workspace input and lose the user's focus/selection.
        if (generalEnvironmentFrame?.Parent == Results) generalEnvironmentFrame.Child = local;
        else { generalEnvironmentFrame = Design.Card(local); Results.Children.Add(generalEnvironmentFrame); }
        Design.ApplyTypography(local); PreferenceFieldAppearance.ApplyTree(local, Design); ResizeGeneralGroups();
    }
    private void ResizeGeneralGroups()
    {
        if (generalGroups == null || generalNotifications == null) return;
        var compact = (XamlRoot?.Size.Width ?? ActualWidth) <= 850;
        var padding = compact ? new Thickness(16, 18, 16, 18) : new Thickness(24, 22, 24, 22);
        if (generalPreferenceFrame != null) generalPreferenceFrame.Padding = padding;
        if (generalEnvironmentFrame != null) generalEnvironmentFrame.Padding = padding;
        var columns = NativeSettingsLayout.GeneralColumns(ActualWidth);
        generalGroups.ColumnSpacing = columns ? 28 : 0;
        generalGroups.ColumnDefinitions[1].Width = columns ? new GridLength(1, GridUnitType.Star) : new GridLength(0);
        Grid.SetColumn(generalNotifications, columns ? 1 : 0);
        Grid.SetRow(generalNotifications, columns ? 0 : 1);
        generalNotifications.BorderThickness = columns ? new Thickness(1, 0, 0, 0) : new Thickness(0, 1, 0, 0);
        generalNotifications.Padding = columns ? new Thickness(28, 0, 0, 0) : new Thickness(0, 20, 0, 0);
        generalNotifications.Margin = columns ? new Thickness(0) : new Thickness(0, 20, 0, 0);
    }
    private void General()
    {
        if (model.Values["get_settings"] is not JsonObject) return;
        model.EditWindowPreferences(WindowPreferences);
        var draft = model.Draft!;
        var prefs = draft["get_settings"]!.AsObject();
        var card = new StackPanel();
        var workspaceHeading = Design.Text(T("settings.general.workspace"), 15);
        workspaceHeading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; workspaceHeading.Margin = new Thickness(0, 0, 0, 16);
        card.Children.Add(workspaceHeading);
        generalGroups = new Grid();
        generalGroups.ColumnDefinitions.Add(new() { Width = new GridLength(1.2, GridUnitType.Star) });
        generalGroups.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        generalGroups.RowDefinitions.Add(new() { Height = GridLength.Auto }); generalGroups.RowDefinitions.Add(new() { Height = GridLength.Auto });
        generalGroups.Children.Add(card);
        var preferences = new StackPanel(); preferences.Children.Add(generalGroups);
        var frame = Design.Card(preferences); frame.Padding = new Thickness(24, 22, 24, 22); Form.Children.Add(frame);
        generalPreferenceFrame = frame;
        InlineChoice(card, T("settings.language"), S(prefs, "locale"), [("en", "English"), ("zh", T("settings.language.zh"))], v =>
        {
            prefs["locale"] = v;
            if (Design.Localization.Preview(v)) { LanguageChanged?.Invoke(); Render(); }
        });
        InlineText(card, prefs, "workspace_dir", T("settings.workspace_dir"), S(model.Values["get_bootstrap_status"], "workspace"));
        InlineToggle(card, T("settings.decentralized_project_storage"), T("settings.decentralized_project_storage_hint"), B(prefs, "decentralized_project_storage"), v => prefs["decentralized_project_storage"] = v);
        InlineToggle(card, T("settings.resume_last_session"), T("settings.resume_last_session_hint"), B(prefs, "resume_last_session"), v => prefs["resume_last_session"] = v);
        if (inputPreferences?.Draft is JsonObject appearance)
        {
            var interaction = NativeInputPreferences.From(appearance);
            inputShortcut = InlineChoice(card, T("settings.send_shortcut"), interaction.SendWithModifier ? "modifier" : "enter",
                [("enter", T("settings.send_shortcut.enter")), ("modifier", Design.Localization.Format("settings.send_shortcut.modifier_enter", ("modifier", "Ctrl")))], v => ChangeInputPreference("send_with_modifier", v == "modifier"));
            inputSelection = InlineToggle(card, T("settings.selection_popup"), T("settings.selection_popup_hint"), interaction.SelectionPopupEnabled,
                v => ChangeInputPreference("selection_popup_enabled", v), divider: false);
        }
        RefreshInputPreferences();
        card = new StackPanel();
        var notificationHeading = Design.Text(T("settings.general.notifications"), 15);
        notificationHeading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; notificationHeading.Margin = new Thickness(0, 0, 0, 16);
        card.Children.Add(notificationHeading);
        generalNotifications = new Border { Child = card, BorderBrush = Design.Brush("border") };
        generalGroups.Children.Add(generalNotifications); ResizeGeneralGroups();
        InlineToggle(card, T("settings.notifications"), T("settings.notifications_hint"), B(prefs, "notifications_enabled"), v => prefs["notifications_enabled"] = v);
        if (prefs.ContainsKey("notification_sound_enabled"))
        {
            var catalog = model.Values["list_notification_sounds"];
            var sounds = NativeNotificationSounds.Choices(catalog, S(prefs, "notification_sound"), Design.Localization.Locale);
            var picker = new Grid { Margin = new Thickness(0, 10, 0, 0), ColumnSpacing = 8 };
            picker.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star), MaxWidth = 220 });
            picker.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            if (sounds.Length > 0)
            {
                var combo = new ComboBox { MaxWidth = 220, HorizontalAlignment = HorizontalAlignment.Stretch,
                    Padding = new Thickness(11, 8, 0, 8), CornerRadius = new CornerRadius(8),
                    Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong") };
                foreach (var sound in sounds)
                {
                    var item = new ComboBoxItem { Content = sound.Label, Tag = sound.Id };
                    Design.BindTypography(item, size: 13); combo.Items.Add(item);
                }
                combo.SelectedItem = combo.Items.Cast<ComboBoxItem>().FirstOrDefault(i => (string)i.Tag == S(prefs, "notification_sound"));
                combo.SelectionChanged += (_, _) => { if (combo.SelectedItem is ComboBoxItem item) { prefs["notification_sound"] = (string)item.Tag; LockConfirmation(); } };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(combo, T("settings.notification_sound_choice"));
                Design.BindTypography(combo, size: 13); choices.Add(combo); picker.Children.Add(combo);
                var preview = Button(T("settings.notification_sound_preview"), async () => await model.PreviewNotificationSoundAsync());
                StyleGeneralButton(preview, compact: true); Grid.SetColumn(preview, 1); picker.Children.Add(preview);
                picker.MaxWidth = 306; picker.HorizontalAlignment = HorizontalAlignment.Left;
            }
            InlineToggle(card, T("settings.notification_sound"), T("settings.notification_sound_hint"), B(prefs, "notification_sound_enabled"), v => prefs["notification_sound_enabled"] = v, details: picker);
        }
        if (model.Values["get_update_check_enabled"] is JsonValue updateValue && updateValue.TryGetValue<bool>(out var enabled))
            updateCheckToggle = InlineToggle(card, T("settings.update_check"), T("settings.update_check_hint"), enabled, async v =>
            {
                if (refreshingUpdatePreference) return;
                await model.SetUpdateCheckEnabledAsync(v);
                if (!model.Closed) Render();
            }, divider: false);
        var updateActions = new Grid { ColumnSpacing = 10, RowSpacing = 10, Margin = new Thickness(0, 12, 0, 0),
            Padding = new Thickness(0, 16, 0, 0), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) };
        updateActions.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        updateActions.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        updateActions.RowDefinitions.Add(new() { Height = GridLength.Auto }); updateActions.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var version = Mute(model.HostVersion is { } hostVersion ? "wisp-science v" + hostVersion : "");
        Design.BindTypography(version, 12); version.VerticalAlignment = VerticalAlignment.Center;
        updateActions.Children.Add(version);
        var check = Button(T("settings.check_updates"), () => { OpenUpdateDialog(); return Task.CompletedTask; });
        StyleGeneralButton(check, compact: true); Grid.SetColumn(check, 1); updateActions.Children.Add(check);
        updateActions.SizeChanged += (_, e) =>
        {
            var stacked = e.NewSize.Width < 290;
            Grid.SetColumn(check, stacked ? 0 : 1); Grid.SetRow(check, stacked ? 1 : 0);
            check.HorizontalAlignment = stacked ? HorizontalAlignment.Left : HorizontalAlignment.Right;
        };
        card.Children.Add(updateActions);
        InlineSaveActions(); MovePreferenceFooter(preferences);
        LocalEnvironment();
    }
    private void NotificationSoundFields(JsonObject draft)
    {
        if (!draft.ContainsKey("notification_sound_enabled"))
        {
            Form.Children.Add(Mute("当前桌面宿主不支持独立提示音，请更新完整原生版本。"));
            return;
        }
        Boolean(draft, "notification_sound_enabled", "任务提示音（独立于桌面通知）");
        Form.Children.Add(Mute("与桌面客户端共享提示音设置，和桌面通知分别控制。试听不会保存设置，也不会改变提示音开关。"));
        var catalog = model.Values["list_notification_sounds"];
        var options = NativeNotificationSounds.Choices(catalog, S(draft, "notification_sound"));
        if (options.Length == 0)
        {
            Form.Children.Add(Mute("此宿主暂未提供可用音色。可保存提示音开关；更新宿主后刷新此页以选择音色。"));
            return;
        }
        var preview = Button("试听", async () => { await model.PreviewNotificationSoundAsync(); });
        Choice(Form, "提示音色", S(draft, "notification_sound"), options, value =>
        {
            draft["notification_sound"] = value;
            preview.IsEnabled = NativeNotificationSounds.CanPreview(catalog, value);
        });
        preview.IsEnabled = NativeNotificationSounds.CanPreview(catalog, S(draft, "notification_sound"));
        Form.Children.Add(preview);
    }
    private void Session()
    {
        if (model.Values["get_settings"] is not JsonObject) return;
        model.EditWindowPreferences(WindowPreferences);
        var d = model.Draft!["get_settings"]!.AsObject();
        var card = InlineCard(T("settings.session.limits"));
        SessionInteger(card, d, "max_iter", T("settings.max_iter"), T("settings.max_iter_hint"));
        Action<bool>? continuations = null;
        InlineToggle(card, T("settings.auto_continue"), T("settings.auto_continue_hint"), B(d, "auto_continue"), v =>
        { d["auto_continue"] = v; continuations?.Invoke(v); });
        continuations = SessionInteger(card, d, "auto_continue_limit", T("settings.auto_continue_limit"), T("settings.auto_continue_limit_hint"), dependent: true);
        continuations(B(d, "auto_continue"));
        InlineHeading(card, T("settings.session.context"));
        Action<bool>? threshold = null;
        InlineToggle(card, T("settings.auto_compact"), T("settings.auto_compact_hint"), B(d, "auto_compact"), v =>
        { d["auto_compact"] = v; threshold?.Invoke(v); });
        threshold = SessionInteger(card, d, "auto_compact_percent", T("settings.auto_compact_percent"), T("settings.auto_compact_percent_hint"));
        threshold(B(d, "auto_compact"));
        if (d.ContainsKey("semantic_compact_on_model_switch"))
            InlineToggle(card, T("settings.semantic_compact_on_model_switch"), T("settings.semantic_compact_on_model_switch_hint"), B(d, "semantic_compact_on_model_switch"), v => d["semantic_compact_on_model_switch"] = v);
        if (d.ContainsKey("semantic_compact_idle_hours"))
            SessionInteger(card, d, "semantic_compact_idle_hours", T("settings.semantic_compact_idle_hours"), T("settings.semantic_compact_idle_hours_hint"));
        InlineHeading(card, T("settings.session.followup"));
        InlineToggle(card, T("settings.follow_up_questions"), T("settings.follow_up_questions_hint"), B(d, "follow_up_questions"), v => d["follow_up_questions"] = v, divider: d.ContainsKey("show_tips"));
        if (d.ContainsKey("show_tips")) InlineToggle(card, T("settings.show_tips"), T("settings.show_tips_hint"), B(d, "show_tips"), v => d["show_tips"] = v, divider: false);
        InlineSaveActions(); MovePreferenceFooter(card);
    }
    private void Records(JsonObject draft, string key, string title, (string Key, string Label)[] fields)
    {
        var rows = draft[key] as JsonArray;
        if (rows == null) { rows = new JsonArray(); draft[key] = rows; }
        Form.Children.Add(Mute(title)); var list = new StackPanel { Spacing = 12 }; Form.Children.Add(list);
        void Add(JsonObject row)
        {
            var group = new StackPanel { Spacing = 6 };
            foreach (var (field, label) in fields)
            {
                var input = new TextBox { Header = label, Text = S(row, field) };
                input.TextChanged += (_, _) => row[field] = input.Text; group.Children.Add(input);
            }
            group.Children.Add(Button("移除", () => { rows.Remove(row); list.Children.Remove(group); return Task.CompletedTask; })); list.Children.Add(group);
        }
        foreach (var row in Rows(rows)) Add(row);
        Form.Children.Add(Button("添加" + title, () => { var row = new JsonObject(); rows.Add(row); Add(row); return Task.CompletedTask; }));
    }
}
