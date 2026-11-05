using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeReviewerTests
{
    private static void Check(bool condition, string message)
    { if (!condition) throw new InvalidOperationException(message); }

    public static async Task RunAsync()
    {
        var fake = new Fake();
        var source = new JsonObject { ["id"] = "reviewer", ["name"] = "Reviewer", ["builtin"] = true,
            ["instructions"] = "Protected prompt", ["model_id"] = "saved", ["future"] = 7 };
        using var editor = new NativeSettingsEditorModel(fake, "project-a", "session-a");
        editor.Edit(source, "save_specialist_cmd", "spec");
        editor.Draft!["review_backend"] = new JsonObject { ["kind"] = "follow_session" };
        editor.Draft["instructions"] = "stale edit";
        var request = editor.TestReviewerAsync();
        Check(editor.Busy && editor.ReviewerTesting && !editor.CanTestReviewer, "reviewer probe is pending and excludes duplicates");
        Check(!await editor.TestReviewerAsync() && fake.Calls == 1, "a second click cannot issue another real call");
        Check(fake.Command == "test_reviewer_backend" && fake.Project == "project-a"
            && fake.Args!["sessionId"]!.GetValue<string>() == "session-a"
            && fake.Args["reviewer"]?["review_backend"]?["kind"]?.GetValue<string>() == "follow_session"
            && fake.Args["reviewer"]?["instructions"]?.GetValue<string>() == "Protected prompt"
            && fake.Args["reviewer"]?["future"]?.GetValue<int>() == 7, "probe binds the unsaved draft to the exact project and session while protecting built-in instructions");
        editor.Draft["model_id"] = "later edit";
        Check(fake.Args!["reviewer"]?["model_id"]?.GetValue<string>() == "saved", "submitted reviewer is a snapshot");
        fake.Pending.SetResult(Report());
        Check(await request && editor.ReviewerReport != null && !editor.ReviewerTesting && !editor.SaveConfirmed
            && editor.Draft != null && editor.HasChanges && source["review_backend"] == null, "probe success retains the draft and never saves or changes the source");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>
        { ["en"] = new() { ["specialists.reviewer.test_ok"] = "{backend} / {model} ({status})." } });
        Check(NativeSpecialists.ReviewerResult(editor.ReviewerReport!, locale) == "ACP / {status} (needs_correction). Check evidence.",
            "successful transport preserves report status and model text instead of claiming a clean review");
        editor.Discard();
        Check(editor.ReviewerReport == null && !editor.ReviewerTestAttempted && !editor.CanTestReviewer && fake.Calls == 1,
            "Cancel clears report and draft without writes");

        using var noProject = new NativeSettingsEditorModel(fake, null, "session-a");
        noProject.Edit(source, "save_specialist_cmd", "spec");
        Check(!await noProject.TestReviewerAsync() && fake.Calls == 1, "a projectless editor cannot probe a conversation backend");
        using var noSession = new NativeSettingsEditorModel(fake, "project-b");
        noSession.Edit(source, "save_specialist_cmd", "spec");
        fake.Pending = new(); var withoutSession = noSession.TestReviewerAsync();
        Check(fake.Project == "project-b" && fake.Args!["sessionId"] == null, "a project without a selected session cannot inherit another editor's session");
        fake.Pending.SetException(new IOException("lost response"));
        Check(!await withoutSession && noSession.ReviewerReport == null && noSession.Error != null
            && noSession.Draft != null && !noSession.SaveConfirmed && fake.Calls == 2, "ambiguous failure retains draft and does not retry");
        fake.Pending = new(); var malformed = noSession.TestReviewerAsync();
        fake.Pending.SetResult(new JsonObject { ["ok"] = true });
        Check(!await malformed && noSession.ReviewerReport == null && noSession.Error != null, "an acknowledgement without a report cannot appear successful");
        fake.Pending = new(); var late = noSession.TestReviewerAsync(); noSession.Dispose(); fake.Pending.SetResult(Report());
        Check(!await late && noSession.ReviewerReport == null && noSession.Draft == null, "late replies cannot repopulate a closed editor");
        using var reader = new NativeSettingsEditorModel(fake, "project-a");
        reader.Edit(new() { ["id"] = "reader" }, "save_specialist_cmd", "spec");
        var calls = fake.Calls;
        Check(!await reader.TestReviewerAsync() && calls == fake.Calls, "only built-in Reviewer has the probe action");
    }

    private static JsonObject Report() => new() { ["backend"] = "acp_agent", ["model"] = "{status}", ["status"] = "needs_correction", ["summary"] = "  Check evidence.  " };
    private sealed class Fake : INativeSettingsClient
    {
        public int Calls;
        public string? Command, Project;
        public JsonObject? Args;
        public TaskCompletionSource<JsonNode?> Pending = new();
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls++; Command = command; Project = projectId; Args = (JsonObject)arguments.DeepClone(); return Pending.Task; }
    }
}
