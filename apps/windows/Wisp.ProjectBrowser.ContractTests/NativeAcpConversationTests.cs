using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeAcpConversationTests
{
    public static async Task RunAsync()
    {
        var transport = new Fake(); var client = new NativeConversationClient(transport);
        var model = new WorkspaceConversationModel(client, transport);
        await model.OpenAsync("p", "s");
        Check(model.Agents.Single().Label == "QA Agent" && model.Models.Single().Id == "acp:agent-exact", "bound ACP identity is visible independently of provider models");
        var permission = model.Snapshot!.Acp!.Permissions.Single();
        await model.RespondAcpPermissionAsync(permission, "allow-label");
        Check(transport.Writes.Count == 0, "permission labels cannot substitute for option IDs");
        await model.RespondAcpPermissionAsync(permission with { FrameId = "elsewhere" }, "allow-exact");
        Check(transport.Writes.Count == 0, "another session's permission is rejected");
        await model.RespondAcpPermissionAsync(permission, "allow-exact");
        Check(transport.Writes.Single().Args["option_id"]!.GetValue<string>() == "allow-exact"
            && transport.Writes.Single().Args["session_id"]!.GetValue<string>() == "s", "exact native permission choice and owner reach the host");
        await model.RespondAcpPermissionAsync(permission, "allow-exact");
        Check(transport.Writes.Count == 1, "resolved permission cannot be submitted again");
        transport.PendingPermission = true; await model.RefreshAsync();
        await model.RespondAcpPermissionAsync(permission, null);
        Check(transport.Writes.Last().Args.ContainsKey("option_id") && transport.Writes.Last().Args["option_id"] == null, "cancel sends a null option, never a guessed deny ID");
        await model.AnswerAcpAsync("not-live", "wrong"); Check(transport.Writes.Count == 2, "stale questions cannot be answered");
        await model.AnswerAcpAsync("question-exact", "自由回答");
        Check(transport.Writes.Last().Command == "native_conversation_acp_answer"
            && transport.Writes.Last().Args["answer"]!.GetValue<string>() == "自由回答", "free text answers the waiting request instead of sending a new turn");
        transport.PendingPermission = true; transport.LoseReply = true; await model.RefreshAsync();
        await model.RespondAcpPermissionAsync(permission, "allow-exact");
        Check(!model.Busy && model.OperationError != null && transport.Writes.Count == 4, "ambiguous permission response is reported without replay");
        await model.RefreshAsync(); Check(transport.Writes.Count == 4, "polling only reads after an uncertain write");
        transport.LoseReply = false;
        Check(await model.CreateAsync("p", agentId: "agent-exact") == "created", "configured ACP can create its own new conversation");
        Check(transport.Writes.Last().Args["acp_agent_id"]!.GetValue<string>() == "agent-exact", "creation preserves configured profile identity");
        var malformed = transport.Snapshot("s") with { Acp = new([permission with { FrameId = "other" }], []) };
        var rejected = false;
        try { ConversationSnapshot.Decode(JsonSerializer.SerializeToNode(malformed, ConversationSnapshot.JsonOptions), "p", "s"); }
        catch (InvalidDataException) { rejected = true; }
        Check(rejected, "cross-session ACP interactions fail snapshot validation");
        await SettingsAsync();
        Console.WriteLine("Native ACP: creation, binding, permission IDs, cancellation, questions and uncertain writes passed.");
    }
    private static async Task SettingsAsync()
    {
        var transport = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(transport), transport);
        await model.OpenAsync("p", "s");
        Check(model.CanChangeAcpSettings && model.Snapshot!.AcpState!.ModeChoices.Length == 2, "bound modes decode with exact camelCase protocol keys");
        await model.SetAcpModeAsync("Agent");
        await model.SetAcpConfigAsync("depth", JsonValue.Create("Deep")!);
        await model.SetAcpConfigAsync("confirm", JsonValue.Create("false")!);
        await model.SetAcpConfigAsync("future", JsonValue.Create("x")!);
        Check(transport.Writes.Count == 0, "display labels, invalid boolean types and unsupported options are rejected");
        await model.SetAcpModeAsync("agent");
        Check(transport.Writes.Count == 1 && model.Snapshot!.AcpState!.CurrentMode == "agent", "mode change is confirmed by authoritative snapshot");
        await model.SetAcpConfigAsync("depth", JsonValue.Create("deep-exact")!);
        Check(transport.Writes.Last().Args["change"]?["value"]?.GetValue<string>() == "deep-exact", "grouped select value uses protocol identity");
        await model.SetAcpConfigAsync("confirm", JsonValue.Create(false)!);
        Check(transport.Writes.Last().Args["change"]?["value"]?.GetValue<bool>() == false, "boolean false is not lost or converted to a string");
        transport.Running = true; await model.RefreshAsync();
        await model.SetAcpModeAsync("read");
        Check(transport.Writes.Count == 3 && !model.CanChangeAcpSettings, "settings cannot change an active turn");
        transport.Running = false; transport.ReadOnly = true; await model.RefreshAsync();
        await model.SetAcpModeAsync("read");
        Check(transport.Writes.Count == 3, "read-only session settings are immutable");
        transport.ReadOnly = false; transport.LoseReply = true; await model.RefreshAsync();
        await model.SetAcpModeAsync("read");
        await model.RefreshAsync();
        Check(transport.Writes.Count == 4 && model.OperationError != null, "uncertain settings write is never replayed by refresh");
        var malformed = transport.Snapshot("s") with { AcpState = transport.Settings("foreign") };
        var rejected = false;
        try { ConversationSnapshot.Decode(JsonSerializer.SerializeToNode(malformed, ConversationSnapshot.JsonOptions), "p", "s"); }
        catch (InvalidDataException) { rejected = true; }
        Check(rejected, "ACP settings from another session are rejected");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args)> Writes = [];
        public bool PendingPermission = true, PendingQuestion = true, LoseReply;
        public bool Running, ReadOnly;
        public string Mode = "read";
        public NativeAcpSessionState Settings(string session) => new(session,
            JsonNode.Parse("""{"currentModeId":"read","availableModes":[{"id":"read","name":"Read only"},{"id":"agent","name":"Agent"}]}""")!.AsObject(),
            [JsonNode.Parse("""{"id":"depth","type":"select","name":"Depth","currentValue":"short","options":[{"value":"short","name":"Short"},{"name":"Advanced","options":[{"value":"deep-exact","name":"Deep"}]}]}""")!.AsObject(),
             JsonNode.Parse("""{"id":"confirm","type":"boolean","currentValue":true}""")!.AsObject()]);
        private ulong sequence;
        public ConversationSnapshot Snapshot(string session) => new(ConversationSnapshot.SchemaId, "host", ++sequence, "p", session,
            [], null, Running, false, ReadOnly, "acp:agent-exact", null, null, [], Acp: new(PendingPermission
                ? [new("permission-exact", session, "QA", "preview", [new("allow-exact", "allow-label", "allow_once")])] : [],
                PendingQuestion ? ["question-exact"] : []), AcpAgentId: "agent-exact", AcpState: CurrentSettings(session));
        private NativeAcpSessionState CurrentSettings(string session)
        { var state = Settings(session); state.Modes!["currentModeId"] = Mode; return state; }
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "native_conversation_snapshot") return Task.FromResult(JsonSerializer.SerializeToNode(Snapshot(args["session_id"]!.GetValue<string>()), ConversationSnapshot.JsonOptions));
            if (command == "list_models") return Task.FromResult<JsonNode?>(new JsonArray());
            if (command == "list_acp_agents") return Task.FromResult(JsonNode.Parse("""[{"id":"agent-exact","label":"QA Agent"}]"""));
            if (command is "native_conversation_acp_permission" or "native_conversation_acp_answer" or "native_conversation_create" or "native_conversation_acp_setting")
            {
                Check(projectId == "p", "writes include project scope");
                Writes.Add((command, (JsonObject)args.DeepClone()));
                if (command == "native_conversation_acp_permission") PendingPermission = false;
                if (command == "native_conversation_acp_answer") PendingQuestion = false;
                if (command == "native_conversation_acp_setting" && args["change"]?["kind"]?.GetValue<string>() == "mode")
                    Mode = args["change"]!["id"]!.GetValue<string>();
                if (LoseReply) throw new IOException("lost response");
                if (command == "native_conversation_create") return Task.FromResult<JsonNode?>(JsonValue.Create("created"));
            }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
