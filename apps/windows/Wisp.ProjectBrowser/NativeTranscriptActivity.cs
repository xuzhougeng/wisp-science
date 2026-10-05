using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeActivityGroup(int Start, int End);

/// <summary>Conservative completed-process grouping. Unknown/unresolved rows and
/// run monitors without a successful exact owner remain visible. Review-needed
/// Runs fold only when the caller exposes their actions outside the group.</summary>
public static class NativeTranscriptActivity
{
    public static IReadOnlyList<NativeActivityGroup> Groups(ConversationItem[] items, bool busy, bool reviewActionsAvailable = false)
    {
        var groups = new List<NativeActivityGroup>();
        for (var start = 0; start < items.Length; start++)
        {
            if (!ActivityAt(items, start, reviewActionsAvailable)) continue;
            var boundary = Array.FindIndex(items, start + 1, row => row.Role is "user" or "queued_user");
            if (busy && (boundary < 0 || items[boundary].Role != "user")) continue;
            var limit = boundary < 0 ? items.Length : boundary;
            var end = start;
            for (var index = start; index < limit; index++)
            {
                if (ActivityAt(items, index, reviewActionsAvailable)) end = index + 1;
                else if (!Glue(items[index])) break;
            }
            groups.Add(new(start, end));
            start = end - 1;
        }
        return groups;
    }

    private static bool Safe(ConversationItem row, bool includeReview = false) => row.Ok != false
        && (row.Run is null || row.Run.Status == "succeeded" && (!row.Run.NeedsReview || includeReview))
        && (row.Status is null or "" or "completed" or "done" or "success" or "succeeded");

    private static bool Tool(ConversationItem row, bool includeReview = false) => Safe(row, includeReview) && row.Role is "tool" or "acp_tool" && row.Ok == true
        && row.ToolName is not ("attempt_completion" or "monitor_run" or "wisp_monitor_run" or "generate_image" or "generate_video");

    public static bool Completion(ConversationItem row) => Safe(row) && row.Role == "tool"
        && row.Ok == true && row.ToolName == "attempt_completion" && !string.IsNullOrWhiteSpace(row.Text);

    // The host may retain both the completion tool and its promoted assistant
    // answer. Fold only a confirmed duplicate, never the sole final answer.
    public static bool RepeatedCompletion(ConversationItem[] items, int index)
    {
        var row = items[index];
        if (!Completion(row)) return false;
        for (var next = index + 1; next < items.Length; next++)
        {
            if (Glue(items[next])) continue;
            return items[next].Role == "assistant" && Safe(items[next])
                && row.Text.Replace("\r\n", "\n").Trim() == items[next].Text.Replace("\r\n", "\n").Trim();
        }
        return false;
    }

    private static bool Glue(ConversationItem row) => Safe(row) && (row.Role is
        "usage" or "compaction" or "file_changed" or "app_context"
        || row.Role == "assistant" && string.IsNullOrWhiteSpace(row.Text));

    /// <summary>These actions must remain outside the collapsed rows. Only the
    /// UI that can expose them opts into folding review-needed successful Runs.</summary>
    public static string[] ReviewRuns(ConversationItem[] items, NativeActivityGroup group) =>
        items[group.Start..group.End].Where(row => row.Run?.NeedsReview == true)
            .Select(row => row.Run!.Id).Distinct(StringComparer.Ordinal).ToArray();

    private static bool ReviewedSubmission(ConversationItem row, int index) =>
        row.Role == "tool" && row.Ok == true && row.Run is { Status: "succeeded", OwnerIndex: int owner }
        && owner == index && !string.IsNullOrWhiteSpace(row.Run.Id)
        && row.ToolName is "run_in_context" or "wisp_run_in_context" or "transfer_between_contexts";

    private static bool ActivityAt(ConversationItem[] items, int index, bool includeReview)
    {
        var row = items[index];
        if (!Safe(row, includeReview)) return false;
        if (row.Run?.NeedsReview == true && !ReviewedSubmission(row, index) && !CompletedMonitor(items, index, includeReview)) return false;
        if (row.Role == "reasoning" || Tool(row, includeReview) || CompletedMonitor(items, index, includeReview) || RepeatedCompletion(items, index)) return true;
        if (row.Role != "assistant" || string.IsNullOrWhiteSpace(row.Text)
            || row.Text.StartsWith("Error: ", StringComparison.Ordinal)) return false;
        for (var next = index + 1; next < items.Length; next++)
        {
            if (Glue(items[next]) || items[next].Role == "reasoning" && Safe(items[next])) continue;
            return (Tool(items[next], includeReview) || CompletedMonitor(items, next, includeReview) || RepeatedCompletion(items, next))
                && ActivityAt(items, next, includeReview);
        }
        return false;
    }

    private static bool CompletedMonitor(ConversationItem[] items, int index, bool includeReview = false)
    {
        var row = items[index];
        if (!Safe(row, includeReview) || row.Role != "tool" || row.Ok != true
            || row.ToolName is not ("monitor_run" or "wisp_monitor_run")
            || row.Run is not { Status: "succeeded", OwnerIndex: int owner } run
            || string.IsNullOrWhiteSpace(run.Id) || row.Input?.Trim() != run.Id
            || owner < 0 || owner >= index) return false;
        var submission = items[owner];
        return Safe(submission, includeReview) && submission.Role == "tool" && submission.Ok == true
            && submission.ToolName is "run_in_context" or "wisp_run_in_context" or "transfer_between_contexts"
            && submission.Run?.Id == run.Id && submission.Run.OwnerIndex == owner;
    }
}
