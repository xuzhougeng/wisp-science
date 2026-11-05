using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSessionRulesTests
{
    public static async Task RunAsync(string fixtures)
    {
        var data = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtures, "../../native-conversations/v1/rules.json")))!;
        var fake = new Fake(data);
        Check(await WorkspaceSessionRules.ReadStaleAsync(fake, "research-1", "main-1"), "shared stale flag enables reload");
        fake.Current = true; Check(!await WorkspaceSessionRules.ReadStaleAsync(fake, "research-1", "main-1"), "current rules omit reload");
        using var groups = new WorkspaceSessionGroups(fake, "research-1");
        var row = new BrowserSession("main-1", "research-1", "Main", 0, "complete", Pinned: false);
        Check(NativeSessionMenuEntries.Conversation(row, groups).All(e => e.Id != "reload_rules"), "unknown/older status never invents staleness");
        var menu = NativeSessionMenuEntries.Conversation(row with { BranchState = "active" }, groups, true);
        Check(menu.Take(4).Select(e => e.Id).SequenceEqual(["copy", "open", "reload_rules", "merge_branch"]), "stale rules precede branch merge as in WebView");
        Check(!NativeSessionMenuEntries.Conversation(row, groups, null).Single(e => e.Id == "reload_rules").Visible, "pending status reserves a hidden entry without moving focus");
        using var state = new WorkspaceSessionRules(fake, "research-1", "main-1");
        Check(await state.ReloadAsync() && state.Result?.Changed == true && !state.CanReload, "confirmed reload completes once");
        var count = fake.Calls.Count; await state.ReloadAsync(); Check(fake.Calls.Count == count, "successful reload is not replayed");
        fake.Unchanged = true; using var noop = new WorkspaceSessionRules(fake, "research-1", "main-1");
        Check(await noop.ReloadAsync() && noop.Result?.Changed == false, "already current rules are a successful no-op");
        fake.Fail = true; using var failed = new WorkspaceSessionRules(fake, "research-1", "main-1");
        Check(!await failed.ReloadAsync() && failed.MutationUnconfirmed && !failed.CanReload, "lost acknowledgement blocks repeat");
        count = fake.Calls.Count; await failed.ReloadAsync(); Check(fake.Calls.Count == count, "no replay after failure"); fake.Fail = false;
        foreach (var mode in new[] { "project", "session", "missing" })
        {
            fake.Wrong = mode;
            try { await WorkspaceSessionRules.ReadStaleAsync(fake, "research-1", "main-1"); throw new Exception("Invalid status accepted"); }
            catch (Exception error) when (error is InvalidDataException or JsonException) { }
            using var invalid = new WorkspaceSessionRules(fake, "research-1", "main-1");
            Check(!await invalid.ReloadAsync() && invalid.MutationUnconfirmed && invalid.Result == null, "invalid acknowledgement cannot complete");
        }
        fake.Wrong = null; fake.Pending = new(); using var closed = new WorkspaceSessionRules(fake, "research-1", "main-1");
        var loading = closed.ReloadAsync(); Check(closed.Busy && !closed.CanReload, "in-flight reload disables action");
        count = fake.Calls.Count; await closed.ReloadAsync(); Check(fake.Calls.Count == count, "busy action cannot submit twice");
        closed.Dispose(); fake.Pending.SetResult(data["reloaded"]!.DeepClone()); Check(!await loading && closed.Result == null, "closed dialog ignores late acknowledgement");
        Check(fake.Calls.All(c => c.Project == "research-1" && c.Args["session_id"]!.ToString() == "main-1" && c.Args.Count == 1), "explicit project/session scope, no transcript or rule text sent");
        var catalog = JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(File.ReadAllText(Path.Combine(fixtures, "../../../apps/macos/Sources/WispProjectBrowserUI/Resources/native-locales.json")))!;
        foreach (var language in new[] { "en", "zh" })
        {
            var locale = new NativeLocalization(catalog, language);
            foreach (var key in new[] { "ctx.reload_rules", "confirm.title", "session.reload_rules_hint", "session.reload_rules_action", "settings.cancel" })
                Check(locale.Text(key) != key, "confirmation uses exported WebView labels");
        }
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake(JsonNode data) : INativeSettingsClient
    {
        public bool Current, Unchanged, Fail; public string? Wrong; public TaskCompletionSource<JsonNode?>? Pending;
        public List<(string Command, JsonObject Args, string? Project)> Calls = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, args.DeepClone().AsObject(), projectId));
            if (Fail) throw new IOException("Acknowledgement lost");
            if (Pending != null) return Pending.Task;
            var read = command == "native_conversation_rules_status";
            var result = data[read ? Current ? "current" : "stale" : Unchanged ? "unchanged" : "reloaded"]!.DeepClone();
            if (Wrong == "project") result["project_id"] = "other";
            if (Wrong == "session") result["session_id"] = "other";
            if (Wrong == "missing") result.AsObject().Remove(read ? "stale_prompt" : "changed");
            return Task.FromResult<JsonNode?>(result);
        }
    }
}
