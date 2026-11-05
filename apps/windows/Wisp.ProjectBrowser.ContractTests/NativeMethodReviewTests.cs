using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeMethodReviewTests
{
    private const string Run = "method-search-001", Version = "selected-version-001";

    public static async Task RunAsync(string fixture)
    {
        var payload = JsonNode.Parse(await File.ReadAllTextAsync(fixture))!;
        var host = new Host(payload);
        var client = new NativePanelClient(host);
        var panel = new WorkspacePanelModel(client, "default", null, new NativePanelTabs(available: ["hosts"]));
        var row = payload["run"]!.Deserialize<NativeRun>(ConversationSnapshot.JsonOptions)!;
        var review = panel.ContextMethodReview("local", row with { CanManage = false });
        review.Expanded = true;
        await review.ExecuteAsync("inspect", default);
        var detail = review.Detail!;
        Check(detail.Audit.SentinelReachable && detail.AuditVersion == "audit-version-001" && detail.Audit.Baseline.MedianPrimary == .5372
            && detail.Candidates[1].ParentCandidateId == detail.Candidates[0].Id && detail.Strategies.Single().Weight == 1.4,
            "mixed-case contract retains the audited baseline, lineage and strategy weight");
        await review.ExecuteAsync("start", default);
        Check(host.Calls.SequenceEqual(["inspect"]), "a read-only review cannot approve a draft");
        review.Observe(row with { CanManage = true });
        await review.ExecuteAsync("start", default); // Host applied it and lost the reply.
        await review.ExecuteAsync("start", default);
        Check(review.Unconfirmed && review.Stale && host.Calls.SequenceEqual(["inspect", "start"]), "never repeat an unconfirmed write");
        await review.ExecuteAsync("inspect", default);
        Check(!review.Unconfirmed && !review.Stale && review.Detail!.Run.Status == "running"
            && NativeMethodReview.Actions("running").SequenceEqual(["pause", "cancel"]), "manual refresh unlocks the actual next transition");
        await review.ExecuteAsync("start", default);
        Check(host.Calls.Count == 3, "no start action after approval");
        var reader = panel.MethodOutputPreview("local", review.Detail!).Fork();
        await reader.OpenAsync(Version);
        Check(reader.Content?.Text == "frozen-v1" && reader.Selected?.Path == "artifact-version:" + Version
            && !reader.CanUseLibrary && !reader.CanReadProvenance, "detached reader keeps the frozen version without consulting current files");
        await reader.ExportAsync(name => Task.FromResult<string?>(name == "selected_method.py" ? null : throw new Exception(name)),
            (_, _, _) => throw new Exception("picker cancelled"));
        Check(reader.ExportError == null && host.Calls.TakeLast(2).SequenceEqual(["read_output","export_output"]), "cancelled save picker only resolves the pinned version");
        host.WrongVersion = true;
        await reader.OpenAsync(Version);
        Check(reader.Content == null && reader.ContentError != null, "wrong version is rejected, never falls back to latest");
        var wrong = detail with { Candidates = [detail.Candidates[0] with { RunId = "other" }] };
        try { wrong.Verify(Run, "local"); throw new Exception("cross-run candidate accepted"); }
        catch (InvalidDataException) { }
        host.WrongSession = true;
        try { await client.MethodSearchAsync("default", null, "local", Run, "inspect", default); throw new Exception("session mixup accepted"); }
        catch (InvalidDataException) { }

        // A poll arriving during the first inspection should trigger exactly one read of the new status.
        var reply = new TaskCompletionSource<NativeMethodDetail>();
        var calls = 0;
        var racing = new NativeMethodReview((op, _) => op != "inspect" ? throw new Exception(op) :
            ++calls == 1 ? reply.Task : Task.FromResult(detail with { Run = row with { Status = "paused" } })) { Expanded = true };
        racing.Observe(row);
        var first = racing.ExecuteAsync("inspect", default);
        racing.Observe(row with { Status = "paused" });
        reply.SetResult(detail);
        await first;
        Check(!racing.Stale && racing.Detail?.Run.Status == "paused" && calls == 2, "superseded detail read reconciles without replaying a write");
    }

    private static void Check(bool value, string reason) { if (!value) throw new InvalidOperationException(reason); }

    private sealed class Host(JsonNode payload) : INativeSettingsClient
    {
        public readonly List<string> Calls = [];
        public bool WrongVersion, WrongSession;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(command == "native_context_run_action" && args["operation"]?["action"]?.ToString() == "method_search"
                && projectId == "default" && args["session_id"] == null && args["context_id"]?.ToString() == "local"
                && args["run_id"]?.ToString() == Run, "request keeps the project and exact environment run without creating a session");
            var operation = args["operation"]!["operation"]!;
            var kind = operation["kind"]!.ToString(); Calls.Add(kind);
            if (kind == "start") { payload["run"]!["status"] = "running"; throw new IOException("reply lost"); }
            JsonNode result;
            if (kind == "inspect") result = payload.DeepClone();
            else {
                Check(operation["version_id"]?.ToString() == Version, "explicit version id required");
                JsonNode content = kind switch {
                    "read_output" => new JsonObject { ["path"] = "snapshot", ["mime"] = "text/plain", ["text"] = "frozen-v1" },
                    "export_output" => new JsonObject { ["path"] = Path.GetFullPath("snapshot"), ["name"] = "selected_method.py", ["total_bytes"] = 9 },
                    _ => throw new Exception(kind)
                };
                result = new JsonObject { ["run_id"] = Run, ["version_id"] = WrongVersion ? "latest" : Version, ["data"] = content };
            }
            return Task.FromResult<JsonNode?>(new JsonObject { ["project_id"] = projectId, ["session_id"] = WrongSession ? "other" : null,
                ["context_id"] = "local", ["data"] = result });
        }
    }
}
