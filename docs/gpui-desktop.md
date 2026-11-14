# GPUI desktop migration

The desktop direction is one Rust/GPUI frontend for macOS, Windows and Linux.
The SwiftUI/WinUI desktops and their dedicated packaging, release, design-sync
and parity-ledger jobs are retired. iOS and Android remain. Shared `native_*`
Rust services, DTOs and fixtures remain because mobile, Tauri and server hosts
still consume them.

`apps/gpui` implements the first slices, not full replacement parity. The
[parity ledger](gpui-parity.md) lists every WebView feature, what GPUI has, and
the order the rest is ported in. WebView
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

No configuration is needed to launch:

```sh
cargo run gpui
```

On first launch Settings opens by itself on Models. Add a model: choose the
provider (`openai`, `openai_responses` or `anthropic`), the endpoint, the model
ID and the API key, or start from one of the provider presets and add the key.
"Test connection" sends one short request with the form as typed and saves
nothing. The host validates the form with the same rules as the server: limits
come from the checked-in exact-ID catalog, and a model outside it needs an
explicit context window and output limit. A rejected form changes nothing.
Saving takes effect for the next message without a restart; a turn already
running keeps its model.

Any number of models can be saved. One is the default: new conversations, and
conversations that chose nothing, run on it. The model button in the composer
picks a different one for that conversation. Removing a model sends the
conversations that used it back to the default. The single model saved by an
earlier build of this preview shows up as a profile with its key intact.

The non-secret fields are saved in the preview database. Each model's API key
goes to the existing Wisp keyring path, per data directory, and is never
written to SQLite or shown again. As elsewhere in Wisp, debug builds keep
secrets in a plaintext development file in the home directory instead of the OS
keyring; release builds use the keyring. If the key is missing at launch, the
window says so and asks for it again.

Environment variables still work and take precedence, for scripted or
development launches. While `WISP_MODEL` is set, Settings shows that model
read-only:

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

`WISP_API_URL`, `WISP_REASONING_EFFORT`, `WISP_MAX_CONTEXT` and
`WISP_MAX_TOKENS` follow the server's existing semantics. `WISP_SKILLS_PATH`
and `WISP_MAX_ITER` are read from the environment only. No remote-access URL or
pairing code is needed.

The preview database lives in `wisp-science-gpui` under the OS local-data directory.
`WISP_GPUI_DATA` overrides it. The default workspace is its sibling
`wisp-science-gpui-workspace`; `WISP_GPUI_WORKSPACE` overrides that root.
**Do not point the preview at a production desktop/server database.** Opening a
folder registers its research root; it does not import existing desktop chats.

## Implemented slice

| Surface | Behavior |
| --- | --- |
| Home | Opens first: every project and the newest conversations across them, with status |
| Projects | Persisted list, native folder picker, reopen registered roots, running and waiting counts, star to keep first, remove from Wisp (files stay) |
| Look | Wisp's light and dark palettes following the system; the shared icon set |
| Conversations | Create, select, restore messages, running / needs-you status |
| Conversation management | Rename, pin (shown first), archive and restore, delete with confirmation |
| Search | Titles and message text across the project, archived conversations included |
| Command palette | One box over everything: projects, files by name in every project, conversations by title or message text, and the window's commands; arrow keys, Return, Escape |
| Settings | Saved models with provider, endpoint, key and limits; presets, a default, a connection test; applied without restart |
| Inbox | Conversations in any project that are waiting for you; opens across projects |
| Read state | A finished answer is marked read once it is on screen in the active window |
| Transcript | Virtualized Markdown, selectable text, paginated history |
| Tool calls | One folded line each with outcome and elapsed time; click for the verbatim input and output; interrupted calls stop claiming to run |
| Runs | Runs started by the conversation: kind, status, exit code, command and output tails on demand |
| Files | Project file browser beside the transcript, available before the first conversation; read-only text and Markdown preview |
| Composer | Native multiline input; Enter for newline, Cmd/Ctrl+Enter to send |
| Conversation options | Ask first or Full access, the model, and reasoning effort, per conversation; both are kept across restarts and apply from the next message |
| Usage | How full the context is, and the tokens the running or latest turn used |
| Questions | When the model asks, a card with its options: one click answers, or type the answer |
| Messages | Copy any message as Markdown |
| Attachments | Attach files with the system dialog, or add a project file, another conversation or a skill from the add-context box; shown as chips until sent |
| Drafts | Survive navigation; preserved on send failure, cleared on durable acceptance |
| Execution | Shared agent/tools, streamed snapshots while running, stop and errors |
| Approvals | Exact pending request; allow once, allow that tool for the rest of the conversation, or deny |
| Escape | One window-level stack: Escape closes only the topmost of command palette, file preview, Files panel, Settings, title editor, delete confirmation, remove-project confirmation, the Full access question, the model and effort menus and the remove-model question |
| Shortcuts | Cmd/Ctrl+K opens the command palette; Cmd/Ctrl+N starts a conversation in the open project; Cmd/Ctrl+, opens Settings |
| Lifecycle | Background IO, stale-result rejection, cancel/clean workers on exit |

Lists and search results show up to 100 conversations per project. Drafts do
not yet survive app restarts. Messages are
text-only. The initial project keeps the embedded host's `Server` name; opened
folders use their own names.

Runs, status and the inbox use commands the embedded host already serves.
Conversation management, Settings, the home page, search and project file
access are host operations in `crates/wisp-server/src/desktop.rs`. They are
typed methods the desktop calls in-process, not dispatch commands, so the
remote allowlist is unchanged and a paired mobile client cannot rename,
delete, reconfigure or read a project's files without a conversation.

Conversation search no longer matches the instructions every conversation
starts with. That clause lives in the shared store
(`search_sessions_with_options`), so the WebView's search changes the same
way: a word that occurs only in the system prompt used to match every
conversation and now matches none.

Limits of this slice:

- Models are API models only. ACP agents, subscription accounts, the decision
  model and image or video models are not ported. A broken model chosen for a
  conversation is reported when a message is sent, not replaced silently.
- Deleting removes the conversation and its messages; files it wrote stay in the
  project. A running conversation must be stopped before it can be archived or
  deleted. There are no folders, export, import, share or branch views yet.

- Files shows the local project root only, one directory at a time (at most
  2000 entries), hides `.wisp`, `.git`, links and special files, and previews
  the first 1 MB of a text file. Markdown up to 256 KB is rendered; larger or
  other text is shown verbatim. No edit, upload, download, image, PDF or
  Office preview.
- The palette finds files by name, not content. Each project is walked
  breadth-first for at most 20,000 entries, skipping links, dot-folders,
  `node_modules` and `__pycache__`, so a file deep in a very large tree can be
  missed. Results cannot yet be opened in a new window or attached to a
  message.
- Full access stops the questions before tools that change files or run
  commands. It does not cover the separate question a dangerous shell command
  asks; that one always appears. "Allow for this conversation" is offered only
  for the standard tool question, is kept in memory, and is forgotten when the
  app restarts; "Ask again" under the transcript revokes it sooner.
- Options changed while a turn runs apply to the next one. Token usage is not
  stored: after a restart it is empty until the next turn. Plan first, thinking
  blocks and fast mode are not ported.
- A file attached from outside the project is copied into the project's
  `uploads` folder (up to 100 MB, never overwriting); a file inside it is used
  where it is. The model receives project-relative paths and reads the files
  with its tools; image pixels are not sent. A referenced conversation is
  passed as its most recent exchange (about 12,000 bytes), and a skill as its
  instructions, for that turn only: neither is saved with the message. Paste,
  drag and drop, and typing `@`, `#` or `/` in the composer are not ported.
- Runs are read-only. Cancel, harvest, review and the full log need host
  commands this build does not serve.
- Status refreshes every two seconds while any turn is running and once more
  when the last one stops; it is not pushed by the host.
- The interface is English only; localization is still open (below).
- Removing a project deletes its conversations from the preview database and
  leaves the folder alone. The workspace the app starts on cannot be removed,
  and a project with a running turn must be stopped first.

Icons are the WebView's one shared set, copied one file per kind into
`apps/gpui/assets/icons` and embedded in the binary. A test compares every file
with `compose_icon` in `ui/src/app_support/messages.rs` while that source
exists, so the two frontends cannot drift; add an icon there first. The palette
in `apps/gpui/assets/wisp-theme.json` uses the WebView's colour tokens, and a
test rejects keys the component library would silently ignore.

Escape is intercepted for the window before key dispatch, so it does not depend
on which element has focus. `apps/gpui/src/layers.rs` owns the order; a new
dismissible surface must be added there rather than handling the key itself.

## Remaining replacement work

1. Move additional Tauri-bound use cases into shared Rust services, reusing DTOs.
2. Safely import existing desktop data/settings; add the remaining settings
   (model profiles and per-conversation selection, network, hooks, MCP,
   skills), account login and ACP.
3. Port artifacts, contexts, run control, literature, notebook, search, attachments,
   terminal/runtime panels and scientific previews in bounded changes. Extend
   Files with project-level access, search, transfers and rich previews.
4. Finish localization, accessibility, platform menus,
   window restoration and input/scroll acceptance on all three platforms.
   Every new dismissible layer must participate in one Escape stack.
5. Add installers, signing, updates and recovery. Verify feature/data parity
   before making GPUI default or removing `ui/` and `src-tauri/`.

## Verification

Local checks/tests require the permission specified in `AGENTS.md`:

```sh
cargo check --locked --manifest-path apps/gpui/Cargo.toml --all-targets
cargo test --locked --manifest-path apps/gpui/Cargo.toml
cargo test --locked -p wisp-dev
cargo test --locked -p wisp-server
python3 -m unittest discover -s scripts -p test_ci_scope.py
node --test docs/test/downloads.test.mjs docs/test/worker.test.mjs
```

Tests use synthetic state and temporary projects; no model keys or remote hosts.
CI checks GPUI on macOS/Windows/Linux and keeps the iOS workflow independent.
Compiler checks do not establish visual or IME parity.

Manual acceptance after approval: on the home page star a project and confirm
it moves first; ask to remove one, press Escape, then remove it and confirm its
folder is untouched; open a recent conversation from another project. Use a
disposable project and two chats; type
Chinese, emoji and multiline text; switch rapidly while loading; check draft and
response ownership; send, approve/deny, stop, load history and copy Markdown.
Open the palette from the search button and with Cmd/Ctrl+K; with nothing
typed it lists projects, recent conversations and commands. Type part of a
file name from another project, move with the arrow keys and press Return: the
project opens with the file previewed; press Escape and its folder is listed.
Ask something that makes the model ask back: answer by clicking an option, and
in another conversation by typing. Let a tool ask for approval and allow it for
the conversation; have it run again and confirm no question appears; press Ask
again and confirm the next run asks. Click Ask first, press Escape, then turn
Full access on, restart, and confirm it and the chosen effort are still set.
Open the effort menu and press Escape. Copy a message and paste it elsewhere.
In Settings add two models, test the connection of one, make the second the
default, choose the first for one conversation and confirm the header names it;
ask to remove a model, press Escape (Settings stays), then remove it and confirm
the conversation falls back to the default.
Attach a file from outside the project and one inside it; add another
conversation and a skill from the add-context box; remove a chip; send, and
confirm the file chips stay on the message and `uploads/` holds the copy. Send
files with no text.
Type a word that occurs only in a message, and one that matches nothing. Press
Escape with the palette over Settings and over the Files panel: only the
palette closes. Click outside it: it closes.
Open Files, enter folders, go up, preview Markdown, a large CSV and a binary file;
switch folders and projects rapidly and confirm no other folder's rows appear.
With the preview open press Escape once: only the preview closes. Press it again:
the panel closes. Repeat right after opening, without clicking inside, and with
focus in the composer; cancelling an IME composition must not close a layer.
Run a tool that fails and one that succeeds and check status and elapsed time.
Start a turn, switch to another conversation and confirm its status and the
inbox update without pressing Refresh; open it and confirm it becomes read.
Start with an empty data directory and no `WISP_*` variables: Settings opens;
save an invalid form, then a valid one, and send a message without restarting.
Relaunch and confirm the model is still configured. Rename with Enter and with
the button; press Escape in the title editor and in the delete confirmation and
confirm only that layer closes. Pin, archive, find the archived conversation by
a word from one of its messages, restore it, then delete it.
Restart and confirm persisted messages. Close during a running tool and verify
worker cleanup. Check missing credentials and invalid workspace errors. Validate
each target OS; a macOS build does not establish Windows/Linux behavior.
