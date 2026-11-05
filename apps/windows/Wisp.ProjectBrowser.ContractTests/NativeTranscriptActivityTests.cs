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
        foreach (var boundary in new[] { Row("question"),
            Row("tool", ok: true, tool: "monitor_run"), Row("tool", ok: true, tool: "generate_image"),
            Row("assistant", "Error: failure"), Row("unknown_action") })
        {
            var groups = NativeTranscriptActivity.Groups([Row("reasoning"), boundary, Row("tool", ok: true)], false);
            Check(groups.SequenceEqual([new NativeActivityGroup(0, 1), new NativeActivityGroup(2, 3)]), $"action/error boundary remains visible: {boundary}");
        }
        Check(NativeTranscriptActivity.Groups([Row("assistant", "Answer"), Row("usage")], false).Count == 0, "ordinary answer is never process");
        Check(NativeTranscriptActivity.Groups([Row("tool", ok: true), Row("usage"), Row("compaction")], false).SequenceEqual([new NativeActivityGroup(0, 1)]), "trailing metadata stays outside");
        var completion = Row("tool", "## Final report\n\nResult.", true, "attempt_completion");
        ConversationItem[] completed = [Row("user"), Row("reasoning"), Row("tool", ok: true, tool: "python"),
            Row("assistant", "Finished checking."), completion, Row("usage"), Row("assistant", completion.Text), Row("usage")];
        Check(NativeTranscriptActivity.Groups(completed, false).SequenceEqual([new NativeActivityGroup(1, 5)]),
            "promoted completion and preceding commentary fold into one process, final answer remains visible");
        Check(NativeTranscriptActivity.Groups(completed, true).Count == 0, "completion in a running turn remains visible");
        foreach (var following in new[] { Array.Empty<ConversationItem>(), new[] { Row("assistant", "Different answer") },
            new[] { Row("user"), Row("assistant", completion.Text) }, new[] { Row("question"), Row("assistant", completion.Text) } })
            Check(!NativeTranscriptActivity.RepeatedCompletion([completion, .. following], 0), "sole, changed or next-turn results are never hidden");
        Check(!NativeTranscriptActivity.RepeatedCompletion([completion with { Ok = false }, Row("assistant", completion.Text)], 0),
            "a failed completion remains visible even if an answer follows");
        Check(NativeTranscriptActivity.RepeatedCompletion([completion, Row("assistant", completion.Text.Replace("\n", "\r\n"))], 0),
            "Windows newline differences do not duplicate the final result");
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
        linked[3] = monitor with { Run = run with { NeedsReview = true } };
        var reviewedGroups = NativeTranscriptActivity.Groups(linked, false, reviewActionsAvailable: true);
        Check(reviewedGroups.SequenceEqual([new NativeActivityGroup(1, 4)])
            && NativeTranscriptActivity.ReviewRuns(linked, reviewedGroups.Single()).SequenceEqual(["run-a"]),
            "successful reviewed Run and monitor share one process with one external review action");
        Check(NativeTranscriptActivity.Groups(linked, true, true).Count == 0, "review actions cannot fold the running turn");
        linked[3] = linked[3] with { Input = "foreign" };
        Check(NativeTranscriptActivity.Groups(linked, false, true).SequenceEqual([new NativeActivityGroup(1, 2)]),
            "review support does not relax monitor identity validation");
        linked[1] = linked[1] with { Run = run with { NeedsReview = true, OwnerIndex = null } };
        Check(NativeTranscriptActivity.Groups(linked, false, true).Count == 0, "unowned review-required Runs remain visible");
        DisplayAndDisclosure();
        Console.WriteLine("Native completed process grouping, final reports, running turns and actionable boundaries passed.");
    }

    private static void DisplayAndDisclosure()
    {
        ConversationItem Row(string role, string text = "text", bool? ok = null, string? status = null) => new(role, text, role == "tool" ? "python" : null, null, ok, status);
        var user = Row("user"); var reasoning = Row("reasoning");
        var tool = Row("tool", "", null, "in_progress") with { Input = "print(result)" };
        var usage = Row("usage", "{}"); var queued = Row("queued_user");
        ConversationItem[] rows = [user, reasoning, tool, usage, queued];
        var active = NativeTranscriptActivity.DisplayGroups(rows, true).Single();
        Check(NativeTranscriptActivity.DisplayGroups(rows, true, modelView: true).Count == 0
            && NativeTranscriptActivity.DisplayGroups(rows, false, modelView: true).Count == 0, "Model context preserves raw order in running and completed turns without narrative folding.");
        Check(active == new NativeActivityGroup(2, 3, false, true), "live tools group separately from reasoning; usage and queued prompts do not end live status");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> {
            ["en"] = new() { ["chat.activity_done"] = "Processed", ["chat.steps_running"] = "Working…", ["chat.model_waiting"] = "Waiting for the model…",
                ["chat.model_wait_1"] = "Wait 1", ["chat.model_wait_2"] = "Wait 2", ["chat.model_wait_3"] = "Wait 3", ["chat.model_wait_4"] = "Wait 4",
                ["chat.steps_1"] = "Ran 1 step", ["chat.steps_1_time"] = "Ran 1 step · {t}", ["chat.steps_n"] = "Ran {n} steps", ["chat.steps_n_time"] = "Ran {n} steps · {t}" },
            ["zh"] = new() { ["chat.steps_running"] = "执行中…" }
        }, "en");
        var summary = NativeTranscriptActivity.Summary(rows, active, "stable", locale);
        Check(summary.Title == "Working…" && summary.Now == "python · print(result)" && summary.Duration == null && summary.Note == null,
            "running header shows current input without inventing elapsed time");
        rows[2] = tool with { Ok = true, Status = "completed", Text = "done", DurationMs = 1250 };
        var waiting = NativeTranscriptActivity.Summary(rows, active, "stable", locale);
        Check(waiting.Title == "Waiting for the model…" && waiting.Note!.StartsWith("Wait ") && waiting.Now == null && waiting.Duration == "1s",
            "completed tool in the active tail shows model wait and known tool time");
        Check(waiting.Note == NativeTranscriptActivity.Summary(rows, active, "stable", locale).Note, "polling does not shuffle model wait copy");
        var unknown = rows.ToArray(); unknown[2] = tool with { Ok = true, Status = "future" };
        Check(NativeTranscriptActivity.Summary(unknown, active, "stable", locale).Note == null, "unknown lifecycle is not proof that the model is waiting");
        var later = rows[..4].Append(Row("assistant", "Streaming answer")).ToArray();
        var settled = NativeTranscriptActivity.DisplayGroups(later, true).Single();
        Check(!settled.Live && !settled.CompletedTurn && NativeTranscriptActivity.Summary(later, settled, "stable", locale).Title == "Ran 1 step · 1s",
            "an answer or reasoning after tools settles that phase while the turn remains active");
        var complete = NativeTranscriptActivity.DisplayGroups(later, false).Single();
        Check(complete == new NativeActivityGroup(1, 3) && NativeTranscriptActivity.Summary(later, complete, "complete", locale).Duration == null,
            "finished turn merges reasoning/tools before answer; tool time never masquerades as whole-turn time");
        foreach (var terminal in new[] { tool with { Ok = false, Status = "failed" }, tool with { Ok = true, Status = "cancelled" }, tool with { Status = "future" } })
        {
            ConversationItem[] stopped = [reasoning, terminal, Row("assistant", "Final answer")];
            var group = NativeTranscriptActivity.DisplayGroups(stopped, false).Single();
            Check(group == new NativeActivityGroup(0, 2) && !NativeTranscriptActivity.Summary(stopped, group, "stopped", locale).SettledOk,
                "failed, cancelled and unknown tool phases fold together with a neutral indicator");
        }
        ConversationItem[] multiple = [user, rows[2], Row("file_changed"), rows[2] with { DurationMs = 61000 }, usage, queued];
        var multipleGroup = NativeTranscriptActivity.DisplayGroups(multiple, true).Single();
        Check(multipleGroup == new NativeActivityGroup(1, 4, false, true)
            && NativeTranscriptActivity.Members(multiple, multipleGroup).SequenceEqual([1, 3]), "hidden metadata does not split a tool group or produce visible nested cards");
        Check(NativeTranscriptActivity.Summary(multiple, multipleGroup, "many", locale).Duration == "1m 2s", "elapsed aggregation uses recorded tool durations");
        Check(NativeTranscriptActivity.DisplayGroups([user, tool, Row("question"), tool], true).Count == 2, "questions remain between independent tool phases");
        Check(NativeTranscriptActivity.DisplayGroups([user, tool with { ToolName = "attempt_completion", Ok = true, Text = "Only final answer" }], true).Count == 0,
            "sole completion output remains a visible answer, never hidden as metadata");
        var acp = tool with { Role = "acp_tool", ToolName = "Read", Status = "completed", Ok = null };
        Check(NativeTranscriptActivity.Summary([acp], new(0, 1), "acp", locale).SettledOk, "ACP completion does not require legacy ok");
        var located = acp with { Status = "in_progress", Locations = "data/source.csv:12", Text = "second-choice output" };
        Check(NativeTranscriptActivity.Summary([located], new(0, 1, false, true), "acp-located", locale).Now == "Read · data/source.csv:12",
            "live ACP group and nested step share the same location-first preview");
        locale.Preview("zh"); Check(NativeTranscriptActivity.Summary([tool], new(0, 1, false, true), "zh", locale).Title == "执行中…", "running group follows shared locale");

        var disclosure = new NativeDisclosureState(true);
        Check(disclosure.Expanded && disclosure.Choice == null, "live group opens automatically without a user override");
        disclosure.Automatic = false; Check(!disclosure.Expanded, "settled tool phase closes if the user has not chosen");
        disclosure.Choice = true; disclosure.Automatic = true; disclosure.Automatic = false;
        Check(disclosure.Expanded, "explicit open survives lifecycle and polling");
        disclosure.Choice = false; disclosure.Automatic = true;
        Check(!disclosure.Expanded, "explicit close survives running updates");
        var refreshed = new NativeDisclosureState(true) { Choice = disclosure.Choice };
        Check(!refreshed.Expanded && !new NativeDisclosureState(false).Expanded, "render refresh preserves choice; a new completed-turn group starts collapsed");
        var empty = new NativeDisclosureState(true) { Choice = true, HasBody = false };
        Check(!empty.Expanded, "a terminal stub cannot restore an expanded empty body from a previous snapshot");
        empty.HasBody = true; Check(empty.Expanded, "real content arriving later can restore the user's disclosure choice");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
