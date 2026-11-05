using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeReviewPresentationTests
{
    private static void Check(bool value, string message) { if (!value) throw new Exception(message); }
    private static ConversationItem Row(string role, string text, string? tool = null, string? kind = null) => new(role, text, tool, null, null, null, Kind: kind);
    private const string Report = """
        {"summary":"Check the claim","reviewer_model":" model ","reviewer_effort":" high ","findings":[
        {"message_index":2,"claim":"Claim","evidence":"Observed evidence","fix":"Verify source","verdict":"fail","severity":"high","status":"open"}]}
        """;
    public static async Task RunAsync()
    {
        var report = NativeReviewPresentation.Parse(Row("review", Report))!;
        Check(report.Model == "model · high" && report.EvidenceCoverage == 100 && report.CoverageGaps.Length == 0, "shared DTO defaults and trimmed reviewer metadata");
        Check(report.FooterKey == "review.agent_correcting" && report.Findings[0].StatusKey == "review.open", "open correction state");
        Check(report.CopyText == "Check the claim\n\n- Claim\n  Evidence: Observed evidence\n  Fix: Verify source", "copy exactly matches WebView's plain-text report");
        Check(NativeReviewPresentation.Parse(Row("review", "{}")) is { NoFindings: true, FooterKey: null }, "old reports use default fields");
        var insufficient = NativeReviewPresentation.Parse(Row("review", """{"review_status":"unreviewable","evidence_coverage":37,"coverage_gaps":["Missing tool output"]}"""))!;
        Check(!insufficient.NoFindings && insufficient.Unreviewable && insufficient.EvidenceCoverage == 37 && insufficient.CoverageGaps.Length == 1, "zero findings with incomplete evidence must not imply a pass");
        var resolved = NativeReviewPresentation.Parse(Row("review", Report.Replace("\"open\"", "\"resolved\"")))!;
        Check(resolved.FooterKey == "review.all_fixed" && resolved.Findings[0].Resolved, "resolved claim and footer");
        var unaddressed = NativeReviewPresentation.Parse(Row("review", Report.Replace("\"open\"", "\"unaddressed\"")))!;
        Check(unaddressed.FooterKey == "review.needs_attention" && !unaddressed.Findings[0].Resolved, "unaddressed remains visible");
        foreach (var invalid in new[] { "not json", "null", "[]", "{\"summary\":4}", "{\"summary\":null}", "{\"findings\":null}", "{\"findings\":[null]}", "{\"coverage_gaps\":[null]}", "{\"evidence_coverage\":256}", "{\"findings\":[{\"message_index\":-1}]}" })
            Check(NativeReviewPresentation.Parse(Row("review", invalid)) == null, "malformed report falls back without crashing: " + invalid);
        Check(NativeReviewPresentation.Parse(Row("assistant", Report)) == null, "ordinary prose is not a review card");
        var items = new[] { Row("user", "question"), Row("usage", "{}"), Row("review_transition", "reviewing"), Row("assistant", " "),
            Row("reasoning", "reason"), Row("tool", "result", "read_file"), Row("tool", "duplicate answer", "attempt_completion"),
            Row("assistant", "answer"), Row("review", Report), Row("system", "hidden"), Row("user", "checkpoint", kind: "checkpoint"),
            Row("acp_tool", ""), Row("plan", "{}"), Row("question", "{}"), Row("branch_merge", "result") };
        Check(new[] { 0, 4, 5, 7, 11, 12, 13, 14 }.Select((row, index) => NativeReviewPresentation.MessageItemIndex(items, (ulong)index) == row).All(v => v), "review message indices exclude UI metadata but retain reasoning and tools");
        Check(NativeReviewPresentation.MessageItemIndex(items, ulong.MaxValue) == null, "out-of-range never wraps");
        var page = new ConversationSnapshot(ConversationSnapshot.SchemaId, "e", 1, "p", "s", items, null, false, false, false, "", null, null, [], UserOffset: 0);
        Check(NativeReviewPresentation.JumpIndex(page, 8, 0) == 5, "finding resolves to original tool row");
        Check(NativeReviewPresentation.JumpIndex(page with { UserOffset = 3 }, 8, 0) == null && NativeReviewPresentation.JumpIndex(page with { NextBeforeSeq = 10 }, 8, 0) == null, "paged turn offsets cannot masquerade as message offsets");
        Check(NativeReviewPresentation.JumpIndex(page, 8, -1) == null && NativeReviewPresentation.JumpIndex(page, 8, 1) == null, "invalid finding selection rejected");
        var host = new Fake(page); var model = new WorkspaceConversationModel(new NativeConversationClient(host), host);
        await model.OpenAsync("p", "s"); var source = model.Snapshot!;
        Check(model.JumpToReviewMessage(source, 8, 0) && model.ScrollTarget == 5, "navigation changes only local reading target");
        await model.RefreshAsync();
        Check(model.JumpToReviewMessage(source, 8, 0), "poll sequence changes preserve valid targets");
        host.Page = page with { Items = items.Select((item, i) => i == 5 ? item with { Text = "replaced" } : item).ToArray() };
        await model.RefreshAsync(); var revision = model.ScrollRevision;
        Check(!model.JumpToReviewMessage(source, 8, 0) && model.ScrollRevision == revision, "old controls cannot jump after prefix replacement");
        host.Page = page; await model.OpenAsync("p", "other");
        Check(!model.JumpToReviewMessage(source, 8, 0), "foreign session button cannot move current transcript");
        await model.OpenAsync("p", "s"); source = model.Snapshot!; model.Pause();
        Check(!model.JumpToReviewMessage(source, 8, 0), "closed page does not move reading target");
    }
    private sealed class Fake(ConversationSnapshot initial) : INativeSettingsClient
    {
        public ConversationSnapshot Page = initial;
        private ulong sequence;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "native_conversation_snapshot") return Task.FromResult(JsonSerializer.SerializeToNode(
                Page with { SessionId = args["session_id"]!.GetValue<string>(), Sequence = ++sequence }, ConversationSnapshot.JsonOptions));
            if (command is "list_models" or "list_acp_agents") return Task.FromResult<JsonNode?>(new JsonArray());
            if (command.Contains("send") || command.Contains("history_action")) throw new Exception("Review navigation must not invoke a mutation");
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
