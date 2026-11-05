using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativePermissionTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var model = new NativeSettingsEditorModel(fake, "current-project");
        Check(!await model.SetApprovalScopeAsync("auto") && !await model.RevokeAllApprovalsAsync() && fake.Calls.Count == 0, "unloaded permissions cannot issue writes");
        await model.LoadAsync("list_connectors", "list_approval_grants");
        Check(model.ApprovalScope == "ask" && model.ApprovalGrants.Length == 3, "permissions retain the host mode and every remembered scope");
        var calls = fake.Calls.Count;
        Check(!await model.SetApprovalScopeAsync("invalid") && await model.SetApprovalScopeAsync("ask") && fake.Calls.Count == calls, "invalid or unchanged modes never issue writes");
        fake.Pending = new(); var change = model.SetApprovalScopeAsync("auto"); calls = fake.Calls.Count;
        Check(model.ApprovalScope == "ask" && !await model.SetApprovalScopeAsync("full") && fake.Calls.Count == calls, "pending mode keeps confirmed selection and rejects a duplicate write");
        fake.Pending.SetResult(null); await change; fake.Pending = null;
        Check(model.ApprovalScope == "auto" && fake.Calls[^1].Project == "current-project" && fake.Calls[^1].Args["scope"]!.ToString() == "auto", "acknowledged mode uses the mounted project context");
        model.Localization = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>(), "en");
        fake.Fail = true; calls = fake.Calls.Count;
        Check(!await model.SetApprovalScopeAsync("full") && model.ApprovalScope == "auto" && fake.Calls.Count == calls + 1
            && model.Error == "The operation could not be confirmed; it will not be retried automatically.\nUnconfirmed write", "failed mode change retains selection and localized error without automatic retry");
        fake.Fail = false;
        Check(await model.SetApprovalScopeAsync("full") && model.ApprovalScope == "full", "explicit advanced mode changes use the same existing command");
        var exact = (JsonObject)model.ApprovalGrants[1].DeepClone();
        var unknown = (JsonObject)exact.DeepClone(); unknown["project_id"] = "missing"; calls = fake.Calls.Count;
        Check(!await model.RevokeApprovalAsync(unknown) && fake.Calls.Count == calls, "unknown grant identity cannot revoke a similarly named record");
        fake.Fail = true;
        Check(!await model.RevokeApprovalAsync(exact) && model.ApprovalGrants.Length == 3, "failed revoke retains the visible rows");
        fake.Fail = false;
        Check(await model.RevokeApprovalAsync(exact) && model.ApprovalGrants.Length == 2
            && model.ApprovalGrants.Any(row => row["project_id"]?.ToString() == "project-a"), "revoke removes only its exact scope, kind, target and owner");
        var sent = fake.Calls[^1];
        Check(sent.Command == "revoke_approval_grant" && sent.Project == "current-project" && sent.Args["projectId"]!.ToString() == "project-b"
            && sent.Args.ContainsKey("sessionId") && sent.Args["sessionId"] == null && !sent.Args.ContainsKey("label"), "grant owner is distinct from request context and uses Tauri casing");
        calls = fake.Calls.Count;
        Check(!await model.RevokeApprovalAsync(exact) && fake.Calls.Count == calls, "stale row cannot replay its acknowledged revoke");
        var session = model.ApprovalGrants.Single(row => row["scope"]!.ToString() == "session");
        Check(await model.RevokeApprovalAsync(session) && fake.Calls[^1].Args["sessionId"]!.ToString() == "session-a", "session grant retains its session identity");
        fake.Fail = true;
        Check(!await model.RevokeAllApprovalsAsync() && model.ApprovalGrants.Length == 1, "failed revoke-all preserves the confirmed count");
        fake.Fail = false;
        Check(await model.RevokeAllApprovalsAsync() && model.ApprovalGrants.Length == 0, "confirmed revoke-all empties the list");
        calls = fake.Calls.Count; Check(!await model.RevokeAllApprovalsAsync() && fake.Calls.Count == calls, "empty revoke-all is disabled in the model too");
        var delayed = new Fake(); var closed = new NativeSettingsEditorModel(delayed, "other");
        await closed.LoadAsync("list_connectors"); delayed.Pending = new(); var pending = closed.SetApprovalScopeAsync("auto");
        closed.Dispose(); delayed.Pending.SetResult(null); await pending;
        Check(closed.ApprovalScope == "ask", "late acknowledgements cannot update a closed page");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool Fail; public TaskCompletionSource<JsonNode?>? Pending;
        public readonly List<(string Command, JsonObject Args, string? Project)> Calls = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)args.DeepClone(), projectId));
            if (command == "list_connectors") return Task.FromResult<JsonNode?>(new JsonObject { ["scope"] = "ask" });
            if (command == "list_approval_grants") return Task.FromResult<JsonNode?>(new JsonArray(
                Grant("project", "project-a", null), Grant("project", "project-b", null), Grant("session", null, "session-a")));
            if (Fail) return Task.FromException<JsonNode?>(new IOException("Unconfirmed write"));
            return Pending?.Task ?? Task.FromResult<JsonNode?>(null);
        }
        private static JsonObject Grant(string scope, string? project, string? session) => new()
        { ["scope"] = scope, ["kind"] = "tool", ["target"] = "read_file", ["label"] = "Read files", ["project_id"] = project, ["session_id"] = session };
    }
}
