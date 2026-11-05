using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProjectSyncJoinTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var client = new Fake();
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>
        { ["en"] = new() { ["err.sync_folder"] = "Choose a folder first." }, ["zh"] = new() { ["err.sync_folder"] = "请先选择目录。" } }, "en");
        using var parent = new NativeSettingsEditorModel(client, "parent-project");
        parent.Edit(new() { ["sync_folder"] = "saved" }, "set_settings", "settings");
        parent.Draft!["sync_folder"] = "unsaved";
        using var join = new NativeProjectSyncJoinModel(client, "parent-project", locale);
        join.Open(); join.Dismiss();
        Check(!join.ModalOpen && parent.HasChanges && parent.Draft["sync_folder"]!.GetValue<string>() == "unsaved" && client.Calls.Count == 0,
            "immediate child dismissal retains the parent draft and does not invoke the host");
        join.Open(); join.SetCode(" \n ");
        Check(!join.CanJoin && !await join.JoinAsync() && client.Calls.Count == 0, "whitespace cannot submit");
        const string code = "  synthetic\ncode  "; join.SetCode(code); join.Dismiss(); join.Open();
        Check(join.Code == code && join.CanJoin, "dismiss and reopen retain the exact unsaved device code");
        await join.JoinAsync();
        Check(join.ModalOpen && join.Code == code && join.JoinedProjectId == null && join.DisplayError == null,
            "cancelled folder picker keeps the modal and its draft without reporting a joined project");
        Check(client.Calls.Single() is { Command: "join_synced_project", Project: "parent-project" } call && call.Args["code"]!.GetValue<string>() == code,
            "join uses the existing command and project scope, passing the original code without trimming");
        client.Handler = () => throw new IOException("Choose a shared sync folder in Settings first.");
        var calls = client.Calls.Count;
        Check(!await join.JoinAsync() && join.ModalOpen && join.Code == code && join.DisplayError == "Choose a folder first." && client.Calls.Count == calls + 1,
            "failure remains inline with its draft and is never retried automatically");
        locale.Preview("zh"); Check(join.DisplayError == "请先选择目录。", "known backend errors use the current shared locale");
        join.Dismiss(); join.Open(); Check(join.DisplayError == null && join.Code == code, "reopening clears the old error but retains the code");
        client.Handler = () => Task.FromResult<JsonNode?>(new JsonObject { ["id"] = "" });
        Check(!await join.JoinAsync() && join.ModalOpen && join.JoinedProjectId == null && join.Code == code, "malformed success cannot navigate or discard the draft");
        var pending = new TaskCompletionSource<JsonNode?>(); client.Handler = () => pending.Task;
        var request = join.JoinAsync(); calls = client.Calls.Count;
        Check(join.Busy && !join.CanJoin && !await join.JoinAsync() && client.Calls.Count == calls, "busy join cannot be submitted twice");
        join.Dismiss(); Check(!join.ModalOpen && join.Busy, "Escape hides an in-flight dialog without cancelling its owned request");
        pending.SetResult(new JsonObject { ["id"] = "joined-project" });
        Check(await request && join.JoinedProjectId == "joined-project" && join.Code == "" && !join.ModalOpen && parent.HasChanges,
            "only confirmed success publishes the returned project and clears the code, leaving parent settings untouched");
        var late = new TaskCompletionSource<JsonNode?>(); client.Handler = () => late.Task;
        var closing = new NativeProjectSyncJoinModel(client, null, locale); closing.Open(); closing.SetCode("late");
        var lateRequest = closing.JoinAsync(); closing.Dispose(); late.SetResult(new JsonObject { ["id"] = "late-project" });
        Check(!await lateRequest && closing.JoinedProjectId == null && closing.Code == "" && !closing.ModalOpen,
            "a disposed settings page cannot navigate from a late host reply or retain its device code");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<Task<JsonNode?>> Handler = () => Task.FromResult<JsonNode?>(null);
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId)); return Handler(); }
    }
}
