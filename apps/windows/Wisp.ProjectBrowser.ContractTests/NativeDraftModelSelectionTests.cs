using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeDraftModelSelectionTests
{
    public static async Task RunAsync()
    {
        await DraftFastAsync();
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenProjectDraftAsync("p"); model.Draft = "Keep this draft"; model.AddRuntimeQuote("Keep this quote");
        Check(!await model.SelectDraftModelAsync("missing") && !await model.SelectDraftModelAsync("image") && host.Writes == 0,
            "unknown and media-only profiles cannot become a draft chat model");
        Check(await model.SelectDraftModelAsync("second") && model.HasProjectDraft && model.ComposerModelId == "second"
            && host.Writes == 1 && host.Creates == 0 && host.Sends == 0 && !host.LastArguments!.ContainsKey("sessionId"),
            "draft HTTP selection updates the shared default without a frame, session override or send");
        Check(model.Draft == "Keep this draft" && model.Quotes.Single().Text == "Keep this quote" && model.Effort.Value == "high",
            "selection retains draft/context and binds the selected profile's effort");
        model.SelectProvisionalAgent("a");
        Check(await model.SelectDraftModelAsync("first") && model.ComposerAgentId == null && model.ComposerModelId == "first"
            && host.Creates == 0, "confirmed HTTP default selection clears provisional ACP without creation");
        await model.SelectDraftModelAsync("second");
        await model.OpenProjectDraftAsync("q");
        Check(model.ComposerModelId == "second", "other project drafts inherit the shared default");
        await model.OpenProjectDraftAsync("p");
        Check(model.Draft == "Keep this draft" && model.Quotes.Length == 1, "returning to a project preserves its own draft");
        var id = await model.EnsureDraftSessionAsync(); await model.SendAsync();
        Check(id == "created-1" && host.Creates == 1 && host.Sends == 1 && host.SentModel == "second",
            "first explicit send creates exactly one frame bound to the selected default");
        Check(!await model.SelectDraftModelAsync("first") && model.ComposerModelId == "second",
            "draft-default API cannot change an existing session");

        host = new(); model = new(new NativeConversationClient(host), host); await model.OpenProjectDraftAsync("p");
        model.Draft = "Failure draft"; model.SelectProvisionalAgent("a"); host.Fail = true;
        Check(!await model.SelectDraftModelAsync("second") && model.ComposerAgentId == "a" && model.Draft == "Failure draft"
            && model.DraftModelFailureDetail == "Synthetic response unavailable" && !model.Busy && host.Creates == 0,
            "failed default change preserves provisional ACP and editable draft");
        await model.RefreshAsync(); Check(host.Writes == 1, "polling does not replay an uncertain model change");
        host.Fail = false; Check(model.RestoreProvisionalHttpModel("first"), "original HTTP choice remains available after failure");
        host.WrongReply = true;
        Check(!await model.SelectDraftModelAsync("second") && model.ComposerModelId == "first" && model.OperationError != null,
            "a successful transport with a mismatched active model is not displayed as confirmation");
        host.WrongReply = false; host.Active = "second";
        await model.ReloadComposerCatalogAsync();
        Check(model.ComposerModelId == "second" && model.OperationError == null && host.Writes == 2 && host.Creates == 0,
            "explicit reload reconciles authoritative default without another write or creation");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        Check(!await model.SelectDraftModelAsync("first", cancelled.Token) && host.Writes == 2, "pre-cancelled selection sends no request");

        var lateWrite = host.WriteHold = new(); var selecting = model.SelectDraftModelAsync("first");
        await model.OpenProjectDraftAsync("q"); model.Draft = "Other draft";
        lateWrite.SetResult(host.Catalog("first"));
        Check(!await selecting && model.CurrentProjectId == "q" && model.ComposerModelId == "second" && model.Draft == "Other draft" && !model.Busy,
            "late write response cannot change another draft cursor or its displayed model");

        await model.OpenProjectDraftAsync("p");
        var lateRead = host.ReadHold = new(); var reading = model.ReloadComposerCatalogAsync();
        Check(await model.SelectDraftModelAsync("first"), "selection may proceed while a previous catalog read is pending");
        lateRead.SetResult(host.Catalog("second")); await reading;
        Check(model.ComposerModelId == "first", "a stale catalog read cannot overwrite a newer confirmed selection");
    }

    private static async Task DraftFastAsync()
    {
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenProjectDraftAsync("p"); model.Draft = "Retain until explicitly sent";
        Check(model.CanChangeFastMode && model.ComposerFastMode is { Enabled: false, Inherited: true }, "draft reads its HTTP profile default");
        await model.SetFastModeAsync(true);
        await model.OpenProjectDraftAsync("q");
        Check(model.ComposerFastMode is { Enabled: false, Inherited: true }, "a draft override is private to its project");
        await model.OpenProjectDraftAsync("p");
        Check(model.ComposerFastMode is { Enabled: true, Inherited: false } && host.Creates == 0 && host.FastWrites == 0, "returning restores only the staged preference");
        await model.SetFastModeAsync(false);
        Check(model.ComposerFastMode is { Inherited: true }, "returning to the profile default clears the override");
        await model.SetFastModeAsync(true); model.SelectProvisionalAgent("a");
        Check(!model.CanChangeFastMode && model.ComposerFastMode == null, "ACP does not expose HTTP priority");
        model.RestoreProvisionalHttpModel("first");
        Check(model.ComposerFastMode is { Enabled: false, Inherited: true }, "ACP selection clears the draft override");
        await model.SelectDraftModelAsync("second");
        Check(model.ComposerFastMode is { Enabled: true, Inherited: true }, "reads the new profile's priority default");
        await model.SetFastModeAsync(false);
        var id = await model.EnsureDraftSessionAsync();
        Check(id != null && host.Creates == 1 && host.FastWrites == 1 && host.Sends == 0 && model.Draft.Length > 0
            && model.ComposerFastMode is { Enabled: false, Inherited: false }, "explicit creation confirms a session-only override without sending");
        await model.SendAsync(); Check(host.Sends == 1 && host.FastWrites == 1, "send uses the already confirmed tier");

        host = new() { FailFast = true }; model = new(new NativeConversationClient(host), host);
        await model.OpenProjectDraftAsync("p"); model.Draft = "Do not lose me"; await model.SetFastModeAsync(true);
        Check(await model.EnsureDraftSessionAsync() == null && model.CurrentSessionId == "created-1"
            && model.DraftFastFailureDetail != null && model.Draft == "Do not lose me" && host.Sends == 0,
            "a failed tier change stops submission while retaining the confirmed frame and draft");
        await model.RefreshAsync(); Check(host.FastWrites == 1 && host.Creates == 1, "reloading never replays tier writes or creates another frame");
        Check(!NativeModelPresentation.SupportsFast(new() { ["provider"] = "anthropic" })
            && !NativeModelPresentation.SupportsFast(new() { ["provider"] = "openai", ["model"] = "gpt-image-2" }), "unsupported providers and media models have no Fast toggle");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public int Writes, Creates, Sends;
        public int FastWrites;
        public bool FailFast;
        private readonly Dictionary<string, bool> tiers = [];
        public bool Fail, WrongReply;
        public string Active = "first";
        public string? SentModel;
        public JsonObject? LastArguments;
        public TaskCompletionSource<JsonNode?>? WriteHold, ReadHold;
        private readonly Dictionary<string, string> bindings = [];
        private ulong sequence;
        public JsonArray Catalog(string active) => new(
            new JsonObject { ["id"] = "first", ["label"] = "First", ["model"] = "chat-a", ["provider"] = "openai", ["active"] = active == "first", ["reasoning_effort"] = "low" },
            new JsonObject { ["id"] = "second", ["label"] = "Second", ["model"] = "chat-b", ["provider"] = "responses", ["service_tier"] = "priority", ["active"] = active == "second", ["reasoning_effort"] = "high" },
            new JsonObject { ["id"] = "image", ["model"] = "gpt-image-2" });
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "list_models")
            { if (ReadHold is { } hold) { ReadHold = null; return hold.Task; } return Task.FromResult<JsonNode?>(Catalog(Active)); }
            if (command == "list_acp_agents") return Task.FromResult(JsonNode.Parse("""[{"id":"a","label":"Agent A"}]"""));
            if (command == "model_catalog_lookup") return Task.FromResult(JsonNode.Parse("""{"efforts":["low","high"]}"""));
            if (command == "set_active_model")
            {
                Writes++; LastArguments = (JsonObject)args.DeepClone();
                if (Fail) throw new IOException("Synthetic response unavailable");
                if (WriteHold is { } hold) { WriteHold = null; return hold.Task; }
                if (!WrongReply) Active = args["id"]!.GetValue<string>();
                return Task.FromResult<JsonNode?>(Catalog(Active));
            }
            if (command == "native_conversation_create")
            { var id = "created-" + ++Creates; bindings[id] = Active; return Task.FromResult<JsonNode?>(JsonValue.Create(id)); }
            var session = args["session_id"]?.GetValue<string>() ?? "";
            var profileDefault = bindings.GetValueOrDefault(session, Active) == "second";
            if (command == "native_conversation_fast")
            {
                FastWrites++;
                if (FailFast) throw new IOException("Synthetic tier response unavailable");
                var enabled = args["enabled"]!.GetValue<bool>();
                if (enabled == profileDefault) tiers.Remove(session); else tiers[session] = enabled;
                return Task.FromResult<JsonNode?>(new JsonObject { ["enabled"] = enabled, ["inherited"] = !tiers.ContainsKey(session) });
            }
            if (command == "native_conversation_snapshot") return Task.FromResult(JsonSerializer.SerializeToNode(
                new ConversationSnapshot(ConversationSnapshot.SchemaId, "memory", ++sequence, projectId!, session, [], null,
                    false, false, false, bindings.GetValueOrDefault(session, Active), null, null, [])
                    { FastMode = new(tiers.GetValueOrDefault(session, profileDefault), !tiers.ContainsKey(session)) }, ConversationSnapshot.JsonOptions));
            if (command == "native_conversation_send") { Sends++; SentModel = bindings[session]; }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
