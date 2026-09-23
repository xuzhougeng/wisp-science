using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeChannelSettingsTests
{
    private static void Check(bool value, string description)
    { if (!value) throw new InvalidOperationException(description); Console.WriteLine("PASS channel settings: " + description); }
    public static async Task RunAsync()
    {
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
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<string, JsonObject, Task<JsonNode?>>? Handler;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId)); return Handler?.Invoke(command, arguments) ?? Task.FromResult<JsonNode?>(null); }
    }
}
