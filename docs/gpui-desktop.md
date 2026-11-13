# GPUI desktop migration

The desktop direction is one Rust/GPUI frontend for macOS, Windows and Linux.
The SwiftUI/WinUI desktops and their dedicated packaging, release, design-sync
and parity-ledger jobs are retired. iOS and Android remain. Shared `native_*`
Rust services, DTOs and fixtures remain because mobile, Tauri and server hosts
still consume them.

`apps/gpui` implements the first slice, not full replacement parity. WebView
installers remain the release channel until the migration is accepted. No new
SwiftUI/WinUI installers are built or offered in the download picker. Existing
immutable download links continue to work.

## Lessons from quiet-md

Reviewed [quiet-md](https://github.com/s1dashu/quiet-md) at
[`7ead2356`](https://github.com/s1dashu/quiet-md/tree/7ead2356ef2c82609bdae9ad084b26dbd877e65d),
including `Cargo.toml`, `src/main.rs`, `src/app.rs`,
[`docs/ARCHITECTURE.md`](https://github.com/s1dashu/quiet-md/blob/7ead2356ef2c82609bdae9ad084b26dbd877e65d/docs/ARCHITECTURE.md)
and `vendor/PATCHES.md`.

- Keep business logic independent of GPUI. Wisp reuses its Rust host, store,
  agent, tools and DTOs. The host runs in-process with no listener or hidden WebView.
- Own view state in GPUI entities; run storage/model/tool IO on Tokio. A persistent
  `ListState` virtualizes transcript rows and preserves unchanged measurements.
- Treat text input as an editor, including IME, selection and clipboard.
  `gpui-component` supplies native multiline input and Markdown rendering.
- Keep platform behavior explicit: native file pickers, windows and graphics;
  Cmd shortcuts on macOS and Ctrl shortcuts on Windows/Linux.

Dependencies are pinned to GPUI 0.2.2 and gpui-component 0.5.1 with its optional
`webview` feature disabled. No quiet-md source, fonts, icons or vendor patches
were copied. Its Nucleo icons and branding are outside its MIT license. Its
font/layout/platform patches are references for future targeted fixes.

## Build and launch

The renderer uses an independent Cargo workspace and lockfile, like `ui/`, so
its graphics dependencies do not change headless builds. Use current stable
Rust. macOS needs Xcode Command Line Tools; Windows needs MSVC Rust, Visual Studio
C++ Build Tools and Windows SDK. Linux dependencies are listed in
`.github/workflows/gpui.yml`; a working graphics driver is needed at runtime.
This binary requires no Trunk, Node, .NET, Swift, Tauri or WebView.
The preview enables GPUI's `runtime_shaders`: macOS compiles its Metal shaders
at launch instead of requiring Xcode's separate Metal Toolchain at build time.
Measure startup and choose precompiled shaders when preparing release packages.

```sh
cargo run build-gpui
cargo run build-gpui --release
```

Output: `apps/gpui/target/debug/wisp-gpui` (`.exe` on Windows), or `release/` for
optimized builds. Cargo target-directory overrides still apply.

Configure a provider before launching, with the API key injected into the
process environment by your existing secret-management setup:

```sh
export WISP_PROVIDER=openai
export WISP_MODEL=gpt-4o
# WISP_API_KEY must be set in the environment.
export WISP_GPUI_WORKSPACE=/absolute/path/to/research
cargo run gpui
```

PowerShell:

```powershell
$env:WISP_PROVIDER = 'openai'
$env:WISP_MODEL = 'gpt-4o'
$env:WISP_GPUI_WORKSPACE = 'C:\Research'
cargo run gpui
```

`WISP_API_URL`, `WISP_REASONING_EFFORT`, `WISP_SKILLS_PATH`, `WISP_MAX_ITER`,
`WISP_MAX_CONTEXT` and `WISP_MAX_TOKENS` follow the server's existing semantics.
Providers: `openai`, `openai_responses`, `anthropic`. Limits use the checked-in
exact-ID catalog; unknown IDs require explicit context/output limits. Keys remain
in process memory, never SQLite. No remote-access URL or pairing code is needed.
Missing configuration appears in the window; configure it and relaunch.

The preview database lives in `wisp-science-gpui` under the OS local-data directory.
`WISP_GPUI_DATA` overrides it. The default workspace is its sibling
`wisp-science-gpui-workspace`; `WISP_GPUI_WORKSPACE` overrides that root.
**Do not point the preview at a production desktop/server database.** Opening a
folder registers its research root; it does not import existing desktop chats.

## Implemented slice

| Surface | Behavior |
| --- | --- |
| Projects | Persisted list, native folder picker, reopen registered roots |
| Conversations | Create, select, filter loaded conversations, restore messages |
| Transcript | Virtualized Markdown, selectable text, tool output, paginated history |
| Composer | Native multiline input; Enter for newline, Cmd/Ctrl+Enter to send |
| Drafts | Survive navigation; preserved on send failure, cleared on durable acceptance |
| Execution | Shared agent/tools, streamed snapshots while running, stop and errors |
| Approvals | Exact pending request; allow once or deny |
| Lifecycle | Background IO, stale-result rejection, cancel/clean workers on exit |

The host lists up to 100 chats per project; filtering covers that loaded list,
not full-history search. Drafts do not yet survive app restarts. Messages are
text-only. The initial project keeps the embedded host's `Server` name; opened
folders use their own names.

## Remaining replacement work

1. Move additional Tauri-bound use cases into shared Rust services, reusing DTOs.
2. Safely import existing desktop data/settings; add settings UI, existing keyring
   integration, model selection, account login and ACP.
3. Port files/artifacts, contexts/runs, literature, notebook, search, attachments,
   terminal/runtime panels and scientific previews in bounded changes.
4. Finish localization, shared icon assets, accessibility, platform menus,
   window restoration and input/scroll acceptance on all three platforms.
   Every new dismissible layer must participate in one Escape stack.
5. Add installers, signing, updates and recovery. Verify feature/data parity
   before making GPUI default or removing `ui/` and `src-tauri/`.

## Verification

Local checks/tests require the permission specified in `AGENTS.md`:

```sh
cargo check --locked --manifest-path apps/gpui/Cargo.toml --all-targets
cargo test --locked --manifest-path apps/gpui/Cargo.toml state::tests
cargo test --locked -p wisp-dev
cargo test --locked -p wisp-server local_
python3 -m unittest discover -s scripts -p test_ci_scope.py
node --test docs/test/downloads.test.mjs docs/test/worker.test.mjs
```

Tests use synthetic state and temporary projects; no model keys or remote hosts.
CI checks GPUI on macOS/Windows/Linux and keeps the iOS workflow independent.
Compiler checks do not establish visual or IME parity.

Manual acceptance after approval: use a disposable project and two chats; type
Chinese, emoji and multiline text; switch rapidly while loading; check draft and
response ownership; send, approve/deny, stop, load history and copy Markdown.
Restart and confirm persisted messages. Close during a running tool and verify
worker cleanup. Check missing credentials and invalid workspace errors. Validate
each target OS; a macOS build does not establish Windows/Linux behavior.
