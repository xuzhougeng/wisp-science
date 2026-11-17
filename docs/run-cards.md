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

## In the GPUI desktop

The GPUI desktop draws one card per Run under a tool row of the transcript,
with the same placement: under the call that waits on the Run while it goes
on (`monitor_run`, or a `run_in_context` still waiting with
`wait_for_completion`), and under its submission row once it has ended, with
no second card for an explicit `monitor_run`. It offers Ask, Move to
background (bell), Cancel (stop), Force cancel once it is cancelling, the
Run review dialog for a finished SSH Run whose workspace is still there
(folder) and Show in Runs. Moving a Run
uses the same shared code (`wisp_runs::background`) through the embedded
host's `background_run`; the completion message is the WebView's, written in
the window's language, or after a restart in the saved Language setting
(else English). The details, and where it differs from the WebView:

- Tool rows fold one by one rather than into a **Processed** group, so the
  card sits under its call row; the command, remote folder and output tails
  show when that row is opened.
- A Run of the conversation that no loaded call names (a scheduler job
  followed through `shell`, a second Run started while one waits) has a
  card of its own while it goes on and for a minute after it ends, placed
  as the WebView places its standalone cards: above the first message of
  yours sent after the Run was created, else after the last row. It shows
  the remote folder, command and output tails at once, and once the Run
  has ended it has a close button that dismisses it while the window is
  open. Cards under a call have no dismiss, as the WebView's cards folded
  into **Processed** have none.
- A Run already cancelling has Force cancel (a struck circle) on its card
  and in the Runs tab, as the WebView's second cancel: it marks the Run
  cancelled without waiting for its host. It goes through the shared
  `RunManager::cancel_with_intent` with the button's intent, as the Tauri
  desktop's native clients do, so a Cancel drawn before the Run began to
  stop never forces it. A method search is never forced.
- When a backgrounded Run ends and its conversation has the message, the
  desktop is told and posts one notification for the app (Task completed
  or Task failed, the conversation and the Run), through the notification
  and sound chosen in Settings, unless that conversation is on screen in
  the focused window. The WebView's in-window status line is not shown.
- No environment snapshot on the card.
- Moving a Run to the background is a desktop command, not a remote-access
  command, so the phone and web clients do not offer it.

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

Local output harvest honors `bundle: true`: matching files (including files
inside selected directories) become one deterministic tar.gz artifact, with one
logical output key. Repeating a harvest for unchanged contents reuses the same
artifact version. Local bundling uses Rust tar/gzip and needs no external `tar`
program on Windows or macOS. File and total copy limits apply to the archive.

本地产出收取支持 `bundle: true`，将匹配文件及所选目录中的文件打包为单个
tar.gz 产物。内容不变时重复收取不会新增版本；复制预算按压缩后的归档大小计算。
`write` 工具会在项目内创建缺失的父目录，并在创建前校验现有祖先路径及符号链接。

The `write` tool creates missing parent directories within the project after validating existing ancestors and symlinks.
