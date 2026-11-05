using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeContextDetailsTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private static ConversationSnapshot Snapshot(string session = "s", string epoch = "epoch", string usage = "10") => new(
        ConversationSnapshot.SchemaId, epoch, 1, "p", session,
        [new("usage", "{\"ctx_tokens\":" + usage + "}", null, null, null, null)], null, false, false, false, "m", null, null, []);
    private static JsonNode Response(string session = "s") => new JsonObject { ["project_id"] = "p", ["session_id"] = session,
        ["details"] = JsonNode.Parse("""{"system_prompt":"Exact instructions\n第二行","tool_definitions":[{"name":"read","description":"Read a file"},{"name":"shell","description":"Run a command"}],"rules":"","skills":"Use documented methods","mcp_dynamic_tools":[],"subagent_definitions":[]}""") };
    public static async Task RunAsync()
    {
        var host = new Fake(); var model = new NativeContextDetailsModel(host); model.Bind(Snapshot());
        Check(host.Reads == 0, "binding does not fetch hidden context content");
        await model.LoadAsync();
        Check(model.Loaded && model.Sections["system_prompt"] == "Exact instructions\n第二行"
            && model.Sections["tool_definitions"] == "read\nRead a file\n\nshell\nRun a command" && model.Sections["rules"] == "", "exact shared section text and blank sections");
        await model.LoadAsync(); Check(host.Reads == 1, "expanding another category reuses one scoped read");
        model.Bind(Snapshot() with { Sequence = 2 }); await model.LoadAsync(); Check(host.Reads == 1, "poll sequence alone never refetches the details");
        model.Bind(Snapshot(usage: "20")); Check(!model.Loaded, "new context usage invalidates prior details");
        host.Fail = true; await model.LoadAsync(); await model.LoadAsync();
        Check(model.Error != null && host.Reads == 2 && model.Sections.Count == 0, "failed read is visible and is not automatically replayed");
        host.Fail = false; await model.RetryAsync(); Check(model.Loaded && host.Reads == 3, "explicit retry recovers the details");
        model.Invalidate(); host.Pending = new(); var pending = model.LoadAsync(); model.Bind(Snapshot("other"));
        host.Pending.SetResult(Response()); await pending; host.Pending = null;
        Check(!model.Loaded && !model.Busy && model.Sections.Count == 0, "late reply cannot populate another conversation");
        await model.LoadAsync(); Check(model.Error != null && !model.Loaded, "response scope is validated independently of request scope");
        model.Bind(Snapshot()); host.Pending = new(); pending = model.LoadAsync(); model.Invalidate();
        host.Pending.SetResult(Response()); await pending; host.Pending = null;
        Check(!model.Loaded && model.Sections.Count == 0, "closing a panel invalidates an in-flight reply");
        var reads = host.Reads; model.Bind(Snapshot() with { AcpAgentId = "agent" }); await model.LoadAsync();
        Check(!model.Available && host.Reads == reads, "ACP does not load built-in model context");
        model.Bind(Snapshot(), available: false); await model.LoadAsync(); Check(host.Reads == reads, "history and unavailable page cannot load head details");
        host.WithState = true; model.Bind(Snapshot()); await model.LoadAsync();
        Check(model.Context?.State.HeadEpoch == 2 && model.Context.Items[0].Role == "checkpoint"
            && model.Context.Items.Count(row => row.Role == "user") == 1, "Usage epoch summary receives scoped checkpoint metadata and counts retained turns only.");
        model.Invalidate(); Check(model.Context == null, "Invalidating details also removes the prior epoch summary.");
        foreach (var malformed in new[] { "{}", "{\"project_id\":\"p\",\"session_id\":\"s\",\"details\":{\"system_prompt\":3}}",
            "{\"project_id\":\"p\",\"session_id\":\"s\",\"details\":{\"tool_definitions\":[null]}}" })
        {
            var rejected = false; try { NativeContextDetailsModel.Decode(JsonNode.Parse(malformed), "p", "s"); } catch (InvalidDataException) { rejected = true; }
            Check(rejected, "malformed display payload is rejected");
        }
    }
    private sealed class Fake : INativeSettingsClient
    {
        public int Reads; public bool Fail, WithState; public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(command == "native_conversation_context" && projectId == "p" && args["session_id"] != null && args.Count == 1,
                "only the read-only context route with explicit project/frame scope is allowed");
            Reads++; if (Fail) throw new IOException("Synthetic detail read failure");
            var response = Response();
            if (WithState)
            {
                response["items"] = JsonNode.Parse("""[{"role":"user","kind":"checkpoint","text":"summary"},{"role":"user","text":"retained"}]""");
                response["state"] = JsonNode.Parse("""{"head_epoch":2,"in_context_from_user_index":1,"context_epochs":[],"compactions":[],"undone_epochs":[]}""");
            }
            return Pending?.Task ?? Task.FromResult<JsonNode?>(response);
        }
    }
}
