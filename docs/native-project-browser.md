# Native project browser preview

Wisp has SwiftUI (macOS) and WinUI 3 (Windows) project browsers alongside the existing Tauri client.
It lists real projects, preserves the desktop's ordering and metadata, searches
names/descriptions/paths, refreshes on demand, and reveals a selected workspace
in Finder or Explorer. The existing desktop remains the client for chat and execution.

## Visual alignment with the WebView

The SwiftUI landing page follows `ui/src/styles/projects.css`: a centered page
with the Wisp wordmark and tagline, warm paper surfaces, teal actions, compact
project cards, and two equal columns. Below 820 points it switches to one column;
long pages scroll. The right column now contains the same five recent saved
sessions as the WebView. Clicking a project enters its workspace; clicking a
recent session opens that exact conversation. The workspace has project switching,
a collapsible left session sidebar, a main transcript pane, and a back-to-projects
action. Saved transcripts load in pages of 20 user turns with an older-messages
control. Reading does not mark messages seen or change the WebView's active session.

The home search icon (Command-K) opens a dismissible search sheet for projects
and recent sessions. Up/Down selects a result and Enter opens it; filtering
resets selection. Escape closes only the topmost menu or search sheet, and IME
candidate selection keeps its keyboard handling. Inside a project, the same
search layer searches its saved conversations; Command-K also works with the
sidebar collapsed. Database selection and refresh live in the preview footer,
so they no longer occupy the WebView's primary project-action positions. The footer provides system/light/dark
appearance choices and the successful-read time; hover over the database filename
to see its full path.

Brand SVGs, the existing `compose_icon()` glyphs, and semantic colors are exported
from the WebView sources into the Swift resource bundle. There is no second icon
set and no WebView embedded in the SwiftUI screen. After changing those shared
sources, run:

```bash
python3 scripts/sync_native_design.py
python3 scripts/sync_native_design.py --check
```

CI and both app builds check for asset drift. WinUI links the same SVG and
palette files into its output. The exporter handles UTF-8 source and Windows CRLF checkouts. Native system font rendering,
window chrome, and file selection remain platform-specific; unsupported WebView
actions retain their WebView positions but are disabled and labeled as not yet connected.

## Build and run on macOS

Requires macOS 13+, Xcode with Swift 5.9+ command-line tools, and the repository's
Rust toolchain. No Swift package dependencies are downloaded.

```bash
bash scripts/build_native_macos.sh
open "target/native-macos/Wisp Science Preview.app"
```

This builds a debug app for the current architecture, bundles `wisp-service`, and
applies an ad-hoc local signature. It is a local preview, not a notarized release
or universal installer. Its bundle ID is `science.wisp-science.native-preview`;
it has separate preferences and no updater.

The default database matches the current desktop location:
`~/Library/Application Support/science.wisp-science/wisp-science/wisp.sqlite`.
Use **Choose database / 选择数据库** (Command-O) for another existing database;
**Refresh / 刷新** (Command-R) reloads it. Database selection is remembered by the
preview. For development, `WISP_BROWSER_DATABASE` overrides the path and
`WISP_SERVICE_PATH` overrides the bundled service executable.

Queries open SQLite read-only. Clicking a project-card star explicitly launches
`wisp-service --database <path> --allow-project-writes` and sends the desired
Boolean state. Only that command opens an existing writable connection; it does
not create a database, run migrations, change journal mode, access credentials,
or execute tools. Database selection and refresh remain queries.

Stars are local project metadata shared with the WebView. They do not touch
activity timestamps, workspace files, or selected sessions. While saving, star
buttons, refresh, and database selection are disabled to prevent competing
updates. The UI applies the server's ordered snapshot only after success. Failures
preserve the previous display and show an error; refresh to confirm the persisted
state if a response was lost. Repeating the same desired state is idempotent.

Databases from before project stars are readable without migration, with projects
treated as unstarred. Attempting to save a star explains that the current WebView
desktop must upgrade the database first. Other incompatible older schemas may
fail to query. SQLite may use ordinary WAL/SHM coordination files when a desktop
writer is also active. Native star writes do not push events to an already-open
WebView; refresh its home list to see the change (and vice versa).

## Build and run on Windows

Requires Windows 10 1809+ (x64), .NET 8+ SDK, Python 3, and the Rust toolchain.
Native settings also require the Microsoft Edge WebView2 Evergreen Runtime.
The Windows App SDK and SDK build tools are restored from pinned NuGet packages;
Visual Studio's packaging workload is not required. From the repository root:

```powershell
pwsh -File scripts/build_native_windows.ps1 -Launch
```

Use `-Python C:/path/to/python.exe` if Python is not on PATH. The output is
`target/native-windows/Wisp.Science.Preview.exe` with its companion files.
The directory includes the .NET and Windows App SDK runtimes, `wisp-service.exe`,
and a full desktop settings host under `settings-host/`;
copy the entire directory, not just the executable. This is an unsigned local x64
preview, not an installer or an update to the installed Tauri application.

The default database is `%APPDATA%/science.wisp-science/wisp-science/wisp.sqlite`.
The preview caches its database selection, theme and palette in
`%LOCALAPPDATA%/WispSciencePreview/settings.json`. `WISP_BROWSER_DATABASE` and
`WISP_SERVICE_PATH` have the same meaning as on macOS. Ctrl+K opens home/project
search, Ctrl+R refreshes, and Ctrl+O opens the native file picker. The native
text context menu, project/appearance flyouts, search dialog and file picker
consume Escape from the topmost surface.

Windows uses device-independent layout units: at 150% scaling, a 1200-pixel
window is only 800 units wide. Home keeps independently scrolling project and
recent-session columns down to 680 units; below that they stack with separate
scroll areas. Home actions remain at the top right, wrapping within that corner
on smaller windows. The entire conversation action strip wraps to a second
right-aligned row instead of dropping entries at the compact breakpoint. A compact,
bounded sidebar tool area preserves room for sessions in short windows, with
preview utilities at its foot. The composer remains visibly disabled. Shared
icons and native controls are used throughout; there is no WebView in this client.

The Windows client starts a hidden process per query, drains stdout/stderr
concurrently, checks the full response envelope, and kills/reaps the process on
cancellation or a 20-second deadline. Output is bounded to 32 Mi characters and
stderr to 64 Ki characters. Failed refreshes retain the previous snapshot and
show an error; changing databases clears it. Independent refresh/navigation/
transcript generations prevent late queries reopening old projects or sessions.
Closing the window cancels active queries. Search is a compact native overlay
with a scope label, icon-bearing results, IME-aware keyboard navigation and no
large dialog footer. Saved transcript text is selectable; headings, lists,
emphasis, HTTP(S) links and code text render through Markdig and native XAML.
HTML stays inert and images are represented by alt text without downloading.
Tool results and the v1 service's name/JSON argument lines are collapsed behind
expanders; the full saved content remains available, with argument strings
decoded to show real Unicode and newlines. Rich attachments, tables, and
interactive tool execution surfaces remain follow-ups.

### Current Windows milestone and validation boundary

The Windows preview now implements the read-only home → project → saved-session
path established by PRs #1274 and #1276: recent-session deep links, project/session
selection, search, history pagination, database selection, refresh, themes and
Explorer reveal. Creation/import, chat submission, model selection, live runs,
approvals, and sidebar tool services are still disabled. This is an incremental
preview, not completed parity with the production WebView.

The first local visual pass verified real data, the two-column home, the usable
session list, initial search/project-menu Escape handling and latest-message
scroll restoration. User screenshot feedback led to a further top-right action
layout, compact search overlay, persistent narrow-window action strip and native
Markdown/tool folding. That build compiles and its final home was visually
rechecked. The final search overlay (including nested text-menu Escape and IME),
action-strip wrapping, tool expanders and Markdown still require a complete
manual pass; automated desktop control was stopped by the user. Do not treat
the earlier dialog checks as validation of the replacement overlay.

## Status semantics

The standalone preview has no access to the desktop's in-memory Agent and
approval state. Responses explicitly declare `activity_source: persisted_only`.
The UI shows saved session/artifact counts, saved unread replies, and sync
metadata; it labels live execution and approval status as unavailable. A zero
`running_count` in this mode must **not** be presented as proof that no work is
running. Failed refreshes retain the last successful snapshot with its timestamp
and an error banner; selecting a different database clears the old snapshot.

## Shared native boundary

- `wisp-app::projects` owns project-list queries, activity enrichment, and the project-star command. The
  existing Tauri command calls the same service with real runtime snapshots.
- `wisp-dto::project_browser` owns the native protocol shapes.
- `wisp-service --database <path>` exposes those queries over stdin/stdout JSONL.
- `apps/macos` contains a Foundation transport client and SwiftUI presentation.
- `apps/windows/Wisp.ProjectBrowser.Contracts` provides `IProjectBrowserClient`
  and C# response/project/session/transcript/command DTOs.
- `apps/windows/Wisp.ProjectBrowser` contains the transport, testable navigation
  state and layout breakpoints; `Wisp.Science.Preview` provides the WinUI window.
- `contracts/project-browser/v1/{projects,sessions,transcript,set-project-starred}.json` are decoded by Rust, Swift, and
  the C# contract smoke test to detect wire-format drift.

The UI never queries SQLite directly. Both adapters start one short-lived
service per query and closes stdin after one request. The service also accepts
multiple requests per process, enabling a future persistent adapter.

Each UTF-8 request is one JSON line, at most 64 KiB including its newline:

```json
{"schema":"wisp.project-browser.v1","id":"projects-1","type":"list_projects"}
{"schema":"wisp.project-browser.v1","id":"sessions-1","type":"list_sessions"}
{"schema":"wisp.project-browser.v1","id":"sessions-2","type":"list_sessions","project_id":"project-id"}
{"schema":"wisp.project-browser.v1","id":"transcript-1","type":"get_transcript","project_id":"project-id","session_id":"session-id","before_seq":null}
{"schema":"wisp.project-browser.v1","id":"capabilities-1","type":"capabilities"}
```

Every response repeats `schema` and `id`. A `projects` response contains
`projects: ProjectSummary[]` and `activity_source: persisted_only`. A
`capabilities` response contains `commands` and `read_only: true` by default.
With `--allow-project-writes`, it additionally advertises `set_project_starred`
and `read_only: false`. The additive v1 command is:

```json
{"schema":"wisp.project-browser.v1","id":"projects-1","type":"set_project_starred","project_id":"project-id","starred":true}
```

Success returns the same ordered `projects` response as `list_projects`. Without
explicit write mode it returns `write_disabled`. An `error`
response contains `code` (`invalid_request`, `unsupported_schema`, or
`query_failed`, `write_disabled`, or `command_failed`) and `message`. Malformed requests have `id: null`. Stdout carries
protocol responses only; startup/transport failures go to stderr and exit
nonzero. EOF exits the process. Clients validate schema, correlation ID, response
type, and supported activity source before presenting results. The macOS client
terminates a service that exceeds its 30-second query deadline.

## Verification

```bash
cargo test -p wisp-app -p wisp-service
swift test --package-path apps/macos --scratch-path target/native-macos/swift
dotnet run --project apps/windows/Wisp.ProjectBrowser.ContractTests -- contracts/project-browser/v1/projects.json
```

The Native Preview workflow runs the Swift build/tests on macOS and the C#
contract/transport/navigation checks, Rust service tests and full WinUI publish
on Windows, uploading the runnable Windows directory as an artifact. Tests use temporary databases,
shared JSON fixtures, and a fake child process; no API key, remote host, or model
is required. Swift presentation tests cover filtered selection, project identity,
both palettes, and native SVG loading for the bundled wordmarks/icons. C# tests
exercise Unicode/spaced paths, pipe pressure, malformed/mismatched envelopes,
service failures, deadline/cancellation process cleanup, exact-session navigation,
database changes during refresh, stale responses, pagination, DPI breakpoints and
lossless separation of tool arguments from ordinary prose.

Manual smoke steps:

1. Compare home project ordering and the five recent sessions with the WebView.
2. Click a project: verify the left session list and main conversation pane.
3. Return home and click a recent session: verify the exact project/session opens.
4. Switch sessions, switch projects, collapse/reopen the sidebar, and return home
   while a query is loading. Old responses must not reopen a previous workspace.
5. Load older messages in a long conversation and confirm no duplicated rows.
6. Open home search, appearance/project menus, or the database chooser, then
   immediately press Escape. Only the topmost surface should close.
7. Check light/dark themes and narrow windows. Refresh and directory reveal must
   still work. On Windows, check 1200×850 physical pixels at 150% scaling: recent
   sessions must remain beside projects and the session list must have usable height.
   Failed queries must offer visible errors rather than blank content.

## Shell alignment checks

| WebView surface | Native preview |
| --- | --- |
| Home header | Same calendar/library/search/settings/scratch/import/new-project order. Search, library, calendar, scratch, new project, and import are connected. The WinUI 随手一聊 button stays disabled. |
| Home content | Projects left, five recent sessions right; cards navigate into a workspace. |
| Project shell | Back/project switch/collapse at the top of the left sidebar, navigation above saved sessions, utility entries below. |
| Session controls | 选择, 排序与分组, and 新建分组 are connected. The old 新建文件夹 label was the session-group action. |
| Conversation | Session title and action strip above, scrollable saved transcript in the center, composer position below. macOS 对话附件 copies a local file into the project and shows it on the saved message. The WinUI button stays disabled. |
| Search | Home/project scope, Up/Down and Enter navigation, topmost Escape, Command-K / Ctrl+K even with the sidebar collapsed. |
| Preview utilities | Database selection, refresh and appearance remain in the home footer / Windows sidebar footer; these do not replace WebView actions. |

## Remaining feature work

The preview aligns the home/workspace shell and includes native settings, project
creation, project import, the library, the research calendar, the research journey, the publication workspace, the capability summary, issue feedback, scratch chat, and the conversation loop described below.
The sidebar tools other than 文件, 新建分组, 收藏, 研究历程, 论文证据, 能力, and 反馈问题 still require their native services.
Those remaining action slots are visible but explicitly disabled in the preview.

The sidebar **新建分组** button creates a session group for the explicit project.
Sessions can be sorted by recent or name, grouped by folder or date, and
selected and moved. A folder section header can rename that group. Those
commands are `native_project_folders`, `native_project_folder_create`,
`native_project_folder_rename`, and `native_project_session_move`. Each one
requires a project id and does not change the WebView's active project or
session. An empty name or a lost reply is not retried, and the rename draft
stays open. Escape closes only the new-group or rename sheet while a sort menu
under it stays open.

The sidebar **文件** button selects the existing right-hand files page and
expands that panel. It uses the same tab layout as the panel itself and does
not add a host command. Escape continues to dismiss only the panel's own top
surface.

## Library

**收藏** on the home header and in the project sidebar opens the same library
sheet. Search and delete go to the desktop host's app-global library store
(`library.sqlite` via `AppState.library`), the same store the WebView library
uses. `wisp-service` cannot search or delete that store.

`native_library_search` takes a query and an optional kind (`code`, `figure`,
or `text`). An empty query lists the library. `native_library_delete` removes
one item by id. Neither command takes a project id. A non-empty project id is
rejected and nothing is deleted. The dispatcher does not create a settings
webview and does not change the WebView's active project or session.

Both commands are announced on `native_settings_capabilities` as `library` and
`library_schema` (`wisp.native-library.v1`). They are not in the settings
command allowlist. A lost search keeps the current list and is not retried. A
lost delete keeps the row and is not retried. A second click while that delete
is in flight does not send again.

**填入对话框** is shown only when a native session is already open. It writes
the item text into that session's composer and does not send. The home page
has no active session, so the button is absent there. **打开来源** closes the
sheet and opens the item's source project and session through the native
project list. It does not call `set_active`.

Escape immediately after the sheet opens closes only the library sheet. A
search surface that was already open stays open. A search reply that arrives
after the sheet has closed does not reopen a project.

WinUI decodes the same fixtures through `INativeLibraryClient` and leaves its
收藏 buttons disabled.

## Research calendar

Home **研究日历** opens one calendar sheet. It asks `native_research_calendar`
for the current local month, then for the selected day. The body lists
`project_ids`, `from`, and `until`. The command does not take a project id, does
not create a settings webview, and does not change the WebView's active project
or session. It is announced as `calendar` and `calendar_schema`
(`wisp.native-calendar.v1`) and is not in the settings allowlist.

The request includes the projects currently listed on the home screen. Opening
or refreshing the sheet first calls `get_privacy_mode`. That command reads the
privacy list the WebView writes with `set_privacy_mode` into the desktop
settings store (`wisp-privacy-mode-active` and `wisp-privacy-mode-projects`).
It does not take a project id. When privacy mode is on, those project ids are
left out of the calendar request. A lost privacy read does not ask for every
project and is not retried. A project
filter only hides rows that were already read; it does not add a project. A
lost read keeps the last rows and is not retried. Choosing a date shows that
day's records inside the sheet. **打开研究历程** closes the calendar and opens
that project on the dated research-journey entry. A calendar reply that arrives
after the sheet has closed does not open a project.

Escape immediately after the sheet opens closes only the calendar. WinUI
decodes the same fixture through `INativeCalendarClient` and leaves its
研究日历 button disabled.

## Research journey

The sidebar **研究历程** button, and the calendar's **打开研究历程** action,
open the same journey sheet for one project. `native_research_journey` requires
that project id and reads only its mainline history for the requested range.
The calendar passes the selected day; the sidebar reads the current local month.
The command is announced as `journey` and `journey_schema`
(`wisp.native-journey.v1`). It is not in the settings allowlist and does not
change the WebView's active project or session.

A lost read keeps the last rows and is not retried. Search filters the rows
already read. A reply that arrives after the sheet has closed does not open or
change a project. Escape closes only the journey sheet. Adding a journal
entry, artifact detail, and run detail stay out of this slice. WinUI decodes
the same fixture through `INativeJourneyClient` and leaves its 研究历程 button
disabled.

## Publication workspace

The sidebar **论文证据** button replaces the conversation column with the
publication workspace for the open project. `native_publication_workspace`
reads that project's papers. `native_publication_create` creates one paper and
its first revision. Both commands require the project id, are announced as
`publication` and `publication_schema` (`wisp.native-publication.v1`), and are
not in the settings allowlist. They do not change the WebView's active project
or session.

An empty title or revision label keeps the draft and does not call the host.
A lost create keeps the draft and is not retried. A reply that arrives after
the workspace has closed does not open a project. Escape closes only the
publication column and returns to the conversation. Evidence binding, readiness,
and reproduction stay out of this slice. WinUI decodes the same fixture through
`INativePublicationClient` and leaves its 论文证据 button disabled.

## Capabilities

The sidebar **能力** button opens a summary for the current project. It reads
`get_bootstrap_status`, `list_skills`, `list_mcp_connections`, and
`get_memory_view` through the existing settings host. Each call carries that
project id. Enabled bundled skills are counted separately from other enabled
skills. Enabled connections and memory files are counted from those replies.
The summary does not call `probe_execution_context` and does not add a host
command. A lost read is not retried. A reply that arrives after the sheet
closes does not open a project. Choosing a count opens the existing settings
section for skills, connections, or memory. Escape closes only the summary.
WinUI leaves its 能力 button disabled.

## Issue feedback

The sidebar **反馈问题** button is available when a native session is open. It
reads `get_bootstrap_status` for that project and writes the same feedback
prompt the WebView builds into the current composer. The prompt includes the
app version, OS, architecture, model, and startup timing. It does not include
the workspace path. The button does not send the message. A lost read keeps the
composer unchanged and is not retried. A reply that arrives after the user has
returned home does not prefill a composer or open a project. WinUI leaves its
反馈问题 button disabled.

## Scratch chat

Home **随手一聊** calls `native_scratch_open`. That command creates a hidden
`scratch:` project, one session frame, and a writable sandbox directory under
the desktop app-data `scratch/` folder. It does not take a project id, does not
call `start_scratch_chat`, and does not change the WebView's active project or
session. The preview then opens that session with the existing native
conversation loop.

**关闭** and Escape call `native_scratch_close` for that scratch project id.
The command deletes the project row and the sandbox directory when the
directory is inside `scratch/`. It does not restore or rewrite another project,
and it refuses a normal project id or a scratch id whose workspace is outside
`scratch/`. A lost open or close is not retried. Closing the preview without
this command leaves the project and sandbox for the existing startup purge:
that purge records the orphans before any new scratch chat can be created, so
a chat opened while the purge is still running is spared. WinUI decodes the
same fixtures through `INativeScratchClient` and leaves its 随手一聊 button
disabled.

## Creating a project

The home **新建项目** button opens a SwiftUI form with 名称, 工作目录, 说明,
Agent Context, and a 标准布局 switch. The directory can be typed or chosen with
the system open panel. `native_project_create` runs on the desktop host and uses
the same checks as the WebView `create_project` command: create a missing
directory, then reject an empty name, an empty directory, a folder already
registered as a project, or a directory that is not writable. The command takes
no project id. It does not change the WebView's active project or active session,
and it does not create a conversation.

With 标准布局 off, the host does not precreate the standard workspace tree.
Non-empty Agent Context is still written to `.wisp/WISP.md`. Turning the switch
on inserts the same convention block the WebView editor inserts; turning it off
removes that block and leaves the rest of the text.

The command is announced on `native_settings_capabilities` under `projects` and
`project_schema` (`wisp.native-projects.v1`). It is not in the settings command
allowlist and `wisp-service` cannot create projects. A validation error keeps the
form and draft open. A lost reply does the same and is not retried; refresh the
project list to see whether the host finished. A confirmed summary closes the
form, reloads the read-only project list, and opens that project. Escape
immediately after the form opens closes only the form. While the request is in
flight the submit button stays disabled, including a second click.

WinUI decodes the same fixture through `INativeProjectClient.CreateAsync` and
leaves its 新建项目 button disabled.

## Importing a project

The home **导入项目** button opens the system file panel for a `.zip` archive.
Canceling the panel does not call the host. A chosen path is sent once as
`native_project_import`, with no project id. The host reads and verifies the
archive with the existing project-transfer code, places the workspace next to
the archive, and registers it. It does not open the Tauri file dialog and does
not consult the WebView's exploration-branch window. Progress is a busy state,
not a streamed bar. A lost or invalid reply stays on the home screen and is not
retried; refresh the project list to see whether the import finished. A
confirmed summary reloads the read-only list and opens that project. A second
click while the request is in flight does not send again.

`native_project_import` is announced next to `native_project_create` on the
host capability document. `wisp-service` still cannot import projects. WinUI
decodes the same fixture through `INativeProjectClient.ImportAsync` and leaves
its 导入项目 button disabled.

The transcript renders text, tool records and basic questions; rich attachments,
branch/review cards and interactive tool surfaces remain follow-ups.

Windows now has a WinUI 3 project/session preview, in-window appearance
settings, a live conversation loop (create/send/stop/approve/model), and
workspace actions that consume the #1284/#1288 contracts: outline, share
(HTML), trajectory, archive, inbox, a text terminal pane, and a right-hand
panel with file create/rename/delete, save, hosts, agents, notebook stars,
highlights and side-chat. PNG share export, a VT terminal emulator, the
remaining 18 settings editors, attachments and ACP composers remain
follow-ups. Windows Markdown is intentionally limited to native text
formatting, with no interactive HTML or attachment rendering.

## Native settings

The macOS preview now includes SwiftUI settings (Cmd+,) backed by the full
desktop runtime. See [native-settings.md](native-settings.md) for scope,
architecture, native/WebView differences, smoke steps and the WinUI 3 transport
interface. The project-browser service remains focused on project/session reads
and project stars.

### Windows alignment after #1281

This increment carries forward #1279, merges #1281 and enables:

- Explicit project star/unstar writes through `--allow-project-writes`, applying
  the server-ordered snapshot after success. Failures keep the previous list;
  no mutation is retried automatically. Conversations remain read-only.
- Home top-right and sidebar settings buttons open settings inside the existing main window. Back returns to the prior home/project/session.
  It pins the database/project for the editing session and reads/writes the
  shared appearance preference document through the authenticated loopback host.
- Theme, both palettes and interface/code font sizes have explicit Save and
  Cancel changes. Refresh preserves dirty fields. Unknown preference fields,
  including font families and WebView custom CSS, survive round trips.
- Saved theme/palette apply to the WinUI browser. The appearance card previews draft palettes and font sizes. Font sizes are saved for the
  desktop, but full WinUI font application, font-family
  editors and CSS editing are follow-ups. The other 18 settings sections remain
  unimplemented on Windows and remain disabled in the shared categorized navigation.
- Returning with a dirty draft first displays an inline warning; a second Back
  discards it. Pending reads can be cancelled by returning. Escape closes an
  open combo dropdown before leaving settings. Leaving settings never kills
  the shared desktop host. Connection attempts have a 15-second deadline; an
  incompatible older desktop intercepting startup produces an actionable error.

The build script bundles the full Rust desktop host and its resources, using an
inert HTML document for legacy command extraction; the settings UI is native
XAML. `WISP_SETTINGS_HOST_PATH` can override the bundled host executable. The
host uses the standard desktop database: selecting another database in the
browser does not reconfigure the host. Alternate databases require an already
running matching host descriptor; settings never fall back to another database.
The unsigned preview does not install WebView2 or deliver an automatic update.

Additional smoke checklist:

1. Build the complete output directory and open Settings from home and sidebar.
2. Read the default database appearance, change a palette, cancel and verify the
   saved value returns; change again, save and compare with the desktop client.
3. Refresh a dirty draft; it must remain intact. Simulate an unavailable host;
   the error remains visible and the draft is not cleared or silently retried.
4. Open a theme dropdown and immediately press Escape: only the dropdown closes.
   Press Escape again: clean settings return to the previous page; dirty settings ask before discard.
5. Resize the main window while viewing settings and home at 150% scaling. Confirm top-right home actions,
   the two home lists and the conversation action strip remain accessible.
6. On disposable data star/unstar, refresh and relaunch; verify persisted order
   without navigation. Switch databases during an outstanding star response;
   the old response must not overwrite the new database view.

If an older installed Wisp is running, it may intercept the helper launch without
providing the native settings broker. Finish work and exit that older desktop
before retrying, or use a desktop build containing #1281. The preview does not
automatically terminate another desktop process.

## Native conversations

SwiftUI now connects HTTP-model conversations to the desktop runtime for sending,
live snapshots, stopping and one-shot approvals. See [native-conversations.md](native-conversations.md)
for scope, recovery guarantees and the equivalent WinUI 3 client contract.

The composer **对话附件** button copies one local file into that project's
`uploads/` directory through `native_conversation_attach`. The command requires
the open project id and session id. It uses the same upload-name rules as
`upload_file` and binds the copy with `bind_new_message_resources`. It does not
send a message and does not change the WebView's active project or session.
Send then includes those project-relative paths in the `attachments` list and
in the same `Uploaded files:` text the WebView persists. A reloaded snapshot
shows those names on the saved user message. A lost attach or send is not
retried. Removing a chip drops it from this draft. WinUI leaves its 对话附件
button disabled and decodes the same fixture.

**排队后续** is enabled while a turn is running. It parks the current
composer draft with `native_conversation_enqueue` for that project and
session. The existing queue driver sends that one draft after the current
turn releases its workflow lock, then stops. A second distinct draft is
refused, and the button does not send another turn by itself. A lost reply
is not retried. The command does not change the WebView's active project or
session. WinUI leaves its 排队后续 button disabled.
