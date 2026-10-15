# Conversation Run cards

Runs launched by `run_in_context` (or `wisp_run_in_context`) and file transfers
launched by `transfer_between_contexts` stay visible in the conversation while
submitted, running, or cancelling. Once a Run succeeds, fails, is cancelled,
times out, or is lost, its card moves inside its submission tool row in the
**Processed / 已处理** activity group. Tool rows are collapsed by default. Expand
the group and the submission row to inspect the Run status, command, output,
environment, and available result-review actions.

Cards are linked by the exact Run ID in the tool result, including both
background submission results and `wait_for_completion` results. Multiple Runs
keep separate cards. An explicit `monitor_run` does not leave a duplicate card
outside the group after completion. Folding does not stop or delete a Run, and
folded cards remain accessible when reopening the conversation.

Runs without a matching submission in the loaded transcript retain the existing
standalone monitor behavior, including dismissal for completed cards. Historical
full Run results can restore their cards even when absent from the recent Runs
list; submission-only results still need their Run in that list.

## Runs tab in the right pane

Long sessions bury Run cards in the transcript. The right pane's **Runs / 运行**
tab (add it with **Add panel**, or from the View menu and command palette) lists
the active conversation's Runs and file transfers without scrolling: working
Runs (submitted, running, cancelling, paused) sit at the top, newest first, and
the tab label carries their count as a badge. Finished Runs collapse into a
**Completed** group below. The tab reuses the conversation Run card in embedded
mode, so cancel, result review, and the output preview behave the same, but
there is no dismiss action and transcript dismissals never hide a row here. Only
work that produces a Run record appears: `run_in_context` Runs and
`transfer_between_contexts` transfers. The synchronous `shell` tool and agent
workflows do not; workflows stay on the **Agents** tab.
