using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Windows.ApplicationModel.DataTransfer;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private string labInviteCode = "", labKnowledgeProject = "";
    private bool ChannelSupports(string command) => NativeChannelSettings.Supports(model.Values["native_settings_capabilities"], command);
    private void ChannelCompatibility(string command)
    {
        if (!ChannelSupports(command)) Form.Children.Add(Warn(L("当前桌面服务尚未提供此功能的原生接口，请使用配套的新版本桌面服务。表单可查看，操作暂不可用。", "This desktop service does not support the native interface for this feature. Use the matching updated desktop service to enable these actions.")));
    }
    private Button ChannelCommand(string label, string command, Func<Task> action, bool primary = false)
    {
        var button = ChannelButton(label, action, primary); button.MinWidth = 84;
        if (primary)
        {
            button.Resources["ButtonBackgroundDisabled"] = new Microsoft.UI.Xaml.Media.SolidColorBrush(Mix(Design.Brush("bg-elev").Color, Design.Brush("clay").Color, .58));
            button.Resources["ButtonBorderBrushDisabled"] = button.Resources["ButtonBackgroundDisabled"];
            button.Resources["ButtonForegroundDisabled"] = Design.Brush("on-clay");
        }
        button.IsEnabled = ChannelSupports(command); return button;
    }
    private void ChannelFooter(StackPanel target, string note, UIElement action)
    {
        target.Children.Add(new Border { Child = ChannelColumns(ModelHint(note), (FrameworkElement)action), BorderBrush = Design.Brush("border"),
            BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(0, 16, 0, 0), Margin = new Thickness(0, 4, 0, 0) });
    }
    private Task CopyChannelText(string value)
    {
        if (value.Length > 0) { var data = new DataPackage(); data.SetText(value); Clipboard.SetContent(data); channelNotice = T("channels.remote.copied"); Render(); }
        return Task.CompletedTask;
    }
    private void RemoteAccess(JsonNode? status)
    {
        ChannelCompatibility("set_remote_access");
        if (model.Draft == null) model.Edit(NativeChannelSettings.RemoteDraft(status), "set_remote_access");
        var draft = model.Draft!;
        Form.Children.Add(ChannelColumns(ChannelCopy(T("channels.remote.heading"), T("channels.remote.hint")),
            ChannelBadge(B(status, "enabled") ? S(status, "state") : "stopped")));
        var remoteFields = new StackPanel { Spacing = 12 }; Form.Children.Add(remoteFields);
        var enabled = ChannelCheck(T("channels.remote.toggle"), B(draft, "enabled")); enabled.IsEnabled = ChannelSupports("set_remote_access");
        enabled.Click += async (_, _) => { draft["enabled"] = enabled.IsChecked == true; if (await model.SaveAsync()) await Reload(); else if (!model.Closed) Render(); };
        remoteFields.Children.Add(enabled);
        ChannelInput(remoteFields, T("channels.remote.relay_url"), S(draft, "relay_url"), value => draft["relay_url"] = value, "https://relay.example.com");
        ChannelSecret(remoteFields, draft, "relay_token", T("channels.remote.token"), B(status, "has_token") ? T("settings.key_stored") : T("channels.remote.token_placeholder"));
        if (B(status, "enabled") && S(status, "detail").Length > 0) Form.Children.Add(Warn(S(status, "detail")));
        if (S(status, "code") is { Length: > 0 } code)
        {
            var codes = new StackPanel { Spacing = 12 }; codes.Children.Add(Design.Text(T("channels.remote.code"), 14));
            codes.Children.Add(new TextBlock { Text = code, IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap, FontSize = 20 });
            var link = S(status, "link"); if (link.Length > 0) codes.Children.Add(new TextBlock { Text = link, IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap });
            var actions = new NativeActionWrap(); var copy = ChannelButton(T("channels.remote.copy_link"), () => CopyChannelText(link)); copy.IsEnabled = link.Length > 0; actions.Children.Add(copy);
            actions.Children.Add(ChannelButton(T("channels.remote.copy_code"), () => CopyChannelText(code)));
            actions.Children.Add(ChannelCommand(T("channels.remote.reset"), "reset_remote_access_code", () => { Confirm(L("重置会使旧链接失效并断开所有已连接网页。继续？", "Reset the code? The previous link will stop working and all connected web clients will disconnect."), () => _ = ChannelAction("reset_remote_access_code", new())); return Task.CompletedTask; }));
            codes.Children.Add(actions); Form.Children.Add(Design.Card(codes));
        }
        if (B(status, "enabled") && S(status, "state") == "running") Form.Children.Add(ModelHint(L("已连接的网页：", "Connected web clients: ") + (status?["clients"]?.ToString() ?? "0")));
        Form.Children.Add(ModelHint(T("channels.remote.security")));
        ChannelFooter(Form, T("channels.secret_note"), ChannelCommand(T("settings.save"), "set_remote_access", async () => { if (await model.SaveAsync()) { channelNotice = T("channels.saved"); await Reload(); } }, true));
    }
    private async Task RefreshLab()
    {
        if (model.Closed || channelDetail != "lab" || !ChannelSupports("lab_status")) return;
        var oldState = S(model.Values["lab_status"], "state");
        var loaded = await model.InvokeAsync("lab_status", new(), value => model.Values["lab_status"] = value?.DeepClone());
        if (model.Closed || channelDetail != "lab") return;
        if (loaded && oldState.Length > 0 && oldState != S(model.Values["lab_status"], "state")) { model.Discard(); labInviteCode = ""; }
        if (loaded && !model.Values.ContainsKey("lab_projects")) await model.InvokeAsync("list_projects", new(), value => model.Values["lab_projects"] = value?.DeepClone());
        if (!model.Closed) Render();
    }
    private async Task LabAction(string command, JsonObject args, string? notice = null, bool discard = false)
    {
        if (!ChannelSupports(command)) return;
        JsonNode? result = null;
        var saved = await model.InvokeAsync(command, args, value => result = value?.DeepClone());
        if (model.Closed) return;
        var error = model.Error;
        if (saved)
        {
            if (discard) model.Discard();
            channelNotice = notice ?? L("操作已完成。", "Done.");
            if (command == "lab_create_invite") labInviteCode = result?.GetValue<string>() ?? "";
            if (command == "lab_join") await model.InvokeAsync("get_settings", new(), value =>
                model.Values["get_settings"] = value is JsonObject prefs ? NativeWindowPreferences.EditableSnapshot(prefs) : null);
            if (command == "lab_join_knowledge_base")
            {
                if (S(result, "id") is { Length: > 0 } id) { ProjectRequested?.Invoke(id); return; }
                channelNotice = L("已取消选择文件夹。", "Folder selection cancelled.");
            }
        }
        await RefreshLab();
        if (!saved && error != null && !model.Closed) model.Fail(NativeLabSettings.Explain(error, Design.Localization));
    }
    private void Lab()
    {
        ChannelCompatibility("lab_status");
        var status = model.Values["lab_status"];
        Form.Children.Add(ChannelColumns(ChannelCopy(S(status, "lab_name") is { Length: > 0 } name ? name : T("channels.lab.heading"),
            S(status, "relay_url") is { Length: > 0 } relay ? relay : T("channels.lab.hint")), ConnectionBadge(NativeLabSettings.StateLabel(status, Design.Localization))));
        switch (S(status, "state"))
        {
            case "none": LabJoin(status); break;
            case "active": LabMembers(status!); break;
            case "pending":
                Form.Children.Add(ModelHint(T("channels.lab.pending_note")));
                LabStateActions(T("channels.lab.withdraw"), false); break;
            case "removed": case "error":
                Form.Children.Add(Warn(S(status, "state") == "removed" ? T("channels.lab.removed_note") : NativeLabSettings.Explain(S(status, "detail"), Design.Localization)));
                LabToken(); LabStateActions(T("channels.lab.forget"), true); break;
            default:
                // A missing capability/status is not proof that this machine has no Lab identity.
                if (!ChannelSupports("lab_status")) LabJoin(null);
                else Form.Children.Add(ChannelButton(L("刷新课题组状态", "Refresh lab status"), RefreshLab));
                break;
        }
    }
    private void LabJoin(JsonNode? status)
    {
        if (model.Draft == null) model.Edit(NativeLabSettings.JoinDraft(), "lab_join");
        var draft = model.Draft!;
        var joinFields = new StackPanel { Spacing = 12 }; Form.Children.Add(joinFields);
        var labName = new StackPanel(); var name = new StackPanel(); var fields = ChannelColumns(name, labName, true, formGrid: true);
        Button? join = null; TextBox? url = null;
        void UpdateJoin()
        {
            var invite = S(draft, "invite").Trim().Length > 0;
            labName.Visibility = invite ? Visibility.Collapsed : Visibility.Visible;
            Grid.SetColumnSpan(name, invite || NativeSettingsLayout.ChannelFieldsStack(XamlRoot?.Size.Width ?? ActualWidth, formGrid: true) ? 2 : 1);
            if (url != null) url.PlaceholderText = invite ? T("channels.lab.relay_url_from_invite") : "https://relay.example.com";
            if (join != null)
            {
                var label = invite ? T("channels.lab.join") : T("channels.lab.found");
                if (join.Content is TextBlock text) text.Text = label;
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(join, label);
                join.IsEnabled = status != null && ChannelSupports("lab_join") && NativeLabSettings.JoinBlockReason(draft) == null;
            }
        }
        ChannelInput(joinFields, T("channels.lab.invite"), S(draft, "invite"), value => { draft["invite"] = value; UpdateJoin(); }, T("channels.lab.invite_placeholder"));
        url = ChannelInput(joinFields, T("channels.remote.relay_url"), S(draft, "relay_url"), value => draft["relay_url"] = value, "https://relay.example.com");
        ChannelSecret(joinFields, draft, "relay_token", T("channels.remote.token"), B(status, "has_token") ? T("settings.key_stored") : T("channels.remote.token_placeholder"));
        ChannelInput(name, T("channels.lab.your_name"), S(draft, "name"), value => { draft["name"] = value; UpdateJoin(); }).MaxLength = 64;
        ChannelInput(labName, T("channels.lab.lab_name"), S(draft, "lab_name"), value => { draft["lab_name"] = value; UpdateJoin(); }).MaxLength = 64;
        fields.SizeChanged += (_, _) => UpdateJoin(); joinFields.Children.Add(fields);
        Form.Children.Add(ModelHint(T("channels.lab.join_note")));
        join = ChannelCommand(T("channels.lab.found"), "lab_join", async () =>
        {
            if (NativeLabSettings.JoinBlockReason(draft) is { } error) { model.Fail(error); return; }
            var args = NativeLabSettings.JoinArguments(draft); draft["relay_token"] = ""; ClearChannelSecrets();
            await LabAction("lab_join", args, L("已提交课题组申请。", "Lab request submitted."), true);
        }, true);
        ChannelFooter(Form, T("channels.secret_note"), join); UpdateJoin();
    }
    private void LabStateActions(string label, bool forget)
    {
        var actions = new NativeActionWrap(); actions.Children.Add(ChannelButton(T("channels.lab.refresh"), RefreshLab));
        actions.Children.Add(ChannelCommand(label, "lab_leave", () => { Confirm(label + L("？本机课题组密钥将被删除，重新加入需要新的校验码。", "? This removes the local lab key. Rejoining requires a new invite code."), () => _ = LabAction("lab_leave", new() { ["forget"] = forget }, T("channels.lab.left"), true)); return Task.CompletedTask; }));
        Form.Children.Add(actions);
    }
    private void LabToken()
    {
        if (model.Draft == null) model.Edit(new() { ["relay_token"] = "" }, "lab_set_relay_token");
        var draft = model.Draft!; var token = new StackPanel { Spacing = 12 };
        ChannelSecret(token, draft, "relay_token", T("channels.lab.new_token"), T("channels.lab.new_token_placeholder"));
        var save = ChannelCommand(T("channels.lab.save_token"), "lab_set_relay_token", async () =>
        {
            if (string.IsNullOrWhiteSpace(S(draft, "relay_token"))) { model.Fail(L("请填写新的中继访问令牌。", "Enter the new relay access token.")); return; }
            var args = new JsonObject { ["relayToken"] = S(draft, "relay_token") }; draft["relay_token"] = ""; ClearChannelSecrets();
            await LabAction("lab_set_relay_token", args, L("令牌已更新。", "Token updated."), true);
        });
        var input = channelSecrets.Last(); input.PasswordChanged += (_, _) => save.IsEnabled = ChannelSupports("lab_set_relay_token") && !string.IsNullOrWhiteSpace(input.Password);
        save.IsEnabled = ChannelSupports("lab_set_relay_token") && !string.IsNullOrWhiteSpace(S(draft, "relay_token"));
        token.Children.Add(save); Form.Children.Add(token);
    }
    private void LabMembers(JsonNode status)
    {
        Form.Children.Add(Design.Text(T("channels.lab.members"), 16));
        var members = new StackPanel(); Form.Children.Add(members);
        foreach (var member in Rows(status["members"]))
        {
            var label = new NativeActionWrap(); label.Children.Add(Design.Text(S(member, "name"), 14));
            if (B(member, "leader")) label.Children.Add(ConnectionBadge(T("channels.lab.role_leader")));
            if (S(member, "id") == S(status, "member_id")) label.Children.Add(ConnectionBadge(T("channels.lab.you")));
            if (B(member, "pending")) label.Children.Add(ConnectionBadge(T("channels.lab.pending")));
            var actions = new NativeActionWrap();
            if (NativeLabSettings.CanManageMember(status, member))
            {
                var args = new JsonObject { ["memberId"] = S(member, "id") };
                if (B(member, "pending")) actions.Children.Add(ChannelCommand(T("channels.lab.approve"), "lab_approve_member", () => LabAction("lab_approve_member", args, T("channels.lab.approved"))));
                var remove = B(member, "pending") ? T("channels.feishu.pending_reject") : T("channels.lab.remove");
                actions.Children.Add(ChannelCommand(remove, "lab_remove_member", () => { Confirm(remove + L("成员“", " member “") + S(member, "name") + "”？", () => _ = LabAction("lab_remove_member", args, T("channels.lab.removed"))); return Task.CompletedTask; }));
            }
            members.Children.Add(new Border { Child = ChannelColumns(label, actions), Padding = new Thickness(0, 12, 0, 12), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
        }
        if (B(status, "leader"))
        {
            ChannelDivider(Form, T("channels.lab.invite_title")); Form.Children.Add(ModelHint(T("channels.lab.invite_hint")));
            if (labInviteCode.Length > 0) { Form.Children.Add(new TextBlock { Text = labInviteCode, IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap }); Form.Children.Add(ChannelButton(T("channels.lab.invite_copy"), () => CopyChannelText(labInviteCode))); }
            Form.Children.Add(ChannelCommand(T("channels.lab.invite_create"), "lab_create_invite", () => LabAction("lab_create_invite", new(), L("校验码已生成。", "Invite code created."))));
        }
        LabKnowledge(status); LabMail(status); LabToken();
        if (!B(status, "leader")) { Form.Children.Add(ModelHint(T("channels.lab.leave_note"))); LabStateActions(T("channels.lab.leave"), false); }
        else Form.Children.Add(ChannelButton(T("channels.lab.refresh"), RefreshLab));
    }
    private void LabKnowledge(JsonNode status)
    {
        ChannelDivider(Form, T("channels.lab.kb_title")); var kb = status["knowledge_base"];
        Form.Children.Add(ModelHint(kb == null ? T("channels.lab.error.no_knowledge_base") : B(kb, "local") ? L("已在本机，项目名为“", "Available locally as “") + S(kb, "project_name") + L("”。在该项目卡片上点击“立即同步”获取组长的更新。", "”. Select Sync now on the project card to get the leader’s updates.") : T("channels.lab.kb_remote")));
        if (kb != null && !B(kb, "local")) Form.Children.Add(ChannelCommand(T("channels.lab.kb_download"), "lab_join_knowledge_base", () => { RequestLeave(() => { model.Discard(); _ = LabAction("lab_join_knowledge_base", new(), L("知识库已下载。", "Knowledge base downloaded.")); }); return Task.CompletedTask; }));
        if (!B(status, "leader")) return;
        var publish = ChannelCommand(T("channels.lab.kb_publish"), "lab_publish_knowledge_base", () => LabAction("lab_publish_knowledge_base", new() { ["projectId"] = labKnowledgeProject }, T("channels.lab.kb_published")));
        publish.IsEnabled &= labKnowledgeProject.Length > 0;
        LabProjects(T("channels.lab.kb_project"), T("channels.lab.kb_choose"), labKnowledgeProject, value => { labKnowledgeProject = value; publish.IsEnabled = ChannelSupports("lab_publish_knowledge_base") && value.Length > 0; });
        Form.Children.Add(publish); Form.Children.Add(ModelHint(T("channels.lab.kb_note")));
    }
    private void LabMail(JsonNode status)
    {
        ChannelDivider(Form, T("channels.lab.mail_title"));
        LabProjects(T("channels.lab.inbox_project"), T("channels.lab.inbox_none"), S(status, "inbox_project_id"), value => _ = LabAction("lab_set_inbox_project", new() { ["projectId"] = value }, T("channels.lab.inbox_saved")), ChannelSupports("lab_set_inbox_project"));
        Form.Children.Add(ModelHint(T("channels.lab.mail_note")));
    }
    private void LabProjects(string title, string empty, string current, Action<string> changed, bool enabled = true)
    {
        var options = new List<(string Id, string Label)> { ("", empty) };
        options.AddRange(Rows(model.Values["lab_projects"]).Select(project => (S(project, "id"), S(project, "name"))));
        if (current.Length > 0 && !options.Any(option => option.Id == current)) options.Add((current, L("当前项目（暂不可用）", "Current project (unavailable)")));
        Choice(Form, title, current, options.ToArray(), changed); choices.Last().IsEnabled = enabled;
    }
}
