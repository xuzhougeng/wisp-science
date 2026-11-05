using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSidebarReferenceTests
{
    public static async Task RunAsync()
    {
        var host = new Host(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        var scope = new NativeSessionReferenceScope("p", "Research", [
            new("other", "p", "Earlier research", 20, "idle", "folder", true),
            new("current", "p", "Current", 19, "idle", "folder"),
            new("last", "p", "Later research", 18, "idle", "folder"),
            new("last", "p", "Duplicate snapshot row", 17, "idle", "folder"),
            new("outside", "p", "Another group", 16, "idle", "elsewhere"),
            new("foreign", "q", "Another project", 15, "idle", "folder")]);
        var folder = new NativeSessionDrag("p", "folder", "folder");
        await model.OpenAsync("p", "current"); var revision = model.FileReferenceRevision;
        model.Draft = "Keep the typed text"; var calls = host.Commands.Count;
        Check(model.ReferenceSidebarSessions(folder, scope, revision) == 2
            && model.References.Select(r => r.Reference.Id).SequenceEqual(["other", "last"])
            && model.References[0].Label == "Earlier research" && model.References[0].Detail == "Research",
            "folder resolves current catalog order, includes pinned members, excludes current/foreign/other-folder sessions and duplicates");
        Check(model.Draft == "Keep the typed text" && host.Commands.Count == calls,
            "referencing never sends, uploads, creates or replaces draft text");
        Check(model.ReferenceSidebarSessions(folder, scope, revision) == 0
            && model.ReferenceSidebarSessions(new("p", "session", "current"), scope, revision) == 0
            && model.ReferenceSidebarSessions(new("p", "session", "missing"), scope, revision) == 0
            && model.ReferenceSidebarSessions(new("p", "folder", "missing"), scope, revision) == 0,
            "repeated/current/unknown drops add no references");
        Check(model.ReferenceSidebarSessions(folder with { ProjectId = "q" }, scope, revision) == 0
            && model.ReferenceSidebarSessions(folder, scope with { ProjectId = "q" }, revision) == 0
            && model.ReferenceSidebarSessions(folder with { Kind = "text" }, scope, revision) == 0,
            "foreign or unrecognized payloads cannot supply references");
        Check(model.ReferenceSidebarSessions(new("p", "session", "outside"), scope, revision) == 1
            && model.References.Last().Reference.ToJson()["kind"]!.GetValue<string>() == "session",
            "single-session references use the existing typed host contract");
        host.ReadOnly = true; await model.RefreshAsync();
        model.RemoveReference("session:other");
        Check(!model.CanRemoveReference && model.References.Length == 3, "read-only references cannot be removed");
        Check(model.ReferenceSidebarSessions(new("p", "session", "current"), scope, revision) == 0 && !model.CanReceiveSessionReference,
            "read-only conversations reject drops");
        host.ReadOnly = false; host.SupportsReferences = false; await model.RefreshAsync();
        Check(!model.CanReceiveSessionReference, "older hosts do not expose reference drops");
        host.SupportsReferences = true;
        await model.OpenProjectDraftAsync("p"); calls = host.Commands.Count;
        Check(model.ReferenceSidebarSessions(folder, scope, revision) == 0, "a delayed drop cannot cross conversation navigation");
        revision = model.FileReferenceRevision;
        Check(model.ReferenceSidebarSessions(folder, scope, revision) == 3 && model.CanSubmit
            && model.CurrentSessionId == null && host.Commands.Count == calls,
            "project draft can stage session references without creating a conversation");
        Check(model.CanRemoveReference, "draft reference removal remains available before session creation");
        model.RemoveReference("session:other");
        Check(model.References.Length == 2 && host.Commands.Count == calls, "removing a draft chip has no host mutation");
        model.ReferenceSidebarSessions(new("p", "session", "other"), scope, revision);
        await model.OpenProjectDraftAsync("q");
        Check(model.References.Length == 0 && model.ReferenceSidebarSessions(folder, scope, revision) == 0,
            "delayed data cannot cross projects");
        await model.OpenProjectDraftAsync("p");
        Check(model.References.Length == 3 && model.ReferenceSidebarSessions(folder, scope, revision) == 0,
            "draft references persist but old data reads stay invalid even after returning to the same project");
        Check(await model.EnsureDraftSessionAsync() == "created" && model.References.Length == 3 && host.Creates == 1,
            "explicit draft creation carries all staged session references forward");
        await model.OpenProjectDraftAsync("limit");
        var many = new NativeSessionReferenceScope("limit", "Large group", Enumerable.Range(0, 70)
            .Select(i => new BrowserSession(i.ToString(), "limit", "Conversation " + i, i, "idle", "folder")).ToArray());
        Check(model.ReferenceSidebarSessions(new("limit", "folder", "folder"), many, model.FileReferenceRevision) == 64
            && model.References.Length == 64, "folder drops respect the existing composer reference limit");
        model.Pause();
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Host : INativeSettingsClient
    {
        public bool ReadOnly, SupportsReferences = true;
        public int Creates;
        public List<string> Commands = [];
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Commands.Add(command);
            if (command == "native_conversation_create") Creates++;
            object? result = command switch {
                "native_conversation_snapshot" => new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", ++sequence, projectId!, args["session_id"]!.GetValue<string>(), [], null, false, false, ReadOnly, "model", null, null, [], ComposerReferences: SupportsReferences),
                "native_conversation_create" => "created", "list_models" => new JsonArray(), _ => null };
            return Task.FromResult(result is JsonNode node ? node : JsonSerializer.SerializeToNode(result, ConversationSnapshot.JsonOptions));
        }
    }
}
