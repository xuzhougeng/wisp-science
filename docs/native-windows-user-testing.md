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
