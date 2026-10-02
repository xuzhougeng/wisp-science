using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeActivityGroup(int Start, int End);

/// <summary>Conservative completed-process grouping. Unknown/actionable rows and
/// run monitors without a successful, non-actionable exact owner remain visible.</summary>
public static class NativeTranscriptActivity
{
    public static IReadOnlyList<NativeActivityGroup> Groups(ConversationItem[] items, bool busy)
    {
        var groups = new List<NativeActivityGroup>();
        for (var start = 0; start < items.Length; start++)
        {
            if (!ActivityAt(items, start)) continue;
            var boundary = Array.FindIndex(items, start + 1, row => row.Role is "user" or "queued_user");
            if (busy && (boundary < 0 || items[boundary].Role != "user")) continue;
            var limit = boundary < 0 ? items.Length : boundary;
            var end = start;
            for (var index = start; index < limit; index++)
            {
                if (ActivityAt(items, index)) end = index + 1;
                else if (!Glue(items[index])) break;
            }
            groups.Add(new(start, end));
            start = end - 1;
        }
        return groups;
    }

    private static bool Safe(ConversationItem row) => row.Ok != false
        && (row.Run is null || row.Run is { Status: "succeeded", NeedsReview: false })
        && (row.Status is null or "" or "completed" or "done" or "success" or "succeeded");

    private static bool Tool(ConversationItem row) => Safe(row) && row.Role == "tool" && row.Ok == true
        && row.ToolName is not ("attempt_completion" or "monitor_run" or "wisp_monitor_run" or "generate_image" or "generate_video");

    private static bool Glue(ConversationItem row) => Safe(row) && (row.Role is
        "usage" or "compaction" or "file_changed" or "app_context"
        || row.Role == "assistant" && string.IsNullOrWhiteSpace(row.Text));

    private static bool ActivityAt(ConversationItem[] items, int index)
    {
        var row = items[index];
        if (!Safe(row)) return false;
        if (row.Role == "reasoning" || Tool(row) || CompletedMonitor(items, index)) return true;
        if (row.Role != "assistant" || string.IsNullOrWhiteSpace(row.Text)
            || row.Text.StartsWith("Error: ", StringComparison.Ordinal)) return false;
        for (var next = index + 1; next < items.Length; next++)
        {
            if (Glue(items[next]) || items[next].Role == "reasoning" && Safe(items[next])) continue;
            return Tool(items[next]) || CompletedMonitor(items, next);
        }
        return false;
    }

    private static bool CompletedMonitor(ConversationItem[] items, int index)
    {
        var row = items[index];
        if (!Safe(row) || row.Role != "tool" || row.Ok != true
            || row.ToolName is not ("monitor_run" or "wisp_monitor_run")
            || row.Run is not { Status: "succeeded", NeedsReview: false, OwnerIndex: int owner } run
            || string.IsNullOrWhiteSpace(run.Id) || row.Input?.Trim() != run.Id
            || owner < 0 || owner >= index) return false;
        var submission = items[owner];
        return Safe(submission) && submission.Role == "tool" && submission.Ok == true
            && submission.ToolName is "run_in_context" or "wisp_run_in_context" or "transfer_between_contexts"
            && submission.Run?.Id == run.Id && submission.Run.OwnerIndex == owner;
    }
}
