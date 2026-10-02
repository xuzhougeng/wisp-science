using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class WorkspaceJourneyTests
{
    public static async Task RunAsync()
    {
        var fake = new Fake();
        using var model = new WorkspaceJourneyModel(fake, "project");
        fake.Page = new([new("late", "产物", 86401, "artifact", "样本", SourceId: "v1"), new("early", "发现", 86399, "finding")], true);
        await model.LoadAsync(0, 172800);
        Check(model.Days(TimeZoneInfo.Utc).Length == 2 && model.Days(TimeZoneInfo.Utc)[0].Entries[0].Id == "late", "grouped by explicit local date, newest first");
        var zone = TimeZoneInfo.CreateCustomTimeZone("UTC+8 test", TimeSpan.FromHours(8), "UTC+8", "UTC+8");
        Check(model.Days(zone).Length == 1, "day grouping uses local timezone rather than UTC boundaries");
        model.Query = "样本"; Check(model.Days().SelectMany(day => day.Entries).Single().Id == "late", "search covers summaries");
        model.Kind = "finding"; Check(model.Days().Length == 0, "type filter combines with search");
        model.Kind = ""; model.Query = "";
        fake.Fail = true; Check(!await model.LoadAsync(0, 172800) && model.Page?.Truncated == true, "failed refresh retains previous evidence and truncation state"); fake.Fail = false;
        var entry = fake.Page.Entries[0];
        var pending = fake.Pending = new(); var selecting = model.SelectAsync(entry);
        Check(model.SourceLoading && model.Selected == entry, "source has an independent loading state");
        model.Back(); pending.SetResult(Artifact("v1")); await selecting;
        Check(model.Selected == null && model.Artifact == null, "late source read cannot reopen returned detail");
        fake.Pending = null; await model.SelectAsync(entry);
        Check(model.Artifact?.VersionId == "v1" && fake.Project == "project", "exact immutable version read under explicit project");
        await model.OpenArtifactAsync("input-v0"); Check(model.HasParentSource && model.Artifact?.VersionId == "input-v0", "source chain follows input version");
        model.Back(); Check(model.Artifact?.VersionId == "v1" && model.Selected == entry, "source back restores parent without re-reading list");
        fake.Fail = true; await model.OpenArtifactAsync("broken");
        Check(model.SourceError != null && model.Artifact?.VersionId == "v1", "partial source failure preserves last readable detail");
        fake.Fail = false; var calls = fake.ArtifactCalls;
        model.Query = "preserved";
        await model.OpenRunAsync();
        Check(model.Run?.Id == "r" && fake.Project == "project", "artifact run opens by exact id in its project");
        model.Back();
        Check(model.Run == null && model.Artifact?.VersionId == "v1" && model.Selected == entry && model.Query == "preserved", "run back preserves parent artifact and filters");
        var runPending = fake.RunPending = new(); var readingRun = model.OpenRunAsync();
        model.Back(); runPending.SetResult(Run("r")); await readingRun;
        Check(model.Run == null && model.Selected == null, "late run read cannot reopen returned journey");
        fake.RunPending = null;
        await model.SelectAsync(entry with { Kind = "run", RunId = "r" });
        await model.OpenRunAsync(); Check(model.Run?.Id == "r", "run entry opens without an artifact or active session");
        model.Back(); fake.RunPending = new(); readingRun = model.OpenRunAsync();
        fake.RunPending.SetResult(Run("wrong")); await readingRun;
        Check(model.Run == null && model.SourceError != null, "mismatched run response rejected");
        fake.RunPending = null;
        await model.SelectAsync(entry with { SourceDiscarded = true }); Check(fake.ArtifactCalls == calls, "discarded source never starts a file read");
        pending = fake.Pending = new(); selecting = model.SelectAsync(entry); model.Dispose(); pending.SetResult(Artifact("v1")); await selecting;
        Check(model.Artifact == null && !model.SourceLoading, "closed view rejects late source result");
        Console.WriteLine("Journey date grouping, filters, partial failures, version navigation and return isolation passed.");
    }
    private static JourneyArtifact Artifact(string id) => new(id, "result.txt", 1, new("r", "Run", "succeeded", "local", null, []), "evidence", "text/plain", null, false, null);
    private static NativeRun Run(string id) => System.Text.Json.JsonSerializer.Deserialize<NativeRun>("{\"id\":\"" + id + "\",\"context_id\":\"local\",\"title\":\"Run\",\"status\":\"succeeded\"}", ConversationSnapshot.JsonOptions)!;
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeJourneyClient
    {
        public JourneyPage Page = new([], false);
        public bool Fail;
        public string? Project;
        public int ArtifactCalls;
        public TaskCompletionSource<JourneyArtifact>? Pending;
        public TaskCompletionSource<NativeRun>? RunPending;
        public Task<NativeRun> RunAsync(string project, string runId, CancellationToken token = default)
        { Project = project; return RunPending?.Task ?? Task.FromResult(Run(runId)); }
        public Task<JourneyPage> ReadAsync(string project, long from, long until, CancellationToken token = default) => Fail ? Task.FromException<JourneyPage>(new IOException("read failed")) : Task.FromResult(Page);
        public Task<JourneyArtifact> ArtifactAsync(string project, string version, CancellationToken token = default)
        { Project = project; ArtifactCalls++; return Fail ? Task.FromException<JourneyArtifact>(new IOException("missing source")) : Pending?.Task ?? Task.FromResult(Artifact(version)); }
    }
}
