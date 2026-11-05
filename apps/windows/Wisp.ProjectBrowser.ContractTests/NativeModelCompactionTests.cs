using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeModelCompactionTests
{
    public static async Task RunAsync()
    {
        await ManualEntryAsync();
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s"); model.Draft = "Keep my draft"; model.AddRuntimeQuote("Keep my quote");
        Check(await model.SelectModelAsync("second") && host.Commands.SequenceEqual(new[] { "model", "/compact --semantic" })
            && model.Compaction is { Busy: true, Acknowledged: true } && !model.CanSend && !model.CanAttach,
            "enabled model switch starts semantic compaction after the model write");
        Check(model.Draft == "Keep my draft" && model.Quotes.Length == 1, "compaction never consumes the composer draft or context");
        model.DismissCompaction(); Check(model.Compaction?.Visible == true, "running compaction cannot be dismissed by Escape");
        await model.StartCompactionAsync(); Check(host.Sends == 1, "a busy compaction cannot be submitted twice");
        host.Running = false; await model.RefreshAsync();
        Check(model.Compaction == null && model.CanSend && model.Draft == "Keep my draft", "authoritative completion closes the modal and restores the composer");

        host.Enabled = false; await model.SelectModelAsync("first"); Check(host.Sends == 1, "disabled preference never compacts");
        host.Enabled = true; host.Items = false; await model.RefreshAsync(); await model.SelectModelAsync("second");
        Check(host.Sends == 1, "empty conversation never compacts");
        host.Items = true; host.FailModel = true; await model.RefreshAsync();
        Check(!await model.SelectModelAsync("first") && host.Sends == 1, "rejected model change never compacts");
        host.FailModel = false; host.LoseReply = true;
        await model.SelectModelAsync("first");
        Check(model.Compaction is { Uncertain: true, Busy: false } && !model.CanSend, "unconfirmed send keeps the draft blocked without automatic replay");
        var request = host.LastRequest; await model.RefreshAsync(); await model.RefreshAsync();
        Check(host.Sends == 2, "polling does not replay compaction");
        model.DismissCompaction(); Check(model.Compaction?.Visible == false && !model.CanSend, "closing an unconfirmed dialog retains its request guard");
        model.ShowCompaction(); host.LoseReply = false;
        await model.StartCompactionAsync(false, "different instruction");
        Check(host.LastRequest == request && host.Commands.Last() == "/compact --semantic", "explicit uncertain retry uses the same request and exact payload");
        host.Running = false; host.Error = "Synthetic compaction failure"; await model.RefreshAsync();
        Check(model.Compaction is { Busy: false, Uncertain: false, Error: "Synthetic compaction failure" }, "terminal failure unlocks retry and cancel");
        await model.StartCompactionAsync(false, " preserve paths ");
        Check(host.LastRequest != request && host.Commands.Last() == "/compact preserve paths", "retry after confirmed failure starts a new explicitly chosen operation");
        host.Running = false; await model.RefreshAsync();

        var held = host.SettingsHold = new(); var selection = model.SelectModelAsync("second");
        await model.OpenAsync("q", "other"); var writes = host.Writes;
        held.SetResult(new JsonObject { ["semantic_compact_on_model_switch"] = true });
        Check(!await selection && host.Writes == writes && model.Compaction == null, "late preference read cannot switch or compact the new session");

        await model.OpenAsync("p", "s");
        host.SendHold = new(); var switching = model.SelectModelAsync("second");
        await model.OpenAsync("q", "other"); host.SendHold.SetResult(null); await switching;
        Check(model.CurrentSessionId == "other" && model.Compaction == null, "late send response cannot open a modal in another session");
        await model.OpenAsync("p", "s"); Check(model.Compaction?.Busy == true, "returning to the original running compaction restores its modal");
        host.Running = false; host.Error = "Retryable failure"; await model.RefreshAsync(); model.DismissCompaction();
        Check(model.Compaction == null, "terminal failure can be dismissed");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel(); var sends = host.Sends;
        await model.StartCompactionAsync(token: cancelled.Token); Check(host.Sends == sends, "pre-cancelled operation is never sent");
    }
    private static async Task ManualEntryAsync()
    {
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s"); model.Draft = "Keep this draft"; model.AddRuntimeQuote("Keep this context");
        Check(model.OpenCompaction() && model.Compaction is { Started: false, Busy: false, Semantic: false, Error: null }
            && host.Sends == 0 && model.Draft == "Keep this draft", "usage entry opens a regular confirmation without sending or consuming the draft");
        await model.RefreshAsync(); Check(model.Compaction is { Started: false }, "polling cannot treat an unsent dialog as completed");
        model.UpdateCompactionDraft(true, "Preserve paths\nand QC blockers");
        Check(model.Compaction is { Semantic: true, Instruction: "Preserve paths\nand QC blockers" }, "dialog editing persists in the session state across page remounts");
        model.DismissCompaction(); Check(host.Sends == 0 && model.Compaction == null && model.Quotes.Length == 1, "Escape/cancel before Start has no backend side effect");
        model.Draft = "/compact";
        Check(model.HandleCompactionCommand(model.Draft) && model.Draft == "" && model.Compaction is { Semantic: false, Started: false }
            && model.Quotes.Length == 1 && host.Sends == 0, "slash command consumes only its text and opens regular confirmation");
        model.DismissCompaction(); model.Draft = "/compact preserve exact file paths";
        Check(model.HandleCompactionCommand(model.Draft) && model.Compaction is { Semantic: true, Instruction: "preserve exact file paths" }, "instruction selects semantic mode without sending");
        await model.StartCompactionAsync(model.Compaction!.Semantic, model.Compaction.Instruction);
        Check(host.Sends == 1 && host.Commands.Last() == "/compact --semantic preserve exact file paths", "Start sends the reviewed semantic instruction once");
        host.Running = false; await model.RefreshAsync();
        host.ReadOnly = true; await model.RefreshAsync();
        Check(!model.OpenCompaction(), "read-only sessions cannot open a compaction operation");
        await model.StartCompactionAsync(); Check(host.Sends == 1, "read-only sessions cannot start compaction");
        host.ReadOnly = false; host.Acp = "agent"; await model.RefreshAsync(); model.Draft = "/compact";
        Check(model.HandleCompactionCommand(model.Draft) && model.Compaction == null && model.OperationError!.StartsWith("ACP conversations"), "ACP slash command explains the limitation without sending");
        host.Acp = null; host.Running = true; await model.RefreshAsync(); model.Draft = "/compact";
        Check(model.HandleCompactionCommand(model.Draft) && model.Compaction == null && model.Draft == "/compact" && host.Sends == 1,
            "a running session retains an unexecuted command and never starts a concurrent compaction");
        host.Running = false; await model.OpenProjectDraftAsync("p"); model.Draft = "/compact";
        Check(model.HandleCompactionCommand(model.Draft) && model.CurrentSessionId == null && model.Compaction == null && host.Creates == 0
            && model.OperationError == "Open a conversation before compacting its context.", "project drafts show the prerequisite and do not allocate a frame");
        await model.OpenAsync("p", "s"); model.Pause(); await model.StartCompactionAsync();
        Check(host.Sends == 1 && !model.CanOpenCompaction, "a paused page cannot initiate compaction");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public bool Enabled = true, Items = true, Running, LoseReply, FailModel, ReadOnly;
        public int Sends, Writes, Creates; public string? LastRequest, Request, Error, Acp; public string Selected = "first";
        public TaskCompletionSource<JsonNode?>? SettingsHold, SendHold;
        public List<string> Commands = []; private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "native_conversation_create") { Creates++; throw new InvalidOperationException("Compaction must not create a session"); }
            if (command == "get_settings") { if (SettingsHold is { } hold) { SettingsHold = null; return hold.Task; }
                return Task.FromResult<JsonNode?>(new JsonObject { ["semantic_compact_on_model_switch"] = Enabled }); }
            if (command == "list_models") return Task.FromResult(JsonNode.Parse("""[{"id":"first","label":"First","model":"chat-a"},{"id":"second","label":"Second","model":"chat-b"}]"""));
            if (command == "native_conversation_snapshot") return Task.FromResult(JsonSerializer.SerializeToNode(
                new ConversationSnapshot(ConversationSnapshot.SchemaId, "memory", ++sequence, projectId!, args["session_id"]!.GetValue<string>(),
                    Items ? [new("user", "Prior research", null, null, null, null)] : [], null, Running, false, ReadOnly, Selected, Request, Error, [], AcpAgentId: Acp), ConversationSnapshot.JsonOptions));
            if (command == "native_conversation_model") { Writes++; if (FailModel) throw new IOException("Model failed"); Selected = args["model_id"]!.GetValue<string>(); Commands.Add("model"); }
            if (command == "native_conversation_send")
            {
                Sends++; LastRequest = args["request_id"]!.GetValue<string>(); Commands.Add(args["message"]!.GetValue<string>());
                Check(!args.ContainsKey("attachments") && !args.ContainsKey("references") && !args.ContainsKey("acp_agent_id"), "compaction has no composer payload");
                if (LoseReply) throw new IOException("Synthetic connection loss");
                Request = LastRequest; Running = true; Error = null;
                if (SendHold is { } hold) return hold.Task;
            }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
