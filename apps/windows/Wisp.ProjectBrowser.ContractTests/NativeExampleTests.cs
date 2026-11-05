using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeExampleTests
{
    private static void Check(bool condition, string text) { if (!condition) throw new InvalidOperationException(text); }
    public static async Task RunAsync()
    {
        var entries = NativeExampleCatalog.Entries;
        Check(entries.Count == 6 && entries[0].Id == "manifest_esr1_01_datasets" && entries[^1].Id == "manifest_memory_01_long_context", "bundle retains the WebView narrative order and six recordings");
        foreach (var entry in entries)
        {
            var page = NativeExampleCatalog.Load(entry.Id);
            Check(page.ReadOnly && !page.Running && page.Approvals.Length == 0 && page.Items.Any(row => row.Role == "user") && page.Items.Any(row => row.Role == "assistant"), "every bundled recording opens as a complete inert transcript");
            Check(page.Items.All(row => !row.Text.Contains("{{artifact:")), "same artifact placeholders are cleaned as in WebView");
        }
        var json = JsonNode.Parse("""{"input_data":{"request":"request"},"output_data":{"context_seed":[{"role":"user","text":"seed","repeat":2,"pad":2}],"items":[{"role":"tool","text":"![figure]({{artifact:a}})","input":"{{artifact:b}}","tool_name":"read","ok":false,"duration_ms":15}]}}""")!;
        var expanded = NativeExampleCatalog.Decode("recording", json);
        Check(expanded.Items.Length == 3 && expanded.Items[0].Text == "seedseed" && expanded.Items[2].Text == "[figure (figure)]" && expanded.Items[2].Input == "(artifact)" && expanded.Items[2].DurationMs == 15 && expanded.Items[2].Ok == false, "seed expansion and tool metadata survive projection");
        var legacy = NativeExampleCatalog.Decode("legacy", JsonNode.Parse("""{"input_data":{"request":"a"},"output_data":{"thinking":"b","response":"c"}}""")!);
        Check(legacy.Items.Select(row => row.Role).SequenceEqual(new[] { "user", "reasoning", "assistant" }), "legacy recordings retain request, thinking and answer");
        var viewer = WorkspaceConversationModel.CreateExampleViewer(); viewer.ShowExample(expanded); viewer.Draft = "cannot send";
        Check(!viewer.CanSend && !viewer.CanAttach && viewer.VisibleItems.Length == 3, "recordings cannot send or attach");
        await viewer.RefreshAsync(); await viewer.SendAsync();
        Check(viewer.ConnectionError == null && viewer.VisibleItems.Length == 3, "read-only projection never polls the transport or sends the draft");
        viewer.Pause();
        var project = new ProjectSummary("target", "Target", "", "C:/target", false, 0, 0, 0, 0, 0, false, null);
        var fake = new Fake(); using var copy = new NativeExampleCopy(fake, entries[0], [project]);
        Check(!await copy.CopyAsync("wrong") && fake.Calls == 0, "only a listed destination can receive an example");
        var pending = copy.CopyAsync("target"); Check(!await copy.CopyAsync("target") && fake.Calls == 1, "pending copy cannot be duplicated");
        Check(fake.Command == "copy_demo_to_project" && fake.Args?["id"]?.GetValue<string>() == entries[0].Id && fake.Args?["targetProjectId"]?.GetValue<string>() == "target" && fake.Project == null, "copy reuses existing command and explicit destination without activating a project");
        fake.Pending.SetResult(JsonValue.Create("copied-session")); Check(await pending && copy.Result == "copied-session", "accepted copy returns the destination conversation");
        Check(!await copy.CopyAsync("target") && fake.Calls == 1, "accepted copy cannot repeat");
        var failed = new Fake(); using var uncertain = new NativeExampleCopy(failed, entries[0], [project]); var attempt = uncertain.CopyAsync("target");
        failed.Pending.SetException(new IOException("response lost")); Check(!await attempt && uncertain.Unconfirmed && !await uncertain.CopyAsync("target") && failed.Calls == 1, "lost acknowledgement never replays a mutation");
        var delayed = new Fake(); var closed = new NativeExampleCopy(delayed, entries[0], [project]); var late = closed.CopyAsync("target"); closed.Dispose(); delayed.Pending.SetResult(JsonValue.Create("late"));
        Check(!await late && closed.Result == null, "late acknowledgements cannot navigate a closed example page");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public int Calls; public string? Command, Project; public JsonObject? Args;
        public TaskCompletionSource<JsonNode?> Pending = new();
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls++; Command = command; Args = (JsonObject)args.DeepClone(); Project = projectId; return Pending.Task; }
    }
}
