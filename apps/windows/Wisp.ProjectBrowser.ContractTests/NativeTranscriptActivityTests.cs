using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeTranscriptActivityTests
{
    public static void Run()
    {
        ConversationItem Row(string role, string text = "text", bool? ok = null, string? tool = null, string? status = null)
            => new(role, text, tool, null, ok, status);
        var rows = new List<ConversationItem> { Row("user") };
        for (var phase = 0; phase < 6; phase++)
        {
            rows.AddRange([Row("assistant", $"phase {phase}"), Row("assistant", ""), Row("reasoning"),
                Row("tool", ok: true, tool: "python"), Row("file_changed"), Row("usage"), Row("app_context")]);
            if (phase == 3) rows.Add(Row("compaction"));
        }
        rows.Add(Row("tool", ok: true, tool: "update_plan"));
        var final = rows.Count;
        rows.AddRange([Row("assistant", "Final report"), Row("usage")]);
        Check(NativeTranscriptActivity.Groups(rows.ToArray(), false).SequenceEqual([new NativeActivityGroup(1, final)]), "six phases collapse once, final and trailing usage stay visible");
        Check(NativeTranscriptActivity.Groups(rows.ToArray(), true).Count == 0, "active tail stays visible");
        Check(NativeTranscriptActivity.Groups([.. rows, Row("user"), Row("reasoning")], true).SequenceEqual([new NativeActivityGroup(1, final)]), "historical turn folds while next turn runs");
        Check(NativeTranscriptActivity.Groups([.. rows, Row("queued_user")], true).Count == 0, "queued prompt does not complete active turn");
        foreach (var boundary in new[] { Row("question"), Row("tool", ok: false), Row("tool", ok: true, status: "running"),
            Row("tool"), Row("tool", ok: true, tool: "monitor_run"), Row("tool", ok: true, tool: "generate_image"),
            Row("assistant", "Error: failure"), Row("unknown_action") })
        {
            var groups = NativeTranscriptActivity.Groups([Row("reasoning"), boundary, Row("tool", ok: true)], false);
            Check(groups.SequenceEqual([new NativeActivityGroup(0, 1), new NativeActivityGroup(2, 3)]), $"action/error boundary remains visible: {boundary}");
        }
        Check(NativeTranscriptActivity.Groups([Row("assistant", "Answer"), Row("usage")], false).Count == 0, "ordinary answer is never process");
        Check(NativeTranscriptActivity.Groups([Row("tool", ok: true), Row("usage"), Row("compaction")], false).SequenceEqual([new NativeActivityGroup(0, 1)]), "trailing metadata stays outside");
        var run = new ConversationRun("run-a", "succeeded", 1, false);
        var submission = Row("tool", ok: true, tool: "run_in_context") with { Run = run };
        var monitor = Row("tool", ok: true, tool: "monitor_run") with { Input = " run-a ", Run = run };
        ConversationItem[] linked = [Row("user"), submission, Row("assistant", "Checking"), monitor, Row("assistant", "Report")];
        Check(NativeTranscriptActivity.Groups(linked, false).SequenceEqual([new NativeActivityGroup(1, 4)]), "exact successful monitor and commentary join submission activity");
        Check(NativeTranscriptActivity.Groups(linked, true).Count == 0, "successful monitor in active turn remains visible");
        foreach (var submitTool in new[] { "run_in_context", "wisp_run_in_context", "transfer_between_contexts" })
        foreach (var monitorTool in new[] { "monitor_run", "wisp_monitor_run" })
        {
            linked[1] = submission with { ToolName = submitTool };
            linked[3] = monitor with { ToolName = monitorTool };
            Check(NativeTranscriptActivity.Groups(linked, false).SequenceEqual([new NativeActivityGroup(1, 4)]), "compute, MCP and transfer monitor aliases share exact ownership rules");
        }
        foreach (var state in new[] { "running", "submitted", "paused", "cancelling", "failed", "timed_out", "cancelled", "lost", "future_state" })
        {
            linked[1] = submission with { Run = run with { Status = state } };
            linked[3] = monitor with { Run = run with { Status = state } };
            Check(NativeTranscriptActivity.Groups(linked, false).Count == 0, $"both submission and monitor stay visible for {state}");
        }
        linked[1] = submission;
        foreach (var invalid in new[] { monitor with { Run = null }, monitor with { Ok = false },
            monitor with { Run = run with { NeedsReview = true } }, monitor with { Run = run with { OwnerIndex = null } },
            monitor with { Run = run with { OwnerIndex = -1 } }, monitor with { Run = run with { OwnerIndex = 3 } },
            monitor with { Run = run with { OwnerIndex = 999 } }, monitor with { Run = run with { Id = "another-run" } },
            monitor with { Input = "different-run" } })
        {
            linked[3] = invalid;
            Check(NativeTranscriptActivity.Groups(linked, false).SequenceEqual([new NativeActivityGroup(1, 2)]), "missing/stale/failed/actionable monitor remains outside process");
        }
        linked[3] = monitor;
        linked[1] = submission with { Run = run with { NeedsReview = true } };
        Check(NativeTranscriptActivity.Groups(linked, false).Count == 0, "actionable owner cannot hide its monitor");
        Console.WriteLine("Native completed process grouping, final reports, running turns and actionable boundaries passed.");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
