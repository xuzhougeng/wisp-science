using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

// Presentation adapter for the existing wisp_dto::ReviewReport JSON in a review
// transcript row. This is not a new wire protocol or a second report producer.
public sealed record NativeReviewFinding(ulong MessageIndex, string Claim, string Evidence, string Fix,
    string Verdict, string Severity, string Status)
{
    public bool Resolved => Status == "resolved";
    public string StatusKey => Status switch { "resolved" => "review.resolved", "unaddressed" => "review.unaddressed", _ => "review.open" };
}

public sealed record NativeReviewPresentation(string Summary, NativeReviewFinding[] Findings,
    string Model, string ReviewStatus, byte EvidenceCoverage, string[] CoverageGaps)
{
    public bool Unreviewable => ReviewStatus == "unreviewable";
    public bool NoFindings => Findings.Length == 0 && !Unreviewable;
    public string? FooterKey => Findings.Length == 0 ? null : Findings.All(f => f.Resolved) ? "review.all_fixed"
        : Findings.Any(f => f.Status == "unaddressed") ? "review.needs_attention" : "review.agent_correcting";
    public string CopyText => Summary + "\n\n" + string.Join("\n", Findings.Select(f => $"- {f.Claim}\n  Evidence: {f.Evidence}\n  Fix: {f.Fix}"));

    public static NativeReviewPresentation? Parse(ConversationItem item)
    {
        if (item.Role != "review") return null;
        try
        {
            using var document = JsonDocument.Parse(item.Text);
            var root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object) return null;
            _ = Text(root, "id"); _ = Text(root, "reviewer_backend");
            var model = Text(root, "reviewer_model").Trim(); var effort = Text(root, "reviewer_effort").Trim();
            var findings = Array(root, "findings").Select(f => new NativeReviewFinding(
                f.TryGetProperty("message_index", out var index) ? index.GetUInt64() : 0,
                Text(f, "claim"), Text(f, "evidence"), Text(f, "fix"), Text(f, "verdict"), Text(f, "severity"), Text(f, "status"))).ToArray();
            return new(Text(root, "summary"), findings, string.Join(" · ", new[] { model, effort }.Where(s => s.Length > 0)),
                Text(root, "review_status"), root.TryGetProperty("evidence_coverage", out var coverage) ? coverage.GetByte() : (byte)100,
                Array(root, "coverage_gaps").Select(g => g.GetString() ?? throw new JsonException()).ToArray());
        }
        catch (Exception ex) when (ex is JsonException or InvalidOperationException or FormatException or OverflowException)
        { return null; } // WebView also falls back to the original text for an invalid report.
    }

    private static string Text(JsonElement root, string name) => root.TryGetProperty(name, out var value)
        ? value.GetString() ?? throw new JsonException() : "";
    private static IEnumerable<JsonElement> Array(JsonElement root, string name) => root.TryGetProperty(name, out var value)
        ? value.EnumerateArray() : [];

    // Mirrors review_message_ui_index in ui/src/app_support/transcript.rs.
    public static int? MessageItemIndex(ConversationItem[] items, ulong messageIndex)
    {
        ulong seen = 0;
        for (var i = 0; i < items.Length; i++)
        {
            var item = items[i];
            var included = item.Role switch {
                "user" when item.Kind == "checkpoint" => false,
                "tool" => item.ToolName != "attempt_completion",
                "acp_tool" or "plan" or "question" => true,
                "review" => Parse(item) == null && !string.IsNullOrWhiteSpace(item.Text),
                "queued_user" or "file_changed" or "approval_pending" or "acp_permission" or "usage" or "compaction"
                    or "review_transition" or "app_context" or "system" or "checkpoint" => false,
                _ => !string.IsNullOrWhiteSpace(item.Text)
            };
            if (included && seen++ == messageIndex) return i;
        }
        return null;
    }

    public static int? JumpIndex(ConversationSnapshot page, int reportRow, int findingIndex)
    {
        // A user-turn offset cannot recover a reviewer message offset. Never
        // silently jump to an unrelated row when the beginning is not loaded.
        if (page.NextBeforeSeq != null || page.UserOffset is > 0 || reportRow < 0 || reportRow >= page.Items.Length
            || Parse(page.Items[reportRow]) is not { } report || findingIndex < 0 || findingIndex >= report.Findings.Length) return null;
        var index = MessageItemIndex(page.Items, report.Findings[findingIndex].MessageIndex);
        return index < reportRow ? index : null;
    }
}
