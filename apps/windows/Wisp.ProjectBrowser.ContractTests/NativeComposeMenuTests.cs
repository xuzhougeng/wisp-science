using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeComposeMenuTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        Check(NativeComposeMenu.Entries.Select(e => e.Id).SequenceEqual(new[] { "attach", "files", "review", "share", "save-skill", "skills" }), "compose actions follow the shared WebView order");
        Check(NativeComposeMenu.Entries.Select(e => e.Icon).Distinct().Count() == 6, "each compose action has a distinct semantic icon");
        var host = new Fake(); var assistant = new NativeAssistantModel(host);
        await assistant.OpenAsync(); assistant.SelectProject("p");
        var model = assistant.Conversation;
        Check(model.ComposeSessionReady && !model.CanShareConversation && model.CanReviewConversation, "empty conversation cannot share but can request the backend's review result");
        await model.AttachFilesAsync(["notes.csv"]); model.Draft = "previous draft";
        model.DraftSkill("Turn this conversation into a reusable skill.");
        Check(model.Draft == "Turn this conversation into a reusable skill." && model.Attachments.Length == 1
            && host.Calls.All(c => c.Command != "native_conversation_send"), "save skill fills a draft without sending or dropping attachments");
        host.Populated = true; await model.RefreshAsync(); Check(model.CanShareConversation, "a populated conversation can open sharing");
        var files = assistant.CreateFiles(); await files.RefreshAsync("files");
        var share = assistant.CreateShare(); await share.LoadAsync();
        Check(files.Error == null && share.Error == null && share.Rows.Length == 2, "assistant files and share surfaces load");
        Check(host.Calls.Where(c => c.Command is "native_conversation_panel_file_locations" or "native_conversation_panel_file_directory" or "native_conversation_share")
            .All(c => c.Project == NativeAssistantClient.ProjectId && c.Args["session_id"]?.GetValue<string>() == NativeAssistantClient.SessionId),
            "files and share use the assistant conversation even with a project context selected");
        host.PendingReview = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var reviewing = model.ReviewConversationAsync(); await model.ReviewConversationAsync();
        Check(model.HistoryBusy && model.Busy && !model.CanReviewConversation && host.Reviews == 1, "pending review blocks duplicate submissions");
        var request = host.Calls.Single(c => c.Command == "native_conversation_review");
        Check(request.Project == NativeAssistantClient.ProjectId && request.Args["session_id"]?.GetValue<string>() == NativeAssistantClient.SessionId
            && !request.Args.ContainsKey("target"), "review targets the current assistant session without a fake historical turn");
        host.PendingReview.SetException(new IOException("reply lost")); await reviewing;
        Check(model.HistoryUncertain && !model.CanReviewConversation, "an ambiguous review is not replayable until acknowledged");
        await model.ReviewConversationAsync(); Check(host.Reviews == 1, "polling or clicking again cannot replay an uncertain review");
        model.AcknowledgeHistoryResult(); host.PendingReview = null;
        host.Running = true; await model.RefreshAsync(); Check(!model.CanReviewConversation, "review waits for a running turn");
        host.Running = false; await model.RefreshAsync();
        host.PendingReview = new(TaskCreationOptions.RunContinuationsAsynchronously);
        reviewing = model.ReviewConversationAsync(); assistant.Close();
        await assistant.OpenAsync(); var snapshot = model.Snapshot;
        host.PendingReview.SetException(new IOException("late error")); await reviewing;
        Check(model.Snapshot == snapshot && !model.Busy && !model.HistoryBusy && model.OperationError == null,
            "a late review response cannot replace the reopened conversation or its error state");
        assistant.Close(); Check(!model.ComposeSessionReady && !model.CanShareConversation, "closed assistant does not expose session actions");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public readonly List<(string Command, string? Project, JsonObject Args)> Calls = [];
        public bool Populated, Running;
        public int Reviews;
        public TaskCompletionSource<JsonNode?>? PendingReview;
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, projectId, (JsonObject)args.DeepClone()));
            JsonNode? reply = command switch {
                "native_assistant_open" => new JsonObject { ["project_id"] = NativeAssistantClient.ProjectId, ["session_id"] = NativeAssistantClient.SessionId },
                "native_assistant_workspace" => new JsonObject { ["day"] = args["day"]!.DeepClone(), ["projects"] = new JsonArray(new JsonObject { ["id"] = "p", ["name"] = "Context", ["description"] = "" }), ["plan"] = new JsonArray() },
                "native_conversation_snapshot" => JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId, "menu", ++sequence, projectId!, NativeAssistantClient.SessionId,
                    Populated ? [new("user", "question", null, null, null, null), new("assistant", "answer", null, null, null, null)] : [],
                    null, Running, false, false, "", null, null, [], ComposerReferences: true), ConversationSnapshot.JsonOptions),
                "native_conversation_attach" => new JsonObject { ["path"] = "uploads/notes.csv", ["name"] = "notes.csv" },
                "native_conversation_share" => JsonSerializer.SerializeToNode(new NativeShareRow[] { new("user", "question"), new("assistant", "answer") }, ConversationSnapshot.JsonOptions),
                "native_conversation_panel_file_locations" => JsonSerializer.SerializeToNode(new NativeFileLocations(NativeFilesClient.SchemaId, projectId!, NativeAssistantClient.SessionId, "/assistant", false, [new("local", "", "local")]), ConversationSnapshot.JsonOptions),
                "native_conversation_panel_file_directory" => JsonSerializer.SerializeToNode(new NativeFileDirectory(NativeFilesClient.SchemaId, projectId!, NativeAssistantClient.SessionId, "local", ".", []), ConversationSnapshot.JsonOptions),
                "native_research_calendar" or "list_models" or "list_acp_agents" or "native_conversation_panel_files" => new JsonArray(),
                _ => null
            };
            if (command == "native_conversation_review") { Reviews++; if (PendingReview != null) return PendingReview.Task; }
            return Task.FromResult(reply);
        }
    }
}
