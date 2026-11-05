using System.Text.Json;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

static class NativeSessionRunGroupsTests
{
    public static void Run()
    {
        NativeRun Row(string id, string? frame, string status, long created, long? ended = null) =>
            JsonSerializer.Deserialize<NativeRun>(JsonSerializer.Serialize(new
            { id, frame_id = frame, context_id = "local", title = id, kind = "file_transfer", status,
                created_at = created, ended_at = ended, progress_json = "{}" }), ConversationSnapshot.JsonOptions)!;
        var rows = new[] { Row("a", "s", "running", 1), Row("z", "s", "submitted", 2), Row("b", "s", "paused", 2),
            Row("c", "s", "cancelling", 3), Row("other", "other", "running", 99), Row("legacy", null, "running", 100),
            Row("old", "s", "succeeded", 1, 4), Row("new", "s", "failed", 0, 5), Row("draft", "s", "draft", 6),
            Row("tie-a", "s", "cancelled", 2, 4), Row("tie-z", "s", "timed_out", 2, 4), Row("lost", "s", "lost", 1, 3) };
        var groups = NativeSessionRunGroups.For(rows, "s");
        Require(groups.Active.Select(run => run.Id).SequenceEqual(["c", "z", "b", "a"]), "active status and timestamp/id order");
        Require(groups.Completed.Select(run => run.Id).SequenceEqual(["draft", "new", "tie-z", "tie-a", "old", "lost"]), "completed end/creation/id order including drafts");
        Require(NativeSessionRunGroups.For(rows, null).Empty && NativeSessionRunGroups.For(rows, "missing").Empty, "no session never includes unscoped runs");
        Require(NativePaletteActions.Match("runs", true, true, true).Any(action => action.Id == "runs")
            && !NativePaletteActions.Match("runs", true, false, true).Any(action => action.Id == "runs"), "Runs palette action requires a conversation");
    }
    private static void Require(bool condition, string message)
    { if (!condition) throw new InvalidOperationException(message); }
}
