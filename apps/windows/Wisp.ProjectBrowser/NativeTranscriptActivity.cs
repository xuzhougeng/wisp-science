using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeActivityGroup(int Start, int End, bool CompletedTurn = true, bool Live = false);
public sealed record NativeActivitySummary(string Title, string? Note, string? Now, string? Duration, bool SettledOk);

/// <summary>WebView process grouping. Live tools retain their own step groups;
/// settled turns combine commentary, reasoning and tools. Active/unowned Run
/// cards and actionable rows remain boundaries.</summary>
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

    public static IReadOnlyList<NativeActivityGroup> DisplayGroups(ConversationItem[] items, bool busy, bool reviewActionsAvailable = false, bool modelView = false)
    {
        if (modelView) return [];
        var completed = Groups(items, busy, reviewActionsAvailable).ToDictionary(group => group.Start);
        var groups = new List<NativeActivityGroup>();
        for (var index = 0; index < items.Length;)
        {
            if (completed.TryGetValue(index, out var group)) { groups.Add(group); index = group.End; continue; }
            if (!StepTool(items, index, reviewActionsAvailable)) { index++; continue; }
            var end = index + 1;
            while (end < items.Length && (StepTool(items, end, reviewActionsAvailable) || Hidden(items[end]))) end++;
            // Queued prompts and usage cannot make the active tail historical.
            var live = busy && items[end..].All(row => Hidden(row) || row.Role == "usage");
            groups.Add(new(index, end, CompletedTurn: false, Live: live)); index = end;
        }
        return groups;
    }

    public static int[] Members(ConversationItem[] items, NativeActivityGroup group, bool reviewActionsAvailable = false) =>
        Enumerable.Range(group.Start, group.End - group.Start).Where(index => group.CompletedTurn
            ? ActivityAt(items, index, reviewActionsAvailable) || items[index].Role is "usage" or "compaction"
            : StepTool(items, index, reviewActionsAvailable)).ToArray();

    private static bool StepTool(ConversationItem[] items, int index, bool includeReview) =>
        Tool(items[index], includeReview) && ActivityAt(items, index, includeReview);

    private static bool Hidden(ConversationItem row) => row.Role is "file_changed" or "app_context" or "queued_user"
        || row.Role == "assistant" && string.IsNullOrWhiteSpace(row.Text);

    public static NativeActivitySummary Summary(ConversationItem[] items, NativeActivityGroup group, string key, NativeLocalization locale)
    {
        var tools = items[group.Start..group.End].Where(row => row.Role is "tool" or "acp_tool").ToArray();
        ulong total = 0;
        foreach (var row in tools) if (row.DurationMs is { } duration) total = ulong.MaxValue - total < duration ? ulong.MaxValue : total + duration;
        // The completed-turn clock needs actual turn timestamps. Tool elapsed
        // time is available for step groups, but must not pretend to be that clock.
        var elapsed = !group.CompletedTurn && total > 0 ? NativeToolPresentation.CompactDuration(total) : null;
        var last = tools.LastOrDefault();
        var waiting = group.Live && last != null && !NativeToolPresentation.Running(last)
            && (NativeToolPresentation.Completed(last) || NativeToolPresentation.IsFailure(last) || last.Status is "cancelled" or "canceled" or "interrupted");
        var title = group.CompletedTurn ? locale.Text("chat.activity_done")
            : group.Live ? locale.Text(waiting ? "chat.model_waiting" : "chat.steps_running")
            : locale.Format(tools.Length == 1 ? elapsed == null ? "chat.steps_1" : "chat.steps_1_time"
                : elapsed == null ? "chat.steps_n" : "chat.steps_n_time", ("n", tools.Length.ToString(System.Globalization.CultureInfo.InvariantCulture)), ("t", elapsed ?? ""));
        var note = waiting ? locale.Text("chat.model_wait_" + (System.Text.Encoding.UTF8.GetBytes(key).Sum(value => (int)value) % 4 + 1)) : null;
        var nowTool = group.Live && !waiting ? tools.LastOrDefault(row => row.ToolName != "update_plan") : null;
        var label = nowTool == null ? default : NativeToolPresentation.StepLabel(nowTool);
        var now = nowTool == null ? null : label.Title + (label.Detail.Length > 0 ? " · " + label.Detail : "");
        return new(title, note, now, group.Live ? elapsed : null, tools.All(NativeToolPresentation.Completed));
    }

    private static bool Safe(ConversationItem row, bool includeReview = false) => row.Ok != false
        && (row.Run is null || row.Run.Status == "succeeded" && (!row.Run.NeedsReview || includeReview))
        && (row.Status is null or "" or "completed" or "done" or "success" or "succeeded");

    private static bool Tool(ConversationItem row, bool includeReview = false) => (row.Run == null || Safe(row, includeReview)) && row.Role is "tool" or "acp_tool"
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
        "usage" or "compaction" or "file_changed" or "app_context" or "system" or "checkpoint"
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
        if (row.Run != null && !Safe(row, includeReview)) return false;
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
