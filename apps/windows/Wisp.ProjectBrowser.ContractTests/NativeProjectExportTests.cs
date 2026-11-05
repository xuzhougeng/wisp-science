using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProjectExportTests
{
    private static void Check(bool ok, string message) { if (!ok) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var destination = Path.Combine(Path.GetTempPath(), "Study.zip");
        Task<string?> Pick() => Task.FromResult<string?>(destination);
        var fake = new Fake(); using var cancelled = new NativeProjectExportModel(fake, "project-1");
        await cancelled.ExportAsync(false, () => Task.FromResult<string?>(null));
        Check(cancelled.Stage == NativeProjectExportStage.Cancelled && fake.Calls == 0, "cancelled picker never exports");
        using var success = new NativeProjectExportModel(fake, "project-1");
        Check(await success.ExportAsync(false, Pick) && success.Stage == NativeProjectExportStage.Complete && fake.Scope == "project-1"
            && fake.Args?["destination_path"]?.GetValue<string>() == destination && fake.Args?["format"]?.GetValue<string>() == "zip", "successful export preserves explicit project, selected path and format");
        Check(!await success.ExportAsync(false, Pick) && fake.Calls == 1, "terminal export cannot replay");
        fake.Fail = true; using var failed = new NativeProjectExportModel(fake, "project-2");
        Check(!await failed.ExportAsync(true, Pick) && failed.Stage == NativeProjectExportStage.Failed && failed.Destination == destination && !failed.LocksProject, "uncertain export retains path and releases UI lock");
        Check(!await failed.ExportAsync(true, Pick) && fake.Calls == 2, "failure is not automatically replayed");
        fake.Fail = false; fake.WrongScope = true; using var wrong = new NativeProjectExportModel(fake, "project-2");
        Check(!await wrong.ExportAsync(false, Pick) && wrong.Stage == NativeProjectExportStage.Failed, "mismatched result cannot claim success");
        fake.WrongScope = false;
        var picker = new TaskCompletionSource<string?>(); var before = fake.Calls;
        using var closed = new NativeProjectExportModel(fake, "project-3"); var pending = closed.ExportAsync(false, () => picker.Task);
        Check(closed.LocksProject && !closed.IsModal && !await closed.ExportAsync(false, Pick), "selection owns one export and locks its project");
        closed.Dispose(); picker.SetResult(destination); await pending;
        Check(fake.Calls == before && !closed.LocksProject, "late picker result after closing cannot dispatch");
        fake.Pending = new(); using var late = new NativeProjectExportModel(fake, "project-4");
        var operation = late.ExportAsync(false, Pick); late.Dispose(); fake.Pending.SetResult(new JsonObject { ["project_id"] = "project-4", ["format"] = "zip", ["destination_path"] = destination }); await operation;
        Check(late.Stage != NativeProjectExportStage.Complete, "late completion never remounts a closed export");
        Check(NativeProjectExportModel.DirectoryName("../A:B\\C ") == "..-A-B-C" && NativeProjectExportModel.DirectoryName("...") == "wisp-project"
            && NativeProjectExportModel.DirectoryName("CON.txt") == "wisp-CON.txt", "project names cannot traverse folders or create Windows device paths");
        var parent = Path.GetTempPath();
        Check(NativeProjectExportModel.DirectoryDestination(parent, "Study", path => path == Path.Combine(parent, "Study")) == Path.Combine(parent, "Study-2"), "directory export never selects an existing project folder");
        Check(NativeProjectExportModel.ArchiveName("RNA seq 中文") == "wisp-project-RNA_seq_中文.zip", "archive suggestion preserves Unicode and shares WebView prefix");
        Check(NativeHomePresentation.Status("running") == "running" && NativeHomePresentation.Status("needs_you") == "needs_you"
            && NativeHomePresentation.Status(null) == "complete" && NativeHomePresentation.Status("idle") == "complete", "home badge follows shared three-state contract");
        Check(NativeHomePresentation.WindowTitle("Study 中文").Contains("Study 中文") && !NativeHomePresentation.WindowTitle(null).Contains("Study"), "separate project windows identify their own project and reset on Home");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public int Calls; public string? Scope; public JsonObject? Args; public bool Fail, WrongScope;
        public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(command == "native_project_export", "only native export is dispatched"); Calls++; Scope = projectId; Args = (JsonObject)arguments.DeepClone();
            if (Fail) throw new IOException("Lost response"); if (Pending != null) return Pending.Task;
            return Task.FromResult<JsonNode?>(new JsonObject { ["project_id"] = WrongScope ? "other" : projectId, ["format"] = arguments["format"]!.DeepClone(), ["destination_path"] = arguments["destination_path"]!.DeepClone() });
        }
    }
}
