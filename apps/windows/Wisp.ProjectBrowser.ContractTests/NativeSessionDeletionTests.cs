using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSessionDeletionTests
{
    public static async Task RunAsync()
    {
        var fake = new Fake(); using var groups = new WorkspaceSessionGroups(fake, "p");
        groups.ToggleSelection(["a", "b"]); groups.ToggleSelection(["b", "c"]);
        Check(groups.Selected.SetEquals(["a", "b", "c"]), "mixed groups select remaining members");
        groups.ToggleSelection(["b", "c"]); Check(groups.Selected.SetEquals(["a"]), "all checked group clears its own members");
        using var model = new WorkspaceSessionDeletion(fake, "p", ["a", "child"]);
        var first = model.LoadAsync(); var latest = model.ChooseArtifactsAsync(true);
        fake.Reads[1].SetResult(Node(Review(true))); await latest;
        fake.Reads[0].SetResult(Node(Review(false))); await first;
        Check(model.IncludeArtifacts && model.CanDelete && model.Preview!.Artifacts.Length == 1, "late preview cannot restore stale file choices");
        var deleting = model.DeleteAsync(); Check(!await model.DeleteAsync(), "double delete suppressed");
        Check(fake.Writes.Count == 1 && fake.Writes[0].Args["artifact_fingerprint"]!.ToString() == "root-fp"
            && fake.Writes[0].Args["child_artifact_fingerprints"]!["child"]!.ToString() == "child-fp", "reviewed family fingerprints forwarded");
        fake.Writes[0].Reply.SetResult(JsonSerializer.SerializeToNode(new[] { "a", "child" }));
        Check(await deleting && model.Removed.SetEquals(["a", "child"]) && fake.Writes.Count == 1, "selected child is deleted only through its parent");
        Check(!await model.DeleteAsync() && fake.Writes.Count == 1, "completed deletion cannot be replayed");

        var partialFake = new Fake(); using var partial = new WorkspaceSessionDeletion(partialFake, "p", ["a", "b", "c"]);
        var load = partial.LoadAsync(); partialFake.Reads[0].SetResult(Node(new("p", ["a", "b", "c"], new(0, 0, 0, []), null, [], null))); await load;
        var batch = partial.DeleteAsync(); partialFake.Writes[0].Reply.SetResult(JsonSerializer.SerializeToNode(new[] { "a" }));
        await partialFake.SecondWrite.Task; partialFake.Writes[1].Reply.SetException(new IOException("Unknown acknowledgement"));
        Check(!await batch && partial.Removed.SetEquals(["a"]) && partial.MutationUnconfirmed, "partial acknowledgements preserved");
        Check(!await partial.DeleteAsync() && partialFake.Writes.Count == 2, "unknown write stops remaining roots and cannot be replayed");

        var invalidFake = new Fake(); using var invalid = new WorkspaceSessionDeletion(invalidFake, "p", ["a", "child"]);
        var crossed = invalid.LoadAsync(); invalidFake.Reads[0].SetResult(Node(Review(false) with { ProjectId = "other" })); await crossed;
        Check(!invalid.CanDelete && invalid.Error != null, "cross-project preview rejected");
        var exploration = invalid.ChooseChildrenAsync(false);
        invalidFake.Reads[1].SetResult(Node(new("p", ["a", "child"], new(0, 0, 1, []), false, [], null))); await exploration;
        Check(!invalid.CanDelete, "open exploration cannot be detached");
        var closed = invalid.LoadAsync(); invalid.Dispose(); invalidFake.Reads[2].SetResult(Node(Review(false))); await closed;
        Check(invalid.Preview == null && invalidFake.Projects.All(p => p == "p"), "closed preview ignores reply and requests keep explicit project");
    }
    private static SessionDeletePreview Review(bool files) => new("p", ["a", "child"], new(1, 2, 0, ["child"]), true,
        files ? [new("a", new("root-fp", ["artifact"], ["results/a.csv"], []), [new("child", new("child-fp", [], [], []))])] : [], null);
    private static JsonNode? Node(SessionDeletePreview value) => JsonSerializer.SerializeToNode(value, ConversationSnapshot.JsonOptions);
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public List<TaskCompletionSource<JsonNode?>> Reads { get; } = [];
        public List<(JsonObject Args, TaskCompletionSource<JsonNode?> Reply)> Writes { get; } = [];
        public List<string?> Projects { get; } = [];
        public TaskCompletionSource<bool> SecondWrite { get; } = new(TaskCreationOptions.RunContinuationsAsynchronously);
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Projects.Add(projectId); var completion = new TaskCompletionSource<JsonNode?>(TaskCreationOptions.RunContinuationsAsynchronously);
            if (command == "native_project_session_delete_preview") Reads.Add(completion);
            else if (command == "native_conversation_delete") { Writes.Add((args, completion)); if (Writes.Count == 2) SecondWrite.SetResult(true); }
            else throw new InvalidOperationException(command);
            return completion.Task;
        }
    }
}
