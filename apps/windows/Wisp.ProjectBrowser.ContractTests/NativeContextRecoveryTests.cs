using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeContextRecoveryTests
{
    private const string Limit = "api: 400 context_length_exceeded";
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        foreach (var message in new[] { Limit, "http: maximum context length", "api: 400 prompt is too long", "api: 400 too many tokens" })
            Check(NativeContextErrors.IsContextLimit(message), "Explicit provider context failures offer recovery.");
        foreach (var message in new[] { "context window exceeded", "api: 400 bad request", "api: 402 too many tokens",
            "api: 400 insufficient_quota context_length_exceeded", "ACP turn failed: api: 400 context_length_exceeded" })
            Check(!NativeContextErrors.IsContextLimit(message), "Generic, balance and ACP failures do not offer Wisp recovery.");
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        host.Error = Limit;
        await model.OpenAsync("p", "source");
        Check(model.ContextRecovery == null, "A saved failure is not a new active-turn event.");
        await Offer(); model.Draft = "Keep my unsent draft";
        Check(model.CanRecoverContext, "A live provider failure offers scoped recovery.");
        model.DismissContextRecovery(); await model.RefreshAsync();
        Check(model.ContextRecovery == null && model.Draft == "Keep my unsent draft" && host.Sends == 0, "Pause/Escape keep the work and suppress the same event.");
        await Offer(); Check(model.CompactAndContinue() && model.Compaction is { Semantic: true, Started: false } && host.Sends == 0,
            "Compact and continue first opens semantic confirmation.");
        model.DismissCompaction(); await model.RefreshAsync();
        Check(host.Sends == 0, "Cancelling confirmation never resumes.");
        await Offer(); model.CompactAndContinue(); await model.StartCompactionAsync();
        Check(host.Compacts == 1 && host.Resumes == 0, "Running compaction has not resumed yet.");
        host.Running = false; host.Error = "api: 500 unavailable"; await model.RefreshAsync();
        Check(host.Resumes == 0 && model.Compaction?.Error != null, "Failed compaction cannot resume."); model.DismissCompaction();
        await Offer(); model.CompactAndContinue(); await model.StartCompactionAsync();
        var compactRequest = host.Request;
        host.Running = false; host.Error = null; await model.RefreshAsync(); await model.RefreshAsync();
        Check(host.Resumes == 1 && host.LastResume?["resume_after"]?.GetValue<string>() == compactRequest
            && host.LastResume?["message"]?.GetValue<string>() == "" && model.Draft == "Keep my unsent draft",
            "Exactly one empty-message resume follows the matching successful compaction, preserving the draft.");
        await Offer(); model.CompactAndContinue(); await model.StartCompactionAsync();
        await model.OpenAsync("p", "other"); host.Running = false; await model.OpenAsync("p", "source"); await model.RefreshAsync();
        Check(host.Resumes == 1, "Navigation cancels automatic continuation even if compaction subsequently succeeds.");
        await Offer(); model.CompactAndContinue(); await model.StartCompactionAsync();
        host.Running = false; host.Request = Guid.NewGuid().ToString(); await model.RefreshAsync();
        Check(host.Resumes == 1 && model.Compaction is { Busy: false, Error: not null }, "A different request cancels continuation and leaves a dismissible explanation.");
        model.DismissCompaction();
        foreach (var mode in new[] { "acp", "readonly", "running" })
        {
            host.Mode = mode; await Offer(); Check(model.ContextRecovery == null, "Ineligible snapshots do not offer recovery.");
        }
        host.Mode = ""; await Offer();
        await model.StartContextRecoverySessionAsync("Continue the unfinished request", async launch =>
        {
            var id = await model.PrepareContextRecoverySessionAsync(launch);
            await model.OpenAsync("p", id); await model.SendAsync();
        });
        Check(host.Creates == 1 && host.LastSend?["references"]?[0]?["id"]?.GetValue<string>() == "source"
            && host.LastSend?["message"]?.GetValue<string>()?.StartsWith("Continue the unfinished request") == true,
            "New conversation sends only its continuation instruction and canonical source reference.");
        await model.OpenAsync("p", "source"); Check(model.Draft == "Keep my unsent draft", "New conversation leaves the source draft intact.");
        await Offer(); host.FailCreate = true;
        Task Launch(NativeContextRecoveryLaunch value) => model.PrepareContextRecoverySessionAsync(value);
        await model.StartContextRecoverySessionAsync("Continue", Launch); await model.StartContextRecoverySessionAsync("Continue", Launch);
        Check(host.Creates == 2 && model.ContextRecovery is { Uncertain: true, Starting: false }, "Unknown create outcome is shown without automatic/duplicate replay.");
        model.DismissContextRecovery(); host.FailCreate = false;
        await Offer(); host.PendingCreate = new();
        var creating = model.StartContextRecoverySessionAsync("Late instruction", async launch =>
        {
            var id = await model.PrepareContextRecoverySessionAsync(launch);
            if (!model.RecoverySourceMatches(launch.Source)) return;
            await model.OpenAsync("p", id); await model.SendAsync();
        });
        var sends = host.Sends; await model.OpenAsync("p", "other"); host.PendingCreate.SetResult(JsonValue.Create("late")); await creating;
        Check(host.Sends == sends && model.CurrentSessionId == "other" && model.ContextRecovery == null, "A late creation cannot send into or navigate the newly selected conversation.");
        host.PendingCreate = null; await model.OpenAsync("p", "source"); await Offer(); model.CompactAndContinue(); await model.StartCompactionAsync();
        host.FailResume = true; host.Running = false; host.Error = null; await model.RefreshAsync();
        Check(model.UncertainSend, "Unknown resume blocks input and is never automatically repeated.");
        sends = host.Sends; await model.RefreshAsync(); await model.OpenAsync("p", "other"); await model.OpenAsync("p", "source");
        Check(model.UncertainSend && host.Sends == sends && model.Draft == "Keep my unsent draft", "Unconfirmed resume survives navigation without touching composer data.");

        async Task Offer()
        {
            host.Error = null; host.Running = false; await model.RefreshAsync();
            host.Request = Guid.NewGuid().ToString(); host.Error = Limit; await model.RefreshAsync();
        }
    }
    private sealed class Fake : INativeSettingsClient
    {
        public string? Error; public string Request = Guid.NewGuid().ToString(), Mode = ""; public bool Running, FailCreate, FailResume;
        public int Sends, Compacts, Resumes, Creates; public JsonObject? LastResume, LastSend;
        public TaskCompletionSource<JsonNode?>? PendingCreate;
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            JsonNode? result = null;
            if (command == "native_conversation_snapshot")
            {
                var id = args["session_id"]!.GetValue<string>(); var source = id == "source";
                result = JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId, "e", ++sequence, projectId!, id,
                    source ? [new("user", "Analyze results", null, null, null, null), new("assistant", "Completed work", null, null, null, null)] : [],
                    null, source && (Running || Mode == "running"), false, source && Mode == "readonly", Mode == "acp" ? "acp:a" : "m",
                    source ? Request : null, source ? Error : null, [], ComposerReferences: true, AcpAgentId: Mode == "acp" ? "a" : null), ConversationSnapshot.JsonOptions);
            }
            else if (command == "native_conversation_send")
            {
                Sends++;
                if (args["resume"]?.GetValue<bool>() == true)
                { Resumes++; LastResume = (JsonObject)args.DeepClone(); if (FailResume) throw new IOException("Unknown resume result"); }
                else if (args["message"]!.GetValue<string>().StartsWith("/compact")) { Compacts++; Running = true; }
                else LastSend = (JsonObject)args.DeepClone();
                Request = args["request_id"]!.GetValue<string>(); Error = null;
                result = new JsonObject();
            }
            else if (command == "native_conversation_create")
            { Creates++; if (FailCreate) throw new IOException("Unknown creation result"); return PendingCreate?.Task ?? Task.FromResult<JsonNode?>(JsonValue.Create("fresh")); }
            else if (command is "list_models" or "list_acp_agents") result = new JsonArray();
            return Task.FromResult(result);
        }
    }
}
