# WinUI user-test fixes (2026-10-02)

This change addresses the connection and navigation failures found while browsing
the compiled Windows preview with Computer Use on a real Windows desktop.

## Behavior and causes

- The bundled service can start from a Windows checkout. The old absolute
  `frontendDist` value was parsed as a `c:` URL by Tauri; joining `native-host.html`
  then panicked with `RelativeUrlWithCannotBeABaseBase`. The build now reads a
  checked-in relative asset-directory override, covered by a Rust parsing test.
- The conversation composer can be constructed. Its `server` icon was absent
  from the exported resources. The design export check now verifies literal
  WinUI icon references. Conversation event subscriptions are installed only
  after construction succeeds, preventing a failed page from observing resets.
- Files, research journey, calendar, library and project creation show their
  destination while connecting, with failure/retry and close controls. Closing a
  destination discards its late response. Reopening a project restores loading
  for a previously visible panel or terminal.
- Settings share the main window's connection. The selected category appears
  immediately while connecting; users can select another category or return.
  The initial category is General, navigation spacing is reduced, and the scope
  selector continues to distinguish global settings from a selected project.
- A short home window has a smaller brand area. Session titles omit appended
  attachment/skill metadata and shorten absolute paths; extensionless or
  truncated paths retain directory context. Original titles remain in storage
  and tooltips. Paths with spaces retain their filename.
- Offline final `attempt_completion` answers render as Markdown. An adjacent
  exact completion echo is shown once; ordinary prose, different results and
  turn boundaries are retained. Consecutive assistant tool calls share one
  disclosure without nested tool expanders. Tool results remain separate because
  the persisted schema does not identify success reliably.
- Connection cancellation/timeouts have Chinese recovery guidance. Errors use
  a warning glyph and neutral text instead of the green brand treatment. Search
  labels describe their scope, and the tutorial tooltip says it opens a browser.

The generated English labels and settings command fixture were refreshed from
the existing sources because the native build checks found baseline drift.

## Automated checks

Regression coverage includes the real Tauri override type, crashed-host exit
diagnostics, title formatting, completion extraction/deduplication and transcript
group boundaries. The native design check rejects missing directly referenced
icons before packaging. Run:

```powershell
python scripts/sync_native_design.py --check
python scripts/sync_native_settings_contract.py --check
dotnet run --project apps/windows/Wisp.ProjectBrowser.ContractTests -- contracts/project-browser/v1/projects.json
cargo test -p wisp-tauri windows_host_bundle_uses_an_asset_directory_not_a_drive_url --lib
```

## Real-window acceptance

Observed on the compiled preview at approximately 800 by 570 pixels:

- Home shows shortened titles and several project/recent-session cards.
- General and Appearance settings load; switching categories and Escape return
  to the workspace work without saving settings.
- Calendar loads the month and recorded days; library loads saved items; the new
  project form opens. No project was created and no collection was deleted.
- An existing project loads history and an enabled composer. File navigation
  lists real directories. Research journey loads dated entries. Escape closes
  the panel/page and retains the conversation.
- Restarting the preview with Files previously open, then reopening the project,
  automatically reloads the directory list and restores the conversation.

No message was sent, remote computation started, or research file changed during
this smoke. Host-crash and timeout behavior is covered by diagnostic reproduction
and synthetic tests; real network-loss recovery and macOS rendering were not
exercised by this Windows smoke. The final PR records complete suite outcomes,
including any sandbox or Windows temporary-path limitations separately.

### Validation results

Native bundle build, final WinUI publish, C# contract suite, asset/contract drift
checks, `cargo fmt --all -- --check`, the focused Tauri override test and the
WASM frontend check passed. The icon export guard also rejected a deliberately
omitted `server` entry in an in-memory negative check.

`cargo test --workspace` stopped at the CLI local-Run test because sandboxed
execution could not create its user-profile run directory; that test passed
outside the sandbox. Continuing the remaining workspace with `--no-fail-fast`
completed with 18 store and 17 Tauri failures. All failing groups subsequently
passed with normal keyring access and/or a full (non-8.3) temporary directory:
store session artifacts 11/11, store MCP secrets 10/10, store keyring 1/1, Tauri
models 50/50, Tauri MCP secrets 3/3 and command hooks 7/7. This is not a claim that
the original full-suite command was green.

Playwright completed with 991 passed, 2 skipped and 1 failed. The failure repeats
in isolation: the font-scale guard rejects existing fixed pixel font sizes at
`ui/src/styles/assistant-workspace.css:114`, `:115` and `:121`. Those declarations
were verified in the base commit and are outside this WinUI change.

## Workspace visual polish (2026-10-02 follow-up)

The conversation workspace uses quieter sidebar actions, a single-line session
section header, a clearer selected-session card and one rounded composer surface.
These changes retain the configured typography and palette.

The workspace panel now has a view-selector menu below its title instead of a
wide strip of tabs. Files uses a search inset, a compact create menu, a parent
folder action, a truncated path with a tooltip and an item count. Folders sort
before files; names sort within each group. File sizes use readable binary units.
Rename and Delete remain available in each row's accessible overflow menu.
Empty folders and searches without matches have distinct messages.

The narrow-window drawer has a lighter, stable backdrop. Background controls
remain disabled while it is open. Panel menus and file dialogs participate in
the window Escape stack before the panel itself. The initial panel title and
loading state render before its data request completes.

Contract tests cover sorting, trimmed case-insensitive filtering, source-list
immutability and size boundaries. Manual acceptance for this follow-up checks
the compiled WinUI window, immediate Escape on a newly opened menu/dialog,
folder navigation, filtering, narrow/wide layouts and the composer. No model
message or destructive file operation is needed for this visual smoke.

Observed in the compiled follow-up: 800-pixel-wide and maximized workspaces,
directory entry/parent navigation, case-insensitive `readme` search, no-match
state, readable file sizes, and immediate Escape closing only the new-file menu,
row menu or new-file dialog. The parent Files panel stayed open. No files were
created, renamed or deleted.

The theme smoke found retained panels and the composer keeping their original
brushes after a theme change. Design brushes and shared SVG sources now update
in place. A dark-to-light switch retained an unsent test draft and updated the
composer; the retained Files panel was also checked after reopening. The test
draft was cleared and the original light appearance restored. This is manual
WinUI coverage; the C# model suite does not render WinUI controls.

Follow-up checks: WinUI Release publish, the C# contract suite, native asset
checks, Rust formatting and the WASM check passed. The full Rust run reached
Tauri with 1052 passing and 9 failing tests. Its temporary directory was inside
the checkout and excessively long for Git/Windows path checks (`$GIT_DIR too
big`, non-Git detection and artifact/candidate path containment). Rechecking
the affected groups with `%LOCALAPPDATA%/Temp/wisp-ui` passed: delegation
isolation 8/8, exploration promotion 1/1 and method search 6/6. The original
full-suite command still exited with failure; this records the targeted
environment correction rather than claiming a green full rerun.

The follow-up Playwright run completed with **992 passed, 2 skipped, 0 failed**
(26.4 minutes). The earlier font-scale failure above is historical and did not
recur on this base.

## All-page visual audit (2026-10-03)

This pass inspected the compiled WinUI application using the computer-use
plugin. It extends the workspace pass above to the shared page shell and
settings, rather than validating only the Files drawer. The final local build
is `target/native-windows-polished/Wisp.Science.Preview.exe`.

Changes include consistent headings, padded cards, quiet header actions,
wrapping action rows, informative empty states and compact calendar markers.
Settings keeps Back, project scope and search pinned while categories scroll.
Model presets and connection tool lists use expandable sections; filtering a
model or skill hides its entire card. Appearance draft colors are isolated
inside the preview card. New-project instructions are optional disclosures,
library entries omit empty code boxes, and capability counts use compact cards.

Notebook cells are grouped, artifact paths are truncated with tooltips, and
provenance input/output details expand per tool. Sidechat uses one model picker
instead of a column of model buttons, and updates Send when its draft changes.
The terminal height is bounded against the window height, including resizing
within a layout breakpoint, so a short window retains the composer controls.

### Observed coverage

- All 20 settings categories: General, Network, Conversation, Appearance, Pet,
  Models, Quick Actions, Workflows, Experts, Memory, Skills, Plugins, Browser,
  Connections, Remote Access, Credentials, Permissions, Environment, Storage
  and Usage. Project settings and an unmodified editor were also opened.
- Home, month calendar and activity entries, journey range/results, a real
  library image entry, new/import project forms, conversation, capabilities
  and the publication creation surface.
- All eight workspace panels: Files, Artifacts, Workflows, Notebook,
  Highlights, Provenance, Environment and Sidechat; plus Outline, Share
  preview, Trajectory, Inbox and the embedded terminal surface.
- Final-build rechecks at approximately 800 x 536 client DIPs: pinned settings
  navigation; collapsed/expanded connection tools; model filtering without
  leftover card frames; provider presets wrapping; dark appearance preview
  without recoloring surrounding chrome; provenance disclosures; sidechat
  dropdown and input; and terminal/composer fit.
- Immediate Escape closed only the sidechat model dropdown and left the
  parent drawer open. Typing a temporary sidechat draft enabled Send; clearing
  it disabled Send. Nothing was sent. The journey date popup also retained
  its parent page after immediate Escape in the preceding audit build.
- Dark workspace colors were checked. The appearance draft was cancelled,
  the temporary sidechat draft cleared, and the original light theme restored.

Storage eventually rendered after about one minute of scanning. This was a
slow loaded state, not a verified timeout. Live credentials and permissions
were viewed only. No auth flow, installation, deletion, remote job, terminal
command, project creation/import, publication creation or export was performed.

Archive was reviewed in code only: opening an empty archive invokes preparation,
which can call a model and persist an archive. It was not activated for a visual
smoke. Populated/error states not encountered during navigation are not claimed
as manually verified, and every category was not independently tested in dark
mode.

### Validation and remaining findings

WinUI Release publish and the complete native C# contract executable passed.
New checks cover wrapping order/bounds, terminal height budgeting and sidechat
model/draft state without dispatch. Manual WinUI checks above exercise the
actual controls; the contract suite alone does not render them. Native design
asset synchronization and `git diff --check` also passed. Build and contract
logs are in `test-results/winui-pages-polished-{build,contracts}.log`.
The Rust/WASM/Playwright results in the preceding section belong to the earlier
workspace pass; they were not rerun for this native-only continuation.

Two observations remain outside the completed layout fixes: the footer theme
menu writes local preview settings, while opening Settings reapplies the
service's saved appearance, so entering Settings can restore a different theme;
and Capabilities displays the bootstrap service workspace path, which may differ
from the selected project's path. These require explicit follow-up and are not
counted as successful cross-page consistency checks.

## Conversation screenshot correction (2026-10-03)

The user's follow-up screenshot exposed remaining density problems in the
800-DIP workspace: a clipped navigation region, too few visible sessions,
a second toolbar row and a Send button forced below the composer controls.
The header now uses actual conversation width: narrow panes keep Search,
Panel and More beside the title; More retains outline, share, trajectory,
archive, inbox and terminal. Wide panes show those actions directly.

Research journey, publication and library are grouped in Research Tools.
New Folder is in the session grouping/sort menu. The sidebar no longer clips
its navigation region with a fixed-height scroll viewport. Session rows and
the empty composer use less padding. At the screenshot's width, Send stays
beside the model/mode controls; very narrow action areas may still wrap.

Release publish and the native contract executable passed, including added
content-width and composer breakpoint checks. Asset synchronization and diff
whitespace checks passed. Actual WinUI checks covered the original session,
approximately 800 x 536 client DIPs, maximize/restore, an unsent draft enabling
Send without moving it below the controls, Research Tools, the new folder menu
entry, and immediate Escape on menus. The draft was cleared without sending.
The local build is `target/native-windows-conversation-fit/Wisp.Science.Preview.exe`;
it includes the existing query service and full settings-host resources.
Logs are `test-results/winui-conversation-fit-{build,contracts}.log`.

An initial launch without settings-host resources showed read-only history.
After copying the existing packaged host, connecting and restarting, both the
composer and session-management controls loaded. This records a local output
packaging correction; it does not claim an offline-reconnect code fix.

## Composer reference alignment (2026-10-03)

The WinUI composer follows the supplied WebView reference: execution environment
sits above the input card, the shortcut hint is centered inside it, attachment
and conversation options sit at the lower left, and model, reasoning effort,
Fast and Send align at the lower right. Circular actions use 32-DIP bounds.
Plan mode remains available in Conversation Options; the trailing controls
wrap together below 480 DIPs. Send uses a shared arrow icon with reduced opacity
when disabled. The native Ctrl+Enter shortcut remains unchanged, and no runtime
status or context percentage is fabricated to match the reference.

The final build is `target/native-windows-composer-aligned/Wisp.Science.Preview.exe`.
Release publish, all native C# contracts (including the 479/480-DIP boundary),
native asset synchronization and diff whitespace checks passed. Logs are
`test-results/winui-composer-aligned-{build,contracts}.log`. Arrow and lightning
SVGs are exported from the shared icon source into the shared native resources.

Live checks confirmed the aligned empty composer at approximately 800 x 536
client DIPs and its disabled Send appearance. Immediate Escape closed the
options flyout and retained the composer before the final opacity adjustment.
The user stopped computer-use before final-build draft and maximize/restore
checks; those remain unverified on this build. Earlier draft/resize results
above apply to their recorded builds, not this final alignment pass.

## Conversation readability and table artifacts (2026-10-05)

The WinUI/WebView audit used the same existing SCOTCH monitoring conversation.
The native preview had no table artifacts while the WebView showed six, and
review-needed Run output expanded raw JSON between separate process groups.
The high-priority correction is available in
`target/native-windows-high-priority-final/Wisp.Science.Preview.exe`, with the
query service and full settings-host resources copied beside it.

Computer Use on the final build verified a successful launch and these flows:

- The latest completed turn has one **显示已完成过程 · 7 条** entry with its
  final Markdown answer outside. **查看待审阅运行** opens the matching Run detail
  in the environment panel. Expanding the process keeps terminal Run cards
  collapsed; raw data sits behind **原始输入与输出**. An expanded Run disclosure
  remains expanded after collapsing and reopening its enclosing process.
- Artifacts contains six tables with dimensions, matching the audited WebView.
  Opening the first table renders a native preview. **复制表格** was checked by
  pasting its header and rows into an empty composer; the test draft was then
  cleared without sending. Escape closes the preview first and the narrow
  artifacts drawer second, without needing to focus inside the preview.
- The approximately 801 x 567 window and maximized window keep compact context
  usage directly beside the model picker. ACP and context no longer occupy
  separate composer rows. The earlier candidate's options check opened the ACP
  submenu; immediate Escape closed only that submenu, then the parent options.
  The final build uses the same options implementation.

No messages, Run cancellation/cleanup actions or provider-setting changes were
submitted during these checks. Table collection covers the displayed transcript
page, including successful completion output; it does not scan unloaded history.

The final native publish, complete C# contract executable, Rust formatting and
WASM UI check passed. Contract coverage includes conservative process grouping,
completion duplication, exact Run ownership and external review actions, table
projection/cache/selection/TSV, and compact context labels. Logs use the prefix
`test-results/winui-high-priority-`.

`npm ci --offline` and the complete Playwright run passed: 1,014 passed and two
skipped. The runner waited during web-server teardown after the last test;
stopping only its verified Trunk process on port 15425 let it report exit 0.
The initial sandboxed Rust run failed the local Run wait test because its
user-profile Run directory was not writable. That exact test passed outside
the sandbox. The unrestricted `--no-fail-fast` workspace run completed with
2,548 passed, nine failed and one ignored; its only failed target was
`wisp-store --lib`. The desktop host's 1,107 tests all passed. All nine failures
were in `wisp-store::session_artifacts` with the default Windows temporary path;
all 11 tests in that module passed with `TEMP`/`TMP` set to a full, non-8.3
temporary path outside the checkout. These are separate results, not a green
full-suite claim.
