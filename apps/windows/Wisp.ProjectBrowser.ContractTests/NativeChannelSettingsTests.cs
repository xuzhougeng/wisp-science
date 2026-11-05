using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeChannelSettingsTests
{
    private static void Check(bool value, string description)
    { if (!value) throw new InvalidOperationException(description); Console.WriteLine("PASS channel settings: " + description); }
    public static async Task RunAsync()
    {
        DraftContracts();
        OverviewContracts();
        RemoteLabContracts();
        await LocalizedBindingContracts();
        var fake = new Fake(); using var binding = new NativeChannelBindingModel(fake, "channel-project");
        fake.Handler = (c, _) => Task.FromResult<JsonNode?>(c == "feishu_bind_start" ? new JsonObject { ["flow_id"] = "flow-a", ["qr_content"] = "synthetic" } : null);
        Check(await binding.StartAsync("feishu", true) && binding.Active && fake.Calls.Single().Args["international"]!.GetValue<bool>(), "Feishu registration preserves Lark selection and opaque flow identity");
        Check(!await binding.StartAsync("weixin") && fake.Calls.Count == 1, "an active binding cannot be silently replaced by another platform");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject { ["state"] = "pending", ["retry_after_ms"] = 60000 });
        await binding.PollAsync(); var count = fake.Calls.Count; await binding.PollAsync();
        Check(fake.Calls.Count == count && fake.Calls.Last().Args["flowId"]!.GetValue<string>() == "flow-a" && fake.Calls.Last().Project == "channel-project",
            "Feishu polling respects host retry interval and bound project scope");
        fake.Handler = (_, _) => throw new IOException("uncertain cancel"); count = fake.Calls.Count;
        Check(!await binding.CancelAsync() && binding.Active && fake.Calls.Count == count + 1, "failed cancellation retains the flow and does not retry automatically");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(null);
        Check(await binding.CancelAsync() && !binding.Active && fake.Calls.Last().Command == "feishu_bind_cancel", "confirmed Feishu cancellation clears the visible registration");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject { ["qrcode"] = "qr-a", ["qr_content"] = "synthetic" });
        await binding.StartAsync("weixin"); count = fake.Calls.Count; await binding.CancelAsync();
        Check(!binding.Active && fake.Calls.Count == count, "Weixin cancellation drops local polling without calling a nonexistent cancellation command");
        await binding.StartAsync("weixin"); fake.Handler = (_, _) => Task.FromResult<JsonNode?>(JsonValue.Create("confirmed"));
        await binding.PollAsync(); count = fake.Calls.Count; await binding.PollAsync();
        Check(!binding.Active && binding.Status == "绑定已确认" && fake.Calls.Count == count, "confirmed Weixin poll ends the flow without another credential-finalizing poll");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject { ["flow_id"] = "flow-b" });
        await binding.StartAsync("feishu"); fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject { ["state"] = "expired" });
        await binding.PollAsync();
        Check(!binding.Active && binding.Status == "二维码已过期", "expired registration clears its QR flow");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject());
        Check(!await binding.StartAsync("feishu") && !binding.Active, "missing registration identity is rejected instead of creating an unusable flow");
        var late = new TaskCompletionSource<JsonNode?>();
        fake.Handler = (c, _) => c == "feishu_bind_start" ? late.Task : Task.FromResult<JsonNode?>(null);
        using var closed = new NativeChannelBindingModel(fake, "late-project"); var start = closed.StartAsync("feishu"); closed.Dispose();
        late.SetResult(new JsonObject { ["flow_id"] = "late-flow" }); await start;
        Check(!closed.Active && fake.Calls.Last().Command == "feishu_bind_cancel" && fake.Calls.Last().Args["flowId"]!.GetValue<string>() == "late-flow",
            "late registration is cancelled without repopulating a closed page");
        fake.Handler = (c, _) => Task.FromResult<JsonNode?>(c == "feishu_bind_start" ? new JsonObject { ["flow_id"] = "dispose-flow" } : null);
        var disposing = new NativeChannelBindingModel(fake, "dispose-project"); await disposing.StartAsync("feishu"); disposing.Dispose();
        Check(fake.Calls.Last().Command == "feishu_bind_cancel" && !disposing.Active, "disposing an active Feishu page releases its host registration");
        using var editor = new NativeSettingsEditorModel(fake, "channel-project");
        editor.Edit(new() { ["enabled"] = true, ["international"] = false, ["app_id"] = "synthetic-app", ["app_secret"] = "" }, "set_feishu_channel");
        await editor.SaveAsync();
        Check(fake.Calls.Last().Args["appId"]!.GetValue<string>() == "synthetic-app" && fake.Calls.Last().Args["appSecret"]!.GetValue<string>() == "",
            "manual channel configuration uses Tauri casing and preserves blank-secret semantics");
        editor.Edit(new() { ["sync_backend"] = "folder", ["sync_folder"] = "C:/fixture-sync", ["model"] = "existing-model", ["future_setting"] = 1 }, "set_settings", "settings");
        editor.Draft!["sync_folder"] = "D:/fixture-sync"; await editor.SaveAsync();
        Check(fake.Calls.Last().Args["settings"]!["model"]!.GetValue<string>() == "existing-model" && fake.Calls.Last().Args["settings"]!["future_setting"]!.GetValue<int>() == 1,
            "sync configuration edits preserve unrelated model and future settings");
        var device = NativeChannelSettings.DeviceDraft(new JsonObject { ["bindIpv4"] = "192.0.2.10", ["port"] = 18766, ["enabled"] = false });
        editor.Edit(device, "set_device_bridge"); editor.Draft!["bind_ipv4"] = "0.0.0.0"; count = fake.Calls.Count;
        Check(!await editor.SaveAsync() && fake.Calls.Count == count && editor.Draft != null,
            "invalid device input stays editable and never reaches the host");
        editor.Draft!["bind_ipv4"] = " 192.0.2.10 "; editor.Draft["port"] = "18767";
        Check(await editor.SaveAsync() && fake.Calls.Last().Args["bindIpv4"]!.GetValue<string>() == "192.0.2.10"
            && fake.Calls.Last().Args["port"]!.GetValue<int>() == 18767 && fake.Calls.Last().Project == "channel-project",
            "device save trims the address, sends a numeric port and preserves project scope");
        editor.Edit(NativeChannelSettings.RemoteDraft(new JsonObject { ["enabled"] = false, ["relay_url"] = "https://relay.example.test" }), "set_remote_access");
        await editor.SaveAsync();
        Check(fake.Calls.Last().Args["relayUrl"]!.GetValue<string>() == "https://relay.example.test"
            && fake.Calls.Last().Args["relayToken"]!.GetValue<string>() == "" && !fake.Calls.Last().Args["enabled"]!.GetValue<bool>(),
            "remote access uses the original camelCase payload and blank token preserves the keyring credential");
        var caps = new JsonObject { ["commands"] = new JsonArray("lab_status", "set_remote_access") };
        fake.Handler = (c, _) => Task.FromResult<JsonNode?>(c == "native_settings_capabilities" ? caps : null);
        editor.Edit(NativeLabSettings.JoinDraft(), "lab_join"); editor.Draft!["name"] = "unsaved";
        await editor.LoadChannelCapabilitiesAsync();
        Check(editor.Draft!["name"]!.GetValue<string>() == "unsaved" && NativeChannelSettings.Supports(editor.Values["native_settings_capabilities"], "lab_status"),
            "optional capability refresh never replaces an unsaved lab draft");
        var before = editor.Values.ToJsonString(); fake.Handler = (_, _) => throw new IOException("old host");
        Check(!await editor.LoadChannelCapabilitiesAsync() && editor.Values.ToJsonString() == before && editor.Draft != null,
            "failed capability reads preserve existing settings and draft");
    }
    private static async Task LocalizedBindingContracts()
    {
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>(), "en");
        var fake = new Fake { Handler = (_, _) => Task.FromResult<JsonNode?>(new JsonObject { ["qrcode"] = "synthetic" }) };
        using var binding = new NativeChannelBindingModel(fake, null, locale);
        Check(await binding.StartAsync("weixin") && binding.Status == "Waiting for scan confirmation", "binding status follows the selected application language");
        fake.Handler = (_, _) => Task.FromResult<JsonNode?>(JsonValue.Create("future-status"));
        await binding.PollAsync();
        Check(binding.Status == "future-status" && binding.Active, "unknown binding status stays visible without inventing completion");
        locale.Preview("zh"); await binding.CancelAsync();
        Check(binding.Status == "已取消绑定" && !binding.Active, "binding cancellation uses the current locale without changing local cancellation semantics");
    }
    private static void DraftContracts()
    {
        var status = new JsonObject { ["feishu_enabled"] = false, ["feishu_bound"] = true, ["feishu_has_secret"] = true,
            ["feishu_app_id"] = "synthetic-app", ["feishu_international"] = true, ["weixin_bound"] = false,
            ["device"] = new JsonObject { ["bindIpv4"] = "192.0.2.10", ["port"] = 18766, ["hasToken"] = true, ["enabled"] = false } };
        var feishu = NativeChannelSettings.FeishuDraft(status);
        Check(feishu["app_secret"]!.GetValue<string>() == "" && !feishu.ContainsKey("feishu_owner_open_id")
            && feishu["international"]!.GetValue<bool>() && !feishu["enabled"]!.GetValue<bool>(),
            "Feishu inline editing preserves enabled/region state without exposing a secret or writing owner state");
        Check(NativeChannelSettings.CanEnable(status, "feishu") && !NativeChannelSettings.CanEnable(status, "weixin")
            && NativeChannelSettings.CanEnable(status, "device") && !NativeChannelSettings.CanEnable(status, "unknown"),
            "list toggles require the same binding or address prerequisites as WebView");
        var device = NativeChannelSettings.DeviceDraft(status["device"]);
        Check(device["bind_ipv4"]!.GetValue<string>() == "192.0.2.10" && device["port"]!.GetValue<int>() == 18766
            && !device.ContainsKey("hasToken") && NativeChannelSettings.DeviceSaveBlockReason(device) == null,
            "camelCase device status becomes an independent editable draft without a token");
        foreach (var invalid in new[] { "", "0.0.0.0", "127.1", "::1", "192.0.2.256", "0x7f.0.0.1", "192.00.2.1", "192.0.2.-1" })
        { device["bind_ipv4"] = invalid; Check(NativeChannelSettings.DeviceSaveBlockReason(device) != null, "reject invalid/unspecified IPv4: " + invalid); }
        device["bind_ipv4"] = "192.0.2.10";
        foreach (var invalid in new[] { "", "0", "65536", "2.5", "abc" })
        { device["port"] = invalid; Check(NativeChannelSettings.DeviceSaveBlockReason(device) != null, "reject invalid device port: " + invalid); }
        device["port"] = 65535; Check(NativeChannelSettings.DeviceSaveBlockReason(device) == null, "accept the upper port bound");
        device["mode"] = "relay"; Check(NativeChannelSettings.DeviceSaveBlockReason(device) != null, "planned relay mode cannot be submitted");
        var prefs = new JsonObject { ["sync_backend"] = "relay", ["sync_relay_token"] = "must-not-display", ["has_sync_relay_token"] = true,
            ["model"] = "keep", ["future"] = new JsonObject { ["value"] = 1 } };
        var sync = NativeWindowPreferences.EditableSnapshot(prefs); sync["future"]!["value"] = 2;
        Check(sync["sync_relay_token"]!.GetValue<string>() == "" && sync["has_sync_relay_token"]!.GetValue<bool>()
            && sync["model"]!.GetValue<string>() == "keep" && prefs["future"]!["value"]!.GetValue<int>() == 1,
            "sync draft keeps unrelated preferences and credential-presence metadata without retaining a token");
    }
    private static void OverviewContracts()
    {
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>(), "en");
        var status = new JsonObject { ["feishu_state"] = "running", ["feishu_app_id"] = "partial",
            ["weixin_bound"] = true, ["weixin_state"] = "connecting",
            ["device"] = new JsonObject { ["enabled"] = false, ["state"] = "listening", ["bindIpv4"] = "192.0.2.10" },
            ["remote"] = new JsonObject { ["enabled"] = true, ["state"] = "error", ["relay_url"] = "https://relay.example.invalid" } };
        NativeChannelSettings.RowStatus Row(string channel) => NativeChannelSettings.Overview(status, channel, locale);
        Check(Row("feishu") == new NativeChannelSettings.RowStatus("stopped", "channels.feishu.not_bound", "channels.feishu.subtitle"),
            "a partially configured bot cannot appear running or advertise its unused app id");
        Check(Row("weixin").Hint == "channels.weixin.subtitle" && NativeChannelSettings.StateColor(Row("weixin").State) == "warn",
            "a bound bot without extra detail retains its subtitle and connecting tone");
        Check(Row("device").State == "stopped" && Row("device").Hint == "192.0.2.10:18766",
            "disabled bridge hides a stale listener status in the overview, while preserving the address and default port");
        status["device"]!["enabled"] = true; status["device"]!["port"] = 18800;
        Check(Row("device").Hint == "192.0.2.10:18800" && NativeChannelSettings.StateColor(Row("device").State) == "ok",
            "enabled listening bridge uses its configured port and the same success tone as a running bot");
        status["device"]!["url"] = "http://192.0.2.10:18800/bridge";
        Check(Row("device").Hint == "http://192.0.2.10:18800/bridge", "the host's explicit device URL takes precedence");
        Check(Row("remote").Hint == "https://relay.example.invalid" && NativeChannelSettings.StateColor(Row("remote").State) == "err",
            "enabled remote access shows its relay address and error tone");
        status["remote"]!["enabled"] = false;
        Check(Row("remote").Hint == "channels.remote.subtitle" && NativeChannelSettings.StateColor(Row("remote").State) == "text-muted",
            "disabled remote access displays the generic hint and neutral state");
        Check(NativeChannelSettings.StateColor("running") == "ok" && NativeChannelSettings.StateColor("future") == "text-muted",
            "unknown states have the same neutral fallback as the WebView");
    }
    private static void RemoteLabContracts()
    {
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>
        {
            ["en"] = new() { ["channels.state.stopped"] = "Stopped", ["channels.device.state.listening"] = "Listening",
                ["channels.lab.not_joined"] = "Not joined", ["channels.lab.role_leader"] = "Leader",
                ["channels.lab.error.invalid_invite"] = "Invite code is invalid." },
            ["zh"] = new() { ["channels.state.stopped"] = "已停止", ["channels.device.state.listening"] = "正在监听",
                ["channels.lab.not_joined"] = "未加入" }
        }, "en");
        Check(NativeChannelSettings.StateLabel("listening", locale) == "Listening"
            && NativeChannelSettings.StateLabel("future-state", locale) == "Stopped", "channel badges use locale keys and retain the stopped fallback");
        Check(NativeLabSettings.StateLabel(null, locale) == "Status unavailable"
            && NativeLabSettings.StateLabel(new JsonObject { ["state"] = "none" }, locale) == "Not joined",
            "localized lab copy distinguishes an unread identity from an unjoined identity");
        Check(NativeLabSettings.Explain("Do not retry.\r\nlab:invalid_invite\nlab:new_code\nuser label", locale)
            == "Do not retry.\nInvite code is invalid.\nlab:new_code\nuser label", "only known protocol errors are translated; retry boundaries and unknown/user text remain literal");
        locale.Preview("zh-CN");
        Check(NativeChannelSettings.StateLabel("listening", locale) == "正在监听"
            && NativeLabSettings.StateLabel(new JsonObject { ["state"] = "none" }, locale) == "未加入", "channel presentation follows subsequent locale changes");
        Check(!NativeChannelSettings.Supports(null, "lab_join") && !NativeChannelSettings.Supports(new JsonObject { ["commands"] = new JsonArray("lab_status") }, "lab_join"),
            "older or partially capable hosts cannot enable unsupported mutations");
        var status = new JsonObject { ["remote"] = new JsonObject { ["relay_url"] = "https://relay.example.test", ["enabled"] = false, ["has_token"] = false, ["code"] = "private-code" } };
        Check(!NativeChannelSettings.CanEnable(status, "remote"), "remote enable requires a stored token");
        status["remote"]!["has_token"] = true;
        Check(NativeChannelSettings.CanEnable(status, "remote"), "configured remote access may be enabled");
        var remote = NativeChannelSettings.RemoteDraft(status["remote"]);
        Check(remote.Count == 3 && remote["relay_token"]!.GetValue<string>() == "" && !remote.ContainsKey("code"), "remote editing never copies connection codes into its save payload");
        var draft = NativeLabSettings.JoinDraft();
        Check(NativeLabSettings.JoinBlockReason(draft) != null, "blank lab member name cannot be submitted");
        draft["name"] = "  Researcher  "; draft["lab_name"] = "  Lab  "; draft["relay_url"] = " https://relay.example.test "; draft["invite"] = " invite "; draft["relay_token"] = " token ";
        var args = NativeLabSettings.JoinArguments(draft);
        Check(args["name"]!.GetValue<string>() == "Researcher" && args["labName"]!.GetValue<string>() == "Lab" && args["invite"]!.GetValue<string>() == "invite"
            && args["relayToken"]!.GetValue<string>() == " token " && args["relayUrl"]!.GetValue<string>() == "https://relay.example.test" && args.Count == 5,
            "lab joining trims metadata but passes the secret unchanged to its existing backend");
        Check(NativeLabSettings.StateLabel(null) == "状态未读取" && NativeLabSettings.StateLabel(new JsonObject { ["state"] = "none" }) == "未加入",
            "missing lab status is never displayed as an unjoined identity");
        var leader = new JsonObject { ["state"] = "active", ["leader"] = true, ["member_id"] = "self" };
        Check(NativeLabSettings.StateLabel(leader) == "组长" && !NativeLabSettings.CanManageMember(leader, new JsonObject { ["id"] = "self" })
            && NativeLabSettings.CanManageMember(leader, new JsonObject { ["id"] = "other" }), "leader actions exclude this machine");
        leader["leader"] = false;
        Check(!NativeLabSettings.CanManageMember(leader, new JsonObject { ["id"] = "other" }), "ordinary members cannot manage another member");
        Check(NativeLabSettings.Explain("操作结果未能确认；不会自动重试。\nlab:invalid_invite").EndsWith("校验码无效，可能已被使用或已过期。")
            && NativeLabSettings.Explain("connection refused") == "connection refused", "known relay codes localize without hiding transport errors or retry boundaries");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<string, JsonObject, Task<JsonNode?>>? Handler;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId)); return Handler?.Invoke(command, arguments) ?? Task.FromResult<JsonNode?>(null); }
    }
}
