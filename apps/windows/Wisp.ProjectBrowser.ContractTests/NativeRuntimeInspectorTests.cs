using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeRuntimeInspectorTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var context = new NativePanelContext("local", "local", "Local", "{}", "{}", null, null);
        var worker = new NativeRuntimeInfo("worker", 2, new("p", "local", "python", "mainline", ""), "ready", null, null, null, 0, 0, null, null);
        var python = new NativeRuntimeSlot(context, "python", "ready", worker); var r = new NativeRuntimeSlot(context, "r", "missing", null);
        var fake = new Fake(); var inspector = new NativeRuntimeInspectorModel(fake); inspector.Bind("p", null); inspector.Observe([python, r]);
        await inspector.OpenAsync("local", "r"); Check(fake.Calls == 0 && inspector.EmptyKey == "runtime.environment_unavailable" && !inspector.CanRefresh, "missing runtime opens without starting or inspecting it");
        await inspector.OpenAsync("local", "python"); Check(fake.Calls == 1 && fake.Args?["runtime_generation"]?.GetValue<ulong>() == 2 && fake.Project == "p" && fake.Args?["session_id"] == null, "inspection is scoped to the visible worker generation without session creation");
        Check(inspector.Rows.Length == 3 && inspector.Snapshot!.TotalCount == 12, "truncated snapshot keeps server total");
        inspector.SetFilter(" FRAME "); Check(inspector.Rows.Length == 1, "filter searches types ignoring case and whitespace");
        inspector.SetFilter("100 rows"); Check(inspector.Rows.Length == 1, "filter also searches summaries");
        inspector.SetFilter("none"); Check(inspector.Rows.Length == 0 && inspector.EmptyKey == "runtime.objects_none_match", "no match is distinct from empty runtime");
        inspector.Close(); await inspector.OpenAsync("local", "python"); Check(inspector.Filter == "" && inspector.Rows.Length == 3, "reopening resets filter and refreshes");
        inspector.Observe([python with { Info = worker with { Status = "dead" }, Status = "dead" }, r]);
        Check(inspector.Rows.Length == 3 && !inspector.CanRefresh, "last-known snapshot survives a stopped worker with refresh disabled");
        inspector.Pinned = true; inspector.Close();
        Check(!inspector.Pinned && inspector.Selected == null && inspector.Filter == "", "navigation closes pinned selection and filter together");
        await inspector.OpenAsync("local", "python");
        Check(inspector.Rows.Length == 3 && !inspector.Pinned && !inspector.CanRefresh, "reopening a stopped environment retains its last snapshot without restoring the floating surface");
        inspector.Observe([python, r]); fake.Pending = new(); var read = inspector.InspectAsync();
        var calls = fake.Calls; await inspector.InspectAsync(); Check(fake.Calls == calls && inspector.Loading, "duplicate refresh is suppressed");
        inspector.Observe([python with { Info = worker with { Generation = 3 } }, r]);
        fake.Pending.SetResult(JsonSerializer.SerializeToNode(new NativeRuntimeObjects([], 0), ConversationSnapshot.JsonOptions)); await read;
        Check(inspector.Rows.Length == 3 && !inspector.Loading, "an old generation cannot replace the last-known object snapshot");
        fake.Pending = null; fake.Fail = true; await inspector.InspectAsync(); Check(inspector.Error != null && inspector.Snapshot != null, "error retains the previous snapshot but surfaces failure");
        fake.Fail = false; fake.Pending = new(); read = inspector.InspectAsync(); inspector.Bind("other", "s");
        fake.Pending.SetResult(fake.Reply()); await read; Check(inspector.Selected == null && inspector.Snapshot == null, "late replies cannot cross project/frame bindings");
        Check(NativeRuntimeInspectorModel.Quote("Python", fake.Objects[0]) == "[Python runtime] frame: DataFrame = 100 rows x 8 columns (2.0 KB)", "runtime quotation matches the reference without invented source turns");
        Check(NativeRuntimeInspectorModel.ClampPosition(1800, 900, 900, 700) == (264, 124)
            && NativeRuntimeInspectorModel.ClampPosition(-4, -8, 400, 300) == (16, 16), "pinned panel stays inside shrunken viewport");
        var conversation = new WorkspaceConversationModel(new NativeConversationClient(fake), fake); await conversation.OpenProjectDraftAsync("p");
        Check(conversation.AddRuntimeQuote("[Python runtime] x: int = 42") && conversation.CanSubmit && conversation.CurrentSessionId == null, "adding a variable to an empty project stays an unsaved quote draft");
        await conversation.OpenProjectDraftAsync("other"); Check(conversation.Quotes.Length == 0, "project drafts do not leak quoted variables");
        await conversation.OpenProjectDraftAsync("p"); Check(conversation.Quotes.Length == 1 && !conversation.Quotes[0].Message.Contains("第 0"), "returning restores quote without fabricated conversation provenance");
        conversation.RemoveQuote(conversation.Quotes[0]); Check(conversation.Quotes.Length == 0 && !conversation.CanSubmit, "draft quotation can be removed without creating a conversation");
        conversation.Pause();
    }
    private sealed class Fake : INativeSettingsClient
    {
        public int Calls; public string? Project; public JsonObject? Args; public bool Fail; public TaskCompletionSource<JsonNode?>? Pending;
        public NativeRuntimeObject[] Objects = [new("frame", "DataFrame", "100 rows x 8 columns", 2048), new("count", "int", "42", 28), new("bad", "unavailable", "Synthetic inaccessible object", null)];
        public JsonNode? Reply() => JsonSerializer.SerializeToNode(new NativeRuntimeObjects(Objects, 12), ConversationSnapshot.JsonOptions);
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command is "list_models" or "list_acp_agents") return Task.FromResult<JsonNode?>(new JsonArray());
            Check(command == "native_runtimes_inspect", "inspection must never execute code, start a worker, or create a session");
            Calls++; Project = projectId; Args = (JsonObject)args.DeepClone();
            return Fail ? Task.FromException<JsonNode?>(new IOException("Synthetic inspect failure")) : Pending?.Task ?? Task.FromResult(Reply());
        }
    }
}
