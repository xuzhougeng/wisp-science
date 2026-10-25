# Recovering an unresponsive window

When a Wisp window no longer accepts clicks, native recovery actions remain
available without asking the page's JavaScript to handle the click:

- **Windows:** right-click the Wisp notification-area icon. Choose **Stop agent
  in last active window** or **Reload last active window**. The target is the
  most recently focused Wisp workspace window, excluding the desktop pet. If
  that window was closed, the main window is used.
- **macOS:** use the native **Window** menu's **Stop current agent** or **Reload
  current window** action. Only the focused window handles the action.
- Linux currently has automatic recovery; these native menu entries are not
  exposed there.

Stop targets only that window's active session and uses the existing native/ACP
cancellation path. A window without an active session does nothing. Reload
recreates only that window's page; it does not restart Wisp, cancel a Run, or
terminate a remote SSH job. In-memory editor drafts and MCP App selections can
be lost on reload. Persisted sessions can be reopened, and backend agents and
the Run Manager remain alive. This does not guarantee survival of arbitrary
unmanaged remote shell processes when the entire application exits.

## Automatic recovery and evidence

Each workspace window reports a heartbeat every five seconds. The backend
tracks windows separately, checks every ten seconds, and reloads a focused
window after sixty seconds without a heartbeat. Returning from an unfocused
window grants a fresh sixty-second grace period. A window that never finishes
initializing is also covered. Attempts have a per-window two-minute cooldown,
including failed reload requests; failure to boot after a reload remains
eligible for another attempt. Closing a window removes its health state.

The log contains `webview health` once per minute per reporting window, a
`webview fault` warning the first time a document reports a renderer fault,
plus `native stop requested` and `webview recovery requested` records with a
window label and reload success/error. The heartbeat carries bounded
diagnostics:

- maximum visible-page timer delay and longest observed long task;
- cumulative long-task, script-error and unhandled-rejection counts;
- active and parked MCP App counts, cumulative incoming App messages;
- current drag-overlay count;
- live host media Blob URL count, encoded Blob bytes and mounted media-owner
  count (`mediaBlobUrls`, `mediaBlobBytes`, `mediaOwners`). These exclude MCP
  iframe images, decoded bitmaps, JS/WASM heaps and GPU allocations;
- the first renderer fault since the document loaded (`firstFault`): the first
  line of a Rust panic (`panicked at ui/src/…:line:col: message`, recorded by
  the WASM panic hook) or of a WebAssembly trap (`RuntimeError: …`), at most
  160 characters from the renderer and 200 in the log. Later faults are not
  recorded. A packaged app has no console, so this is where a trap that left a
  panel busy forever becomes visible.

Cumulative counters and maxima are bounded and reset with the document. No
conversation text, plugin arguments, image data, URLs or ordinary JavaScript
error messages are included; only a Rust panic or a WebAssembly trap is quoted,
because those name code, not content. Long-task metrics depend on browser
support. A living
timer does **not** prove that clicks work: healthy heartbeats with a stuck UI
call for checking overlays, pointer targeting and script failures. A native
reload request being accepted also does not prove that navigation completed.

## Long-running media sessions

Host chat images, videos, attachment thumbnails and research-journal previews
release their Blob URLs after both their mounted DOM owners and cache entries
let go of them. Full-media cache retention is limited to 64 entries / 64 MiB;
thumbnail retention is limited to 128 entries / 16 MiB. Mounted cards and
in-flight decodes can exceed those cache budgets. Small thumbnails share their
source URL safely, concurrent loads share work, and failed reads can be retried.
These limits apply to encoded Blobs, not total renderer memory.

This fixes a confirmed leak found during the #1179 follow-up: the previous
caches deleted lookup entries without revoking Blob URLs, retaining media until
the document was unloaded. It does **not** establish that this leak caused the
reported Windows stall after approximately five hours. An idle window without
media churn may have a different cause. The existing watchdog still handles
heartbeat loss; there is no periodic forced reload of a healthy window.

## Validation

Run the focused browser workload from `ui-tests`:

```powershell
$env:UI_TEST_PORT="1432"
npx playwright test tests/media-lifetime.spec.ts tests/webview-health.spec.ts tests/long-session-stress.spec.ts tests/chat-responsive.spec.ts --workers=1
```

The media workload cycles through 600 thumbnail loads, checks live Blob counts
after eviction, tests byte limits with large media, and covers shared URLs,
concurrent reads and cards removed while loading. It is an accelerated
lifecycle regression, not a five-hour WebView2 soak test. With 800×600 SVG
fixtures, the original implementation retained 400 / 800 / 1200 Blob URLs
after successive batches of 200 loads. The regression now checks that each
batch returns to 192 (64 source URLs plus 128 downscaled thumbnail URLs).

The mock workload combines a paginated long transcript, two simultaneous
session streams, and two App instances with 96 local thumbnail cards each.
One App remains parked while the other is visible. The test clicks a candidate,
Stop, tab close and New session, and checks numeric diagnostics. Each App must
finish rendering its thumbnails before it is clicked, and host responsiveness
is bounded by the page's own clock: the stream's 50 ms timer must keep firing,
and no main-thread task or timer lag may exceed three seconds while the
controls are used. The Playwright clicks themselves only have to complete within ten
seconds, so a busy CI runner cannot fail the test unless the host stalls. It
also guards against a model picker overflowing over Stop in a narrow chat
split.

Run the optional native smoke from the repository root:

```powershell
$env:WISP_CATALOG_OFFLINE="1"
cargo run -p wisp-tauri --example webview_recovery_smoke
```

This opens two hidden, disposable **real WebView2** windows on Windows (the
platform WebView elsewhere), with temporary browser profiles and fake backend
sessions. It blocks one page's JavaScript for fifteen seconds, exercises the
same Rust stop/reload functions as the native menus, and checks that the scoped
stop completes before the block ends and that only the target page reboots.
It does not open the user's SQLite database, contact MCP/model services, or
exercise a real SSH job. Native menu clicking, macOS execution, and the exact
SFL 0.6.7 incident still require separate manual validation.

Manual Windows check: open two project windows, focus the second, then use the
tray's stop and reload entries. Check the target label in the log, the first
window's unchanged state, and existing Run records. For a stopped heartbeat,
check that another window's healthy heartbeat cannot suppress target recovery.

For the five-hour Windows report, record the installed Wisp and WebView2
versions and compare `webview health` samples before and near the stall while
repeating the actual workload for at least six hours. Track host Blob counts
alongside the renderer's memory in Task Manager. Also compare an idle session.
If it stalls, record whether tray **Reload last active window** restores input
and whether the last heartbeats continued; this distinguishes page liveness
from input/compositor failures. Check that existing backend Runs continue.

## Scope and follow-up

This addresses recovery gaps and the reproduced narrow-pane click obstruction
reported while investigating #1179. It does not establish the original stall's
root cause. Independent MCP App WebViews, suspending parked Apps, candidate-list
virtualization and cross-session plugin-process sharing remain separate work,
to be guided by the new diagnostics and profiling of the real SFL workload.
