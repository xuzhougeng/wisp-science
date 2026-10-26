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

## Move a Run to the background

While a turn waits on a Run (`monitor_run`, or `run_in_context` with
`wait_for_completion`), the conversation is busy until the Run ends. The bell
button on an active Run card, **Move to background / 转到后台**, ends that wait
without touching the Run: the agent gets `wait_backgrounded` and is told not to
monitor the Run again, finishes its reply, and the conversation takes new
messages while the Run keeps executing. The card stays live and notes that it
is running in the background. The button also works on a card no turn is
waiting on; it then only asks for the completion message.

When the Run succeeds, fails, is cancelled, times out, or is lost, Wisp appends
one message to the conversation that started it, with the Run title, outcome,
exit code, elapsed time, Run ID, and execution context, and raises a desktop
notification unless that conversation is showing in a foreground window. A
turn running in that conversation at that moment finishes first. The message
is part of the conversation history, so a later question about the result
reaches an agent that knows the Run ended. The agent is not resumed on its own.

The request is stored with the Run, so a Run that ends while Wisp is closed is
reported after the next launch. Cluster jobs submitted through
`scheduler_job.sh --follow` (Slurm `sbatch`, PBS, LSF, Grid Engine) are Runs
that last as long as the job, so they report the same way. Only Runs started
from a conversation can move to the background, and there is no way to bring a
backgrounded Run back into a waiting turn; ask the agent about it instead.

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

## Preflight budget

A `run_in_context` preflight runs the interpreter/package handshake with a
budget that scales with the language and the declared packages: R gets 15 s
plus 10 s per package (capped at 120 s) because every package is attached, and
Python gets 10 s plus 2 s per package (capped at 60 s) because only import specs
are resolved. A handshake that times out is retried once with twice the budget,
so a cold Bioconductor library on a fresh session is not reported as a broken
interpreter. The agent can set `preflight.timeout_secs` (1–600) explicitly when
a package is known to start slowly; a second timeout fails the `interpreter`
check with a message that names both budgets. Syntax parsing has its own fixed
15 s budget.

## Shell exit status on Windows

The synchronous `shell` tool runs its command inside a PowerShell script block
and reports the native tool's `$LASTEXITCODE` when one ran. Windows PowerShell
5.1 otherwise exits 1 whenever native stderr is merged into the pipeline
(`Rscript … 2>&1 | Tee-Object run.log`), because the stderr lines become error
records even though the tool succeeded. When no native command ran, a failing
cmdlet still yields exit 1, and an explicit `exit N` is passed through. A cmdlet
failure that follows a successful native command in the same script is still
masked by that native exit code; end such scripts with an explicit `exit`.
