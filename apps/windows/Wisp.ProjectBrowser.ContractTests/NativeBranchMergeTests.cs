using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeBranchMergeTests
{
    public static async Task RunAsync(string fixtures)
    {
        var data = JsonNode.Parse(File.ReadAllText(Path.Combine(fixtures, "../../native-conversations/v1/branch-merge.json")))!;
        var preview = data["preview"]!.Deserialize<NativeBranchReply<NativeBranchPreview>>()!;
        var merged = data["merged"]!.Deserialize<NativeBranchReply<NativeBranchMergeResult>>()!;
        Check(preview.Result.BranchSessionId == merged.Result.BranchSessionId && preview.Result.Messages.Length == 2, "shared branch preview and merge payloads");
        var fake = new Fake(data); using var model = new WorkspaceBranchMerge(fake, "research-1", "branch-1");
        await model.LoadAsync();
        Check(model.Preview?.GuardHash == "reviewed-guard" && model.Draft == "Generated draft" && model.CanMerge && fake.Calls.Count == 2, "opening previews the delta then generates a reviewable draft");
        model.Draft = "  Keep my exact draft\n";
        Check(await model.GenerateAsync("Retain uncertainty") && fake.Calls.Last().Args["current_version"]!.ToString() == "  Keep my exact draft\n"
            && fake.Calls.Last().Args["user_guidance"]!.ToString() == "Retain uncertainty", "guided generation passes the current version without trimming content");
        var calls = fake.Calls.Count;
        Check(!await model.GenerateAsync(" ") && !await model.GenerateAsync(new string('x', 8001)) && fake.Calls.Count == calls, "blank/oversize guidance stays local");
        model.Draft = " "; Check(!model.CanMerge && !await model.MergeAsync(), "empty reviewed summary cannot merge");
        model.Draft = "  Reviewed exact text\n";
        Check(await model.MergeAsync() && model.Result?.MainSessionId == "main-1" && !model.CanMerge && !model.CanGenerate, "matching merge acknowledgement completes exactly once");
        Check(fake.Calls.Last().Args["summary"]!.ToString() == "  Reviewed exact text\n" && fake.Calls.Last().Args["expected_guard_hash"]!.ToString() == "reviewed-guard", "submit exact approved text against the reviewed guard");
        calls = fake.Calls.Count; await model.MergeAsync(); Check(fake.Calls.Count == calls, "confirmed merge cannot repeat");

        var failed = new Fake(data); using var lost = new WorkspaceBranchMerge(failed, "research-1", "branch-1"); await lost.LoadAsync();
        lost.Draft = "My edits"; failed.FailGenerate = true;
        Check(!await lost.GenerateAsync() && lost.Draft == "My edits" && lost.CanMerge, "generation failure retains the edited draft");
        failed.FailMerge = true; Check(!await lost.MergeAsync() && lost.MutationUnconfirmed && lost.Draft == "My edits" && !lost.CanMerge && !lost.CanGenerate, "unconfirmed merge retains content and forbids replay");
        calls = failed.Calls.Count; await lost.MergeAsync(); Check(failed.Calls.Count == calls, "lost merge acknowledgement does not repeat");
        var wrong = new Fake(data) { WrongPreview = true }; using var scoped = new WorkspaceBranchMerge(wrong, "research-1", "branch-1"); await scoped.LoadAsync();
        Check(scoped.Preview == null && wrong.Calls.Count == 1, "wrong-project preview never starts model generation");
        var wrongMerge = new Fake(data); using var mismatched = new WorkspaceBranchMerge(wrongMerge, "research-1", "branch-1"); await mismatched.LoadAsync(); wrongMerge.WrongMerge = true;
        Check(!await mismatched.MergeAsync() && mismatched.Result == null && mismatched.MutationUnconfirmed, "wrong-main merge reply cannot navigate to another conversation");
        var slow = new Fake(data) { PendingPreview = new() }; using var closed = new WorkspaceBranchMerge(slow, "research-1", "branch-1");
        var loading = closed.LoadAsync(); Check(closed.CanDismiss, "read-only preview loading can be dismissed immediately"); closed.Dispose(); slow.PendingPreview.SetResult(data["preview"]!.DeepClone()); await loading;
        Check(closed.Preview == null && slow.Calls.Count == 1, "closing invalidates late previews and prevents implicit generation");
        Check(fake.Calls.All(c => c.Project == "research-1" && c.Args["session_id"]!.ToString() == "branch-1"), "every request retains its explicit project and branch");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private sealed class Fake(JsonNode data) : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls = [];
        public bool FailGenerate, FailMerge, WrongPreview, WrongMerge; public TaskCompletionSource<JsonNode?>? PendingPreview;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, args.DeepClone().AsObject(), projectId));
            if (command.EndsWith("_preview"))
            {
                if (PendingPreview != null) return PendingPreview.Task;
                var value = data["preview"]!.DeepClone(); if (WrongPreview) value["project_id"] = "other"; return Task.FromResult<JsonNode?>(value);
            }
            if (command.EndsWith("_generate"))
            {
                if (FailGenerate) throw new IOException("Generation failed");
                return Task.FromResult<JsonNode?>(JsonSerializer.SerializeToNode(new NativeBranchReply<string>(projectId!, "Generated draft")));
            }
            if (FailMerge) throw new IOException("Acknowledgement lost");
            var result = data["merged"]!.DeepClone(); if (WrongMerge) result["result"]!["main_session_id"] = "another-main";
            return Task.FromResult<JsonNode?>(result);
        }
    }
}
