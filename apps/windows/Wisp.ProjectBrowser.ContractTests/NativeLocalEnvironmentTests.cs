using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeLocalEnvironmentTests
{
    public static async Task RunAsync()
    {
        var client = new Fake();
        using var model = new NativeLocalEnvironmentModel(client, "project-a");
        model.Edit();
        Check(!model.Editing && !await model.SaveAsync() && client.Calls.Count == 0,
            "a pending or failed first read cannot replace unknown interpreter paths with an empty draft");
        var bootstrap = Snapshot("C:\\Python\\python.exe");
        model.AcceptSnapshot(bootstrap); model.Edit();
        model.Draft!["python_executable"] = "D:\\研究\\python.exe";
        model.Draft["rscript_executable"] = "";
        model.AcceptSnapshot(Snapshot("unsolicited replacement"));
        Check(model.HasChanges && model.Draft["python_executable"]!.GetValue<string>() == "D:\\研究\\python.exe",
            "preference reloads cannot replace an independent path draft");
        Check(!await model.DetectAsync() && client.Calls.Count == 0, "detection never discards an open edit");
        model.Cancel();
        Check(!model.Editing && model.Paths!["python_executable"]!.GetValue<string>() == "C:\\Python\\python.exe"
            && client.Calls.Count == 0, "cancel restores the read snapshot without a backend write");
        model.Edit(); model.Draft!["python_executable"] = "";
        client.Result = null;
        Check(!await model.SaveAsync() && model.Editing && model.HasChanges && client.Calls.Count == 1,
            "unconfirmed/malformed saves retain path edits without automatic replay");
        var args = client.Calls[0].Args;
        Check(args["paths"]!["python_executable"]!.GetValue<string>() == ""
            && args["paths"]!["future_executable"]!.GetValue<string>() == "keep", "blank paths request detection and unknown paths are preserved");
        client.Result = Snapshot("auto-detected");
        Check(await model.SaveAsync() && !model.Editing && model.Saved && model.Paths!["python_executable"]!.GetValue<string>() == "auto-detected",
            "successful saves use the host's returned detection result");
        Check(client.Calls.All(c => c.Command == "save_local_environment_paths" && c.Project == "project-a"),
            "path edits cannot invoke set_settings or change another preference scope");
        client.Result = Snapshot("refreshed");
        Check(await model.DetectAsync() && client.Calls[^1].Command == "detect_local_environment"
            && client.Calls[^1].Args.Count == 0 && !model.Saved, "explicit refresh uses its independent command");
        var detection = new TaskCompletionSource<JsonNode?>(); client.Pending = detection.Task;
        var checking = model.DetectAsync(); model.Edit();
        Check(model.Busy && !model.Saving && !model.Editing && !await model.DetectAsync(),
            "detection locks only the path editor and refuses duplicate checks");
        detection.SetResult(Snapshot("detected")); Check(await checking && model.CanEdit, "the card unlocks on reply");
        var pending = new TaskCompletionSource<JsonNode?>(); client.Pending = pending.Task;
        model.Edit(); var save = model.SaveAsync();
        Check(model.Saving && !await model.SaveAsync(), "pending path writes guard leave/submit separately from a detection");
        model.Dispose(); pending.SetResult(Snapshot("late"));
        Check(!await save && model.Snapshot == null && model.Draft == null, "closing rejects late path responses");
        Check(bootstrap["local_environment"]!["paths"]!["python_executable"]!.GetValue<string>() == "C:\\Python\\python.exe",
            "editing never mutates the shared bootstrap snapshot");
    }
    private static JsonObject Snapshot(string python) => new() { ["local_environment"] = new JsonObject {
        ["paths"] = new JsonObject { ["python_executable"] = python, ["future_executable"] = "keep" } } };
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public JsonNode? Result;
        public Task<JsonNode?>? Pending;
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            return Pending ?? Task.FromResult(Result?.DeepClone());
        }
    }
}
