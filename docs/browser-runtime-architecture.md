# Browser Runtime architecture

Wisp extension 0.5.0 treats the Chrome extension as a **controlled access adapter**.
The desktop process owns sessions, waits, staging, and approval.

```
Agent tools
    -> Browser Runtime (src-tauri/src/browser_bridge)
        -> ws://127.0.0.1:18765  shared / daily Chrome
        -> ws://127.0.0.1:<allocated-port>/<instance-token>  project workspace
            -> Manifest V3 extension (browser-extension/)
```

## Sessions

| Session | Browser | Login state | Port |
|---|---|---|---|
| `shared` | User's existing Chrome/Edge profile | Daily cookies and extensions | 18765 |
| `workspace` | Per-project Chrome-family profile under the app data directory, `browser-workspaces/<SHA-256(project_id)>/profile` | Persistent for that project | OS-allocated loopback port |

Both can be connected at once. Omitting `session` always selects `shared`;
workspace mode is used only when a tool explicitly passes `session=workspace`.

Each resolved lane has an independent Browser Task Lease, held until the turn
completes. `shared` remains one globally serialized lane. The tool-facing
`workspace` alias resolves from the trusted ToolEnv project id into that
project's running instance; tools reject missing project context and never
accept another project's internal lane name. Multiple projects can therefore
use their own workspaces concurrently while shared Chrome remains protected.

The registry owns each process, listener, profile path, extension copy, and
connection identity. Profile and extension directories use a SHA-256 digest
of the opaque project id, never a raw path component. Every start assigns a
new lane generation and WebSocket URL token. A client must present both the
bundled extension origin and the correct token, so a stale extension cannot
attach to a different project after a port is reused. Tabs, selected tabs,
pending requests, turn ledgers and human-verification state use resolved lanes.

Lifecycle transitions are serialized; normal commands on independent lanes
remain concurrent. Up to three workspace instances may run at once. At the
limit, start fails with instructions to stop an idle project's workspace.
Repeated starts reuse the existing process. `stop_workspace` stops only the
current project's instance, fails its pending requests, and drops its tab and
human-verification records. Project deletion also stops that project's instance.
Profiles are retained for the same project's later login sessions; the old
unscoped `browser-workspace` profile is never adopted by an arbitrary project.
After app restart, old workspace cleanup and human-verification records are
discarded because tab ids are process-local; shared records retain their
existing recovery behavior. Closing a Wisp window still leaves its running
sessions alive; idle-window reclamation and a dedicated workspace status UI
remain follow-up work.

### Workspace mode needs a build that still loads unpacked extensions

The workspace window is launched with `--load-extension` pointed at a copy
of `browser-extension/` whose `session_config.js` targets its own allocated endpoint and instance token.
**Official Google Chrome removed that flag in version 137** and now only
logs `--load-extension is not allowed in Google Chrome, ignoring`, so the
window opens with no Wisp extension. Chromium and Chrome for Testing keep
the flag.

So `resolve_browser()` prefers a build that honors the flag (Chromium,
Chrome for Testing, or whatever `WISP_WORKSPACE_BROWSER` points at) and
falls back to branded Chrome only because releases older than 137 still
work. `start_workspace` then **waits up to 20 s for the workspace
extension to connect**; if it never does it closes the window and fails
with `WORKSPACE_EXTENSION_BLOCKED` explaining the flag removal and the
shared-session alternative. It never reports a spawned process as ready,
which used to leave the agent driving a blank `about:blank` window (#952).

Shared mode is unaffected: the user loads the extension once from
`chrome://extensions` in the browser they already use. Wisp validates the
release package (manifest version, stable extension id, required files, and
SHA-256 inventory) and stages it under the application's stable data directory.
Chrome remembers that managed path instead of a version-dependent release
resource path.

When the running shared extension is older than the staged package, the app
shows both versions and the exact managed path. Extension 0.3.1 and later expose
`runtime_reload`, so future compatible upgrades can acknowledge a reload
request, restart their service worker, reconnect, and be rechecked without a
Chrome click. Older extensions cannot receive that control operation. Wisp then
opens the extension manager, copies the managed path, and waits for the user to
click **Reload** (or load the unpacked managed directory once). The app polls
the handshake and clears the update banner only after the required version is
actually connected.

### When a connected extension is not a usable session

The extension popup only reports whether its own socket is open, so it can
read *Connected to Wisp* while Wisp reports `connected=false`.
`browser_setup` therefore names the cause instead of only the symptom:

| Field | Meaning |
|---|---|
| `refused_connection` | A client reached a bridge port and was refused — a foreign extension id, or another loopback bridge on the port. Carries the observed `origin`, the `expected_origin`, and the handshake error. |
| `update_required` / `reload_required` | A connected extension is below the bundled version, below `protocol_version` 2, or reported no capabilities. `browser_setup action=update_extension` refreshes and verifies the managed copy, then either reloads a capable extension or returns `manual_reload_required`. |

## What the extension does

- Handshake: `extension_version`, `protocol_version=2`, `capabilities[]`
- Controlled service-worker reload (`runtime_reload`) after the managed package is verified
- Conditional wait (URL / selector / text / settle)
- Article scan (`images[]`, `figures[]`, `code_blocks[]`)
- Host-permission asset download into `Downloads/WispBrowserStaging`
- Viewport / full-page / selector capture
- Pause control from the popup (`USER_CONTROLLING`)
- Side panel (`side_panel`): on the user's request it reads the active tab's
  text and selection, sends the question as `side_ask`, and renders the
  `side_*` events of the resulting turn. It is the only traffic the extension
  initiates; pausing agent control does not disable it.

The extension never writes project directories and never returns large base64 files as the archive path.

## What the Runtime does

- Multiplexes one shared listener and up to three project workspace listeners
- Per-session Browser Task Leases with `shared` as the default and explicit
  `workspace` routing
- Copies staged files into the project and hashes SHA-256
- Starts/stops the workspace browser window and verifies it connected
- Per-turn ledger of tabs Wisp created (`web_open_tab` / tab-create), closed at turn end or confirmed in the UI
- Records the last refused connection so an unclaimed extension has a reason
- Maintains and verifies the stable shared-extension directory and rechecks the
  handshake after every update attempt
- In-browser chat one-shot (`web_agent_send` / `wait` / `read`) on an
  already-logged-in ChatGPT, Gemini, or Google AI Mode tab
- Runs a side panel question as an IM-origin agent turn in the main window's
  project (`browser_bridge/side_panel.rs`). A new panel conversation creates a
  session titled after the page; `side_started`, `side_delta`, `side_tool`,
  `side_approval`, and `side_done` / `side_error` return on the connection that
  asked.

Playwright is not used. The user's daily Chrome User Data directory is never passed as `--user-data-dir`.

## Project tab scope (#1617)

The shared daily browser mixes several projects' agents with the user's own
tabs, so the runtime keeps a **tab scope per project** on that lane. The scope
is derived from the trusted `ToolEnv` project id; nothing in a tool argument
can name another project, window or group to reach its tabs.

| Rule | Behaviour |
|---|---|
| Ownership | A project owns the tabs it opened (`web_open_tab`, tab-create commands), popups a page opened from one of those tabs, and tabs the user explicitly shared with it. Owned tabs keep that owner across turns, so a project that takes the shared lane later does not inherit tabs another project left behind. |
| Listing | `web_scan tabs_only` returns `tabs` (the scope), `unscoped_tabs` (the user's tabs no project owns) and a `scope` report with `project_id`, `agent_frame_id`, `window_id`, `group_id`, `window_pinning`, per-tab `opened_by` / `granted`, and `detached`. Tabs owned by **another** project are not listed at all. `{"cmd":"tabs"}` and the `remaining` list of a close command are narrowed the same way before the model sees them. |
| Selection | A requested `switch_tab_id` must be in scope. Omitting it selects the project's own last-used tab, then its active tab, then its first tab — never the profile's active or lane-selected tab. A project with nothing in scope is told to open a tab or ask for one. |
| Sharing | Passing the id of one of `unscoped_tabs` asks the user through the normal approval prompt (“Share browser tab …?”). Only a yes adds the tab to the scope, marked `granted`; the model cannot grant itself access. Another project's tab is refused without asking. A question asked from the extension's side panel shares that page's tab with the conversation's project without a second prompt, since the user started from it. |
| Embedded commands | Tab ids inside `{"cmd":"tabs"}` (`close`, `switch`) and `{"cmd":"cdp"}` get the same check as the outer tab id. CDP `Target.*` and `Browser.*` methods are refused in a scoped session because they can reach other targets. |
| Window pinning | The first tab a project opens fixes its window (`window_id`). Later tab-create commands carry that `windowId`, so switching the browser's foreground window does not move where the agent's tabs appear. When that window was closed, the extension opens a **new** window for the same owner (never another open window) and the binding follows the new window. |
| Tab groups | New tabs are placed in a native tab group titled `Wisp · <project name>` with a colour derived from the project id. The group is a label, not a boundary: the backend scope is what refuses out-of-scope access. |
| Revalidation | Every extension snapshot re-checks the scope: a closed tab leaves it; a tab the user moved to another window leaves it and is reported once under `detached`; a new `browser_instance` (the browser process restarted, so tab/window/group ids may now belong to someone else) clears every scope on that lane. Nothing is persisted across an app restart, so stale ids are never reused. |
| Cleanup | The per-turn ledger (and therefore auto-close and the cleanup prompt) now also covers popups the page opened from the project's tabs. Cleanup still closes only ledger tabs, never another project's or the user's. |

Capability negotiation: extension 0.5.0 advertises `tab_scope`, honours
`windowId` / `group` on tab-create commands and reports `browser_instance`.
With an older extension the access boundary above is still enforced by the
backend, but `scope.window_pinning` reads `unsupported_extension` and no
groups are created; the app does not claim pinning it cannot deliver.

What the scope is **not**: it does not isolate cookies, logins or site
storage — every tab in the shared profile still shares those. Use the project
workspace (`session=workspace`) for that. Workspace lanes belong to one project
already, so no narrower scope applies there. The shared cross-project lease
(`occupy_turn`) is unchanged: the shared lane still serves one project's turn
at a time; scoped parallelism is a separate, later decision.

Current limitations and follow-ups: same-project agents are told apart by
`opened_by` (the frame id) in the scope report, not by separate groups; the
Wisp UI does not yet show the scope or offer sharing outside the approval
prompt; a project's first tab still opens in whichever window Chrome treats as
current, and the binding is only fixed from then on.

## Safety checks

- `side_*` messages are honoured only from the live, origin-checked extension
  connection of their session. The panel can stop a turn or answer an approval
  only in conversations it has sent a message to since Wisp started, and an
  approval needs its exact id. Page text and the selection are length-capped,
  quoted as untrusted content, and cannot close their own quoting block. The
  panel receives tool names, never tool arguments or results. It renders the
  reply's Markdown into a fixed set of tags built node by node (`markdown.js`),
  never through `innerHTML`; links are limited to `http(s)` and `mailto`.

- `web_agent_*` accepts only already-open HTTPS tabs at `chatgpt.com` /
  `chat.openai.com`, `gemini.google.com`, or `google.com` with `udm=50`
  (Google AI Mode). Lookalike hosts such as `chatgpt.com.evil.com` or
  `gemini.google.com.evil.com` are rejected before any prompt is filled.
  Ordinary `google.com/search` tabs without `udm=50` are not treated as chat.
- `web_save_assets` `dest_dir` and `web_screenshot` `save_path` must be
  project-relative: absolute paths and `..` segments are rejected, so tool
  arguments cannot write outside the project root.
- The extension's pause gate parses each incoming command and lets only
  `cmd:"control"` through while paused; it never sniffs the raw request
  string.
- Tab ownership is decided from the host's project id, never from tool
  arguments. Out-of-scope tab ids, including ones nested in `tabs` / `cdp`
  commands, fail with `TAB_OUT_OF_SCOPE`; the user's own tabs join a scope only
  through an approval prompt the user answers.
