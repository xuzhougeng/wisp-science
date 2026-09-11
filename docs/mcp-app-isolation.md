# Windows MCP App renderer isolation

## Scope and user-visible behavior

On Windows, an MCP App still occupies the center split pane, but its document
no longer lives inside the primary chat document. A native child WebView hosts
a small trusted shell and an opaque-origin `sandbox="allow-scripts"` iframe.
The child uses a new WebView2 data directory for each mount; it never receives
the primary window's profile. macOS/Linux retain the legacy iframe backend.

The host toolbar, Reload App button, tab close controls, sidebar, composer and
Stop stay in the primary document. Creation errors are visible and retryable;
there is **no Windows production fallback to a primary-document iframe**.

This work sits on current `main` (including #1188). Host chat images, videos
and thumbnails keep the media owner / `revokeObjectURL` stack; isolation does
not replace it. #1188 still excludes MCP iframe images from host Blob
diagnostics. On Windows those App images now live in the child WebView
instead of the primary document.

Leaving a tab or session invalidates the child's identity immediately, then
destroys its native view. Reopening creates a new generation and sends the
saved inputs/results, without automatically replaying a tool call. This is not
full UI-state restoration: unsubmitted selections, page positions and edits
may be lost. The UI displays this warning. Already dispatched server-side
effects cannot be undone by closing the view, and the MCP connection or remote
Run is not terminated. A disconnected original server is not advertised as
`serverTools`; the restored viewer displays an unavailable-connection notice.

## Native compatibility and authority

Adding a child makes Tauri's single-view `WebviewWindow` lookup/extractor stop
matching the owner window. `WorkspaceSurface` therefore holds a `Window` and
its primary `Webview` explicitly. Workspace command injection accepts only a
real primary document (its label equals its native window label), never a
`mcp-app-child-*` caller. Reload/eval operate on that primary document; native
size/menu/tray operations operate on the native window. `WorkspaceManager`
enumerates primary surfaces rather than assuming one WebView per window.

Tauri remains locked to the existing dependency versions; only its `unstable`
feature is enabled for `Window::add_child()`. The migrated references across
command modules are deliberate, not a rename of the native window object.

All capabilities now target **webview labels**, not native-window labels.
The child capability grants **no core/plugin permissions** (`permissions: []`).
Application commands are registered globally, so the invoke handler is the
fail-closed second layer: it denies every child command except bootstrap,
ready, protocol request and action reply. A child cannot use workspace
settings, filesystem, dialog, event subscriptions, window controls or
another session's bridge.

The Rust registry binds owner label, owner document epoch, mount serial,
child label, instance/session identity and bridge generation. Serial
tombstones fence close-before-create races. Old creation replies never choose
the active tab; old close operations cannot revoke a newer bridge generation.
Logical references survive suspension and are counted across windows. Final
user close releases only the corresponding logical binding/context/bridge.

No registry lock is held while constructing or destroying native WebViews.
`add_child` and native hide/close share `native_ops` so a retired label cannot
be created after close. Construction uses `spawn_blocking` and does not hold
`native_ops` while waiting for the shell to become ready. Close invalidates
first, hides the view, sends optional resource teardown, and closes natively
after at most a 500 ms grace period without waiting for guest JavaScript.
Generated profile cleanup retries independently when WebView2 still holds
files; it only targets generated leaf profiles under this feature's per-boot
directory. A process exit during cleanup can leave a disposable profile
directory; do not erase user profiles to fix it.

## Protocol and layout

```
guest iframe -> child shell -> restricted Rust request -> original MCP server
```

Tool traffic does not pass through primary-page JavaScript. The shared Rust
tool service retains same-server visibility, schema checking, approval and
Plan Mode gates, 3 MiB arguments, 4 MiB results, 30 s execution timeout, four
concurrent calls and twenty calls per ten seconds. Identity is rechecked
after asynchronous approval and after tool completion. Motif host actions
carry the current handle and a bounded request/response id; a stale reply
cannot reach a replacement view. Large payloads are targeted, not broadcast.

The child shell and legacy host share CSP/Motif document helpers. Guest
messages require the iframe's exact `event.source` and opaque `null` origin.
The shell permits only its own local navigation; new windows/downloads are
denied. It does not grant `allow-same-origin`, filesystem or connection secrets.

The primary page measures a dedicated content rectangle beneath host controls.
CSS client coordinates plus the actual primary-WebView physical size determine
bounds (including DPI/page zoom). Child `set_bounds` is relative to the parent
window client area; the primary document fills that area, so the origin is
(0, 0). The primary webview's `position()` is not added, because a
webview-window reports a desktop-relative inner position. Resize and mutation
observation, scroll/resize events and ancestor-animation tracking coalesce
updates by frame.
Only bounds/context are resent, never the HTML. Host interactions hide the
native view before opening host overlays; semantic overlays and hit-testing
keep it hidden until the active view is unobscured. Clicking native selects
(including the chat approval scope or its label) skips this temporary
hide/show cycle so WebView2 does not dismiss the option picker. DOM menus and
dialogs retain the same occlusion handling. Native resize/minimize
hides stale geometry. Guest Escape explicitly returns focus to the primary
document and dispatches to its existing window Escape stack.

## Validation and release gates

Automated entry points (PowerShell, from the worktree):

```powershell
$env:WISP_CATALOG_OFFLINE = '1'
cargo fmt --all -- --check
cargo check -p wisp-tauri --offline
cargo test --workspace --offline
cargo run -p wisp-mcp --example smoke --offline
cargo run -p wisp-tauri --example webview_recovery_smoke --offline
$env:WISP_ISOLATION_CYCLES = '100'
cargo run -p wisp-tauri --example mcp_app_isolation_smoke --offline
Push-Location ui
cargo check --target wasm32-unknown-unknown --offline
Pop-Location
Push-Location ui-tests
npx --no-install playwright test
Pop-Location
```

The native isolation probe uses hidden disposable windows, fake sessions,
the production native registry/create/close implementation and production
child shell. Its profiles are under `target/mcp-isolation-smoke/`. It records
browser/renderer process trees, runs a bounded 20-second guest block, exercises
primary DOM control handlers during the block, rejects a child workspace IPC,
tests scoped native Stop, closes a blocked child and cycles mount/destruction.
It waits for profile cleanup and asserts no orphan controllers. It does not
instantiate the normal database, normal Wisp plugins or real remote tasks.

Browser tests explicitly distinguish a native-backend mock (no primary
iframe) from legacy iframe regression. They cover mount/suspend/rebuild,
close-before-create, fail-closed creation/retry, geometry, host occlusion,
Escape, Motif host selection and direct shell protocol. Existing MCP/Motif
business tests remain important; native mock success is not real WebView2
acceptance. `WISP_MOTIF_APP_HTML` and `WISP_SNAPGENE_FIXTURE` are optional local
fixture inputs for the pre-existing real-Motif tests; absence is a skip, not
a pass.

Before release, **also** verify visibly in disposable Windows environments:

- Real SFL paging/details/selection/tools, Motif local/project imports and
  selection-to-composer, real approval and Plan Mode transitions.
- Actual primary mouse/keyboard interactions and heartbeat within 1 s during
  guest blockage, with no primary reload. The hidden smoke's programmatic DOM
  events alone do not cover all physical focus/hit-test behavior.
- 100%, 150%, 200% DPI; maximization, cross-monitor movement, zoom, split-pane
  dragging, host menus/modals over the guest, Escape and focus return.
- Long-session / large-result / dual-stream replay without Apps, then with SFL
  paging. Capture performance traces if primary rendering still stalls.
- macOS/Linux build and legacy behavior (Windows checks cannot substitute).

Do not close the original freeze issue solely because isolation code compiles
or a blocked view can be reloaded. This change removes one propagation path;
it does not establish the initial JavaScript/layout call stack that froze the
user's original chat. No claim is made that every long-chat bottleneck is fixed.

## Separate Windows baseline regression fixes

The full-workspace run also exposed existing Windows path/env mismatches:
`snapshot_store` relative/canonical containment, a case-sensitive `.R`
expectation in a resource-lease test, and `Command` env lookups that assumed
distinct `https_proxy` / `HTTPS_PROXY` keys. These small Windows
compatibility fixes are separate from renderer isolation.
The capability regression assertion was updated from native-window matching
to primary-WebView matching as part of the isolation change.
