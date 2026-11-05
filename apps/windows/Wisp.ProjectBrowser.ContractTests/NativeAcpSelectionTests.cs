using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeAcpSelectionTests
{
    public static async Task RunAsync()
    {
        var host = new Fake(); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenProjectDraftAsync("p"); model.Draft = "first draft";
        Check(model.SelectProvisionalAgent("a") && model.ComposerModelId == "acp:a" && host.Creates == 0 && host.Sends == 0, "project draft selection performs no writes");
        Check(!model.SelectProvisionalAgent("missing") && model.ComposerAgentId == "a", "unknown identity cannot replace the selection");
        Check(model.SelectProvisionalAgent("b") && model.RestoreProvisionalHttpModel("http") && model.ComposerAgentId == null && host.Creates == 0, "empty draft can change ACP profiles and return to HTTP without creating a frame");
        model.SelectProvisionalAgent("a");
        await model.OpenProjectDraftAsync("q"); Check(model.ComposerAgentId == null, "choice is scoped to its project");
        await model.OpenProjectDraftAsync("p"); Check(model.ComposerAgentId == "a", "returning to the draft restores its choice");
        var created = await model.EnsureDraftSessionAsync();
        Check(created == "new-1" && host.Creates == 1 && host.CreateAgent == null && model.ComposerAgentId == "a" && model.Draft == "first draft", "explicit first action creates one ordinary frame and retains its provisional selection and draft");
        await model.RefreshAsync(); Check(model.ComposerAgentId == "a", "empty snapshots cannot erase an unsent selection");
        await model.SendAsync();
        Check(host.Sends == 1 && host.SendAgent == "a" && model.Draft == "" && model.ComposerAgentId == "a", "first send carries the selected profile and snapshot binding replaces its provisional choice");

        host = new(); model = new(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "empty"); model.Draft = "empty session text";
        Check(model.SelectProvisionalAgent("b") && host.Creates == 0, "an existing empty HTTP session switches in place");
        host.FailSend = true; await model.SendAsync(); await model.SendAsync();
        Check(host.Sends == 1 && model.UncertainSend && model.ComposerAgentId == "b" && model.Draft == "empty session text", "uncertain first ACP send retains draft/choice and is never replayed");

        host = new(); model = new(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "full"); model.Draft = "move me";
        Check(!model.SelectProvisionalAgent("a"), "a nonempty transcript cannot be rebound in place");
        var next = await model.CreateAcpDraftAsync("b");
        Check(model.AcpDraftRevision == 1, "successful draft transfer produces one new-session notice revision");
        Check(next == "new-1" && model.CurrentSessionId == next && model.ComposerAgentId == "b" && model.Draft == "move me" && host.Sends == 0 && host.CreateAgent == null, "new ACP conversation moves text and remains provisional without launching a process");
        await model.OpenAsync("p", "full"); Check(model.Draft == "" && model.VisibleItems.Length == 1, "source keeps messages but no duplicate draft");
        await model.OpenAsync("p", next!); Check(model.Draft == "move me" && model.ComposerAgentId == "b", "moved draft and choice survive navigation");

        await model.OpenAsync("p", "full"); model.Draft = "keep on failure"; host.FailCreate = true;
        Check(await model.CreateAcpDraftAsync("a") == null && model.CurrentSessionId == "full" && model.Draft == "keep on failure", "failed creation cannot discard or move the source draft");
        Check(model.AcpDraftRevision == 1, "failed creation cannot announce a new session");
        Check(model.CreationFailureDetail == "create reply unavailable", "creation failure exposes literal host detail for localized UI copy");
        host.FailCreate = false; var hold = host.CreateHold = new(); var pending = model.CreateAcpDraftAsync("a");
        await model.OpenAsync("q", "other"); model.Draft = "other project draft"; hold.SetResult(JsonValue.Create("late"));
        Check(await pending == null && model.CurrentProjectId == "q" && model.Draft == "other project draft", "late creation cannot navigate or copy another project's draft");
        Check(model.AcpDraftRevision == 1, "late creation cannot announce success on another cursor");
        await model.OpenAsync("p", "full"); Check(model.Draft == "keep on failure", "late creation leaves the original source draft intact");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public int Creates, Sends; public bool FailCreate, FailSend;
        public string? CreateAgent, SendAgent; public TaskCompletionSource<JsonNode?>? CreateHold;
        private readonly Dictionary<string, string> bindings = []; private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            var session = args["session_id"]?.GetValue<string>() ?? "";
            if (command == "native_conversation_snapshot")
            {
                var agent = bindings.GetValueOrDefault(session);
                return Task.FromResult(JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId, "memory", ++sequence, projectId!, session,
                    session == "full" || agent != null ? [new("user", "prior message", null, null, null, null)] : [], null, false, false, false,
                    agent == null ? "http" : "acp:" + agent, null, null, [], AcpAgentId: agent), ConversationSnapshot.JsonOptions));
            }
            if (command == "list_models") return Task.FromResult(JsonNode.Parse("""[{"id":"http","label":"HTTP","model":"chat","active":true}]"""));
            if (command == "list_acp_agents") return Task.FromResult(JsonNode.Parse("""[{"id":"a","label":"A"},{"id":"b","label":"B"}]"""));
            if (command == "native_conversation_create")
            {
                Creates++; CreateAgent = args["acp_agent_id"]?.GetValue<string>();
                if (FailCreate) throw new IOException("create reply unavailable");
                if (CreateHold is { } hold) { CreateHold = null; return hold.Task; }
                return Task.FromResult<JsonNode?>(JsonValue.Create("new-" + Creates));
            }
            if (command == "native_conversation_send")
            {
                Sends++; SendAgent = args["acp_agent_id"]?.GetValue<string>();
                if (FailSend) throw new IOException("send reply unavailable");
                if (SendAgent != null) bindings[session] = SendAgent;
            }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
