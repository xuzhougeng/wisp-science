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
| Projects | Persisted list, native folder picker, reopen registered roots, running and waiting counts, star to keep first, remove from Wisp (files stay); Project settings for the name, a description, and the rules the model follows in the project |
| Look | Wisp's light and dark palettes, following the system or fixed to one in Settings; the shared icon set |
| Conversations | Create, select, restore messages, running / needs-you status |
| Conversation management | Rename, pin (shown first), archive and restore, delete with confirmation |
| Groups | Named groups per project: create, rename, reorder, delete (conversations stay); put a conversation in one from its bar; fold a section |
| Sort and group | The list by groups, by last activity or flat, newest first or by name; remembered across launches |
| Select mode | Pick conversations by click, Shift-click for a range, or all; move them to a group, archive or restore them, or delete them after one confirmation; a line reports what changed and why anything was left |
| Copy or move | Copy or move the open conversation's transcript to another project |
| Edit and send again | Under a message of yours: after one confirmation the conversation goes back to just before it and its text returns to the composer |
| Branch | Under a reply: a new conversation holding everything through that turn, opened at once and listed under the one it came from |
| Search | Titles and message text across the project, archived conversations included |
| Command palette | One box over everything: projects, files by name in every project, conversations by title or message text, and the window's commands; arrow keys, Return, Escape |
| Pictures in answers | When an answer shows a picture of the project with Markdown image syntax, the picture is drawn under the answer; clicking it opens it in the Files tab |
| Project archives | Export on a project's card writes the project, its records and its files, as one zip; Import on the home page reads one and puts the project in a folder you choose |
| Capabilities | A dialog for the open project: how many skills, connectors, MCP servers and SSH hosts are on or usable, the state of memory, and which command-line tools were found on this computer |
| Follow-up questions | After an answer, up to three questions you could ask next show above the composer; picking one puts it in the composer to send or change, and the list can be put away |
| Sample project | The home page's flask button makes a small sample project in a folder you choose and opens it: made-up counts for twelve samples, a script, and a README of things to ask |
| Host folders | With an SSH host saved, the Files tab offers it beside the project: its folders are listed, to walk through and see what is there |
| File origin | A file's preview has Origin: which tool call last wrote it, with the code it ran, the project files it read and the conversation, which it can open |
| Terminal | The Files tab's terminal button (or the command palette) opens the system's terminal in the folder being browsed |
| Updates | Settings, General, has Check for updates: it says whether a newer release has been published and opens the release page |
| Tabs | The conversations you open in a window line up as tabs above the conversation, across projects; click one to go back to it, close one to drop it from the strip |
| Sidebar | The project's sidebar hides for more room and comes back with the button left of the conversation's title or Cmd/Ctrl+B; it stays as it was left, across restarts |
| Notifications | When a turn ends or a tool waits for approval while Wisp is in the background, a system notification is posted and, if chosen, a sound is played (Settings, General) |
| Language | Settings, General, chooses the interface language: System (Chinese on a Chinese system, English otherwise), English or 中文 |
| Delete a project's folder | Removing a project offers Delete folder too: after you type the project's name, the project is removed and its folder is deleted with every file in it |
| What the model is sent | Clicking the context percentage by the composer (or the command palette) lists the conversation's working set: the instructions, the summary that stands in for compacted turns, and each message with about how many tokens it takes |
| Interpreters | The Python and R interpreters a conversation keeps between tool calls show above the composer with their version and state; Stop ends one, Dismiss takes a stopped one off the list |
| Experts | Settings, Experts, keeps personas (a name and instructions); the composer's Expert menu starts a new conversation as one, and its instructions hold for the whole conversation |
| Quick actions | Settings, Quick actions, keeps requests you make often under a name; the composer's quick actions menu puts one into the message box, after whatever is already typed, to finish and send |
| Undo a turn | Going back to the latest message can also undo its files: a file the turn edited gets its earlier content, a file it created goes to the project's trash, and a file changed again since is left and named |
| Table preview | A `.csv` or `.tsv` file is previewed as a grid of its first 200 rows and 30 columns, with a switch to its text |
| File actions | A previewed file's path can be copied, the file attached to the message, shown in the file manager or opened with the system's own application |
| Slash commands | A message that is only a command does something instead of being sent: `/review`, `/trajectory`, `/context`, `/btw <question>`, `/plan`, `/save-as-skill`, `/skills`, `/files`, `/upload`, `/memo`, `/remember`, `/archive`, `/compact`, `/timer 30m <prompt>`, `/fork`, `/rewind`, `/permission`, `/share`, `/help`; the composer's "/" button lists them |
| Browser | Settings, Browser, switches on a bridge to the Wisp extension in your own Chrome or Chromium; conversations can then open a tab, read it and run a script in it, only in tabs they opened themselves, and hosts can be blocked or preferred |
| Workflows | Settings, Workflows, keeps templates of a goal broken into tasks, each saying which earlier tasks it needs; Run starts a conversation in the open project and gives the model the tasks stage by stage |
| Remote web access | Settings, Remote access, opens this desktop's projects in a phone or a browser through a wisp-relay of your own: it dials out (no inbound port), shows whether the tunnel is up and how many browsers are connected, and gives a link and a code that can be copied or replaced |
| Plugins | Settings, Plugins, installs a plugin from a folder in the Claude plugin layout (`.claude-plugin/plugin.json`, `skills/*/SKILL.md`) into the app's data without running anything in it, lists what each brings, switches one on for the open project, whose conversations then have its skills, and removes it |
| Decision model | Settings, Decision model, sets up one fast endpoint (TypeSafe Jev, StartLux-Decision hosted or self-hosted, or a custom System One server) with its key, thinking mode and timeout, and tests it; while one is in use the model's `map_items` tool, which runs one step over many items, also offers its `decide` step |
| Pet | Settings, Pet, turns on an animated companion drawn from a folder you choose (pet.json and a sprite sheet, Codex-compatible version 2); it stands in the transcript's corner and shows whether the conversation is working, being reviewed, waiting for you, done or failed |
| Research archive | A conversation's Research archive button (or the command palette) has the model draft a report of its findings with the scripts behind them; you change the draft and freeze it, which writes it into the project and makes the conversation read-only |
| Review turns | The composer's Review turns switch has each turn that used a tool, or said a good deal, reviewed when it ends; the result shows above the composer, and See opens the findings |
| Review | A conversation's Review button (or the command palette) has the model read the conversation as a reviewer and lists claims its own tool output does not support, each with the evidence and a fix; Ask Wisp to correct puts the corrections in the composer |
| Save as skill | A conversation's Save as skill button (or the command palette) puts a request in the composer that asks Wisp to distill the conversation into a reusable skill; you send it, changed or not |
| Trajectory | A conversation's Trajectory button (or the command palette) shows every turn's steps in the order they happened: what you asked, what the model said, each tool with how it went and how long it took, and each model call with its tokens; the totals are at the top |
| Windows | New Window (the app menu, Cmd/Ctrl+Shift+N, or the command palette) opens another window on the same projects, each with its own page and conversation |
| Export | A conversation's Export button saves the whole of it as a Markdown file where you choose |
| Notebook | A tab of the side panel: the conversation's code as cells, with what each run returned; copy, save to favorites, or show it in the conversation |
| Research calendar | A page across every project: a month as a grid with how many records each day holds, and the chosen day's records; projects can be left out |
| Scheduled tasks | A page across projects (the clock on the home page, or the command palette): a prompt is sent into a new conversation of its project every few hours, daily or weekly at a time, while the app is open; each task can be run now, switched off or deleted, and its latest firing opens the conversation it made |
| Paper evidence | A page for the open project (Papers in the sidebar, or the command palette): the papers it is writing, each with its versions and, in a draft version, figures, tables and claims with the project results and runs they rest on and why each was chosen |
| Research journey | A page for the open project: one month of what happened, by day (what you asked, runs, files, decisions, your own records), a filter, and a form to add a record of your own |
| Outline | A button in the conversation's header lists what you asked, in order; choosing one scrolls to it |
| Find in conversation | Cmd/Ctrl+F, or the command palette: a bar over the transcript marks every message, and every tool call, that contains the words; Return and the arrows move between them |
| Help | Documentation (home page, Settings, palette) and Report a problem (Settings, palette) open the tutorials and the issue tracker in the system's browser |
| Settings | A search field narrows the pages to those about what you type. Pages: General (language; appearance: system, light or dark; the send key; notifications and sound; update check; what the app opens on), Network (a route for model requests, for connectors and MCP servers, and for the code the model runs; a conda mirror, a Python package index and a CA bundle the model is told to use), Models (saved models with provider, endpoint, key and limits; presets, a default, a connection test), Conversation (steps per turn, automatic compaction and its threshold, suggesting what to ask next, reviewing turns, the reviewer model and the sub-agent model), Skills (every skill with where it comes from, search, on or off), Connections (your own MCP servers: add over HTTP or as a local program, test, switch off, remove; and the bundled literature and biology connectors with their tools and links, search, on or off), Credentials (the service keys the connectors use, kept in the system keychain), Environments (the SSH hosts work can run on: add, change, remove, add from `~/.ssh/config`, check the connection and see what the host is, and which one new work goes to), Permissions, Usage (tokens in total, per day over the last thirty, by model and by project) and Storage (where the data is and how much room it takes) (how new conversations start, tools that are always allowed); applied without restart |
| Inbox | Conversations in any project that are waiting for you; opens across projects |
| Read state | A finished answer is marked read once it is on screen in the active window |
| Transcript | Virtualized Markdown, selectable text, paginated history |
| Tool calls | One folded line each with outcome and elapsed time; click for the verbatim input and output; interrupted calls stop claiming to run |
| Runs | A Runs tab: runs started by the conversation with kind, status, exit code, command and output tails; a run that has not ended can be cancelled, and is followed until it ends. In the transcript, a call that started or watched a run shows the run's state now, and Show in Runs opens its output there |
| Artifacts | Files a turn wrote are recorded when it ends; an Artifacts tab lists them for the conversation or the project and opens them |
| Side panel | Tabs: Files, Runs, Artifacts, Notebook, Agents, Highlights, Memos, Memory, Aside; the open tab is named, the others are icons |
| Agents | The Agents tab lists the helpers the conversation sent off to explore: what each was asked, how it went, and what it found |
| Aside | The Aside tab (or the command palette) asks the conversation's model a question about the open conversation; the answer shows beside it and nothing is added to the conversation |
| Favorites | A star under any message saves it as it reads now; an image in Artifacts can be saved as a figure. Highlights lists what this conversation saved; the Favorites page lists everything from every project, with search, a kind filter, the item in full, its source conversation and removal after a confirmation |
| Memory | One switch; habits the model is shown in every project; this project's notes by day, which the model can search; add, edit and delete either, and save any message to one or the other |
| Memos | Your own notes per project: add, edit, pin, copy, delete after a confirmation, and put one into the message box. The model is never shown them |
| Files | Project file browser, available before the first conversation; read-only text and Markdown preview, pictures (PNG, JPEG, GIF, WebP, SVG, BMP), and Jupyter notebooks as their cells with what each printed or returned (a figure is named, not drawn; Show as text gives the file itself); new folder, new file, rename, and move to the project's trash; listed by name, newest first or largest first, folders first, in the order last chosen |
| Composer | Native multiline input. Return sends and Shift+Return starts a new line, or, with the other choice in Settings, General, Return starts a new line and Cmd/Ctrl+Return sends; Cmd/Ctrl+Return always sends. While an input method is composing, Return goes to it. While a turn runs the same key queues the message: it is sent when the turn ends, or given to the running turn at once with the queue's lightning button |
| Plan first | The model investigates and submits a plan as a card; nothing is changed until you approve it, which leaves plan mode and starts the work |
| Thinking | What the model is thinking, above the composer, while it thinks; afterwards it stays in the transcript as a folded Thought ahead of the answer, to open when wanted |
| Conversation options | Ask first or Full access, the model, reasoning effort, and where its work runs (the default, this computer or a saved SSH host), per conversation; all are kept across restarts and apply from the next message |
| Usage | How full the context is, and the tokens the running or latest turn used |
| Questions | When the model asks, a card with its options: one click answers, or type the answer |
| Messages | Copy any message as Markdown |
| Attachments | Attach files with the system dialog or by dropping them onto the conversation, or add a project file, another conversation or a skill from the add-context box; shown as chips until sent |
| Drafts | Survive navigation; preserved on send failure, cleared on durable acceptance |
| Execution | Shared agent/tools, streamed snapshots while running, stop and errors |
| Approvals | Exact pending request; allow once, allow that tool for the rest of the conversation, always allow it in this project, or deny |
| Escape | One window-level stack: Escape closes only the topmost of command palette, file preview, Files panel, Settings, title editor, delete confirmation, remove-project confirmation, the Full access question, the model and effort menus, the remove-model question, the sort menu, select mode and its group menu and delete question, a group's name field, actions and delete question, the conversation's group and move menus, the go-back question, the memo editor and its delete question, the memory editor and its delete question, and the Favorites page and its remove question |
| Shortcuts | Cmd/Ctrl+K opens the command palette; Cmd/Ctrl+N starts a conversation in the open project; Cmd/Ctrl+, opens Settings |
| Lifecycle | Background IO, stale-result rejection, cancel/clean workers on exit |

Lists and search results show up to 100 conversations per project. Drafts do
not yet survive app restarts. Messages are
text-only. The initial project keeps the embedded host's `Server` name; opened
folders use their own names.

Runs, status and the inbox use commands the embedded host already serves.
Conversation management and organisation, Settings, the home page, search and
project file access are host operations in `crates/wisp-server/src/desktop.rs`. They are
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
  project. A running conversation must be stopped before it can be archived,
  deleted, copied, moved, rewound or branched. There is no export, import or
  share yet.
- Going back to a message removes it and everything after it from the
  conversation for good. Files those turns wrote, their runs and their
  artifacts are not undone, and files that were attached to the message are
  not re-attached. The desktop names the message by how many of your messages
  follow it and what it says; if that does not match the saved conversation
  the host changes nothing and says so. This replaced counting rows, which is
  how a past release cut a conversation in the wrong place.
- A branch is a copy: the model's context through the chosen turn, in the same
  project and group, with the source's model, effort, Plan first and Full
  access. It is linked to its source for display only, so it can be rewound
  and branched like any conversation. The WebView's branch cards and merging a
  branch back with a summary are not ported, and a branch made here does not
  take part in them. A conversation cannot be deleted or moved while it has
  branches; delete those first.
- A group holds a conversation together with its branches. Group names are
  unique in a project, ignoring case, and at most 60 characters. Groups are
  reordered one place at a time; conversations are not dragged. Sections show
  only while browsing: search results keep the host's order and the archive is
  a flat list. Date sections are spans of elapsed time (24 hours, 7 days, 30
  days), not calendar days. Which sections are folded is forgotten on restart.
- Copying or moving takes the transcript only: files, runs and artifacts stay
  in the project they were made in, and a moved conversation gets a new id.
  The conversation keeps its model, effort and Plan first choices and goes
  back to Ask first, since Full access was given for other files. It arrives
  read. The WebView's option to move artifacts along is not ported.
- A batch applies the single-conversation rules one by one and carries on
  past a refusal, so a running conversation is left while the rest change.

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
- The desktop now loads the skills that ship with Wisp (`skills/` in a
  checkout; an installed build has to name its resource directory) after any
  directory in `WISP_SKILLS_PATH`, then the project's own `skills/` and
  `.wisp/skills/`. Before, only the last two and the variable were read, so a
  desktop without the variable had no bundled skills. A headless host is
  unchanged. Turning a skill off uses the WebView's list (`disabled_skills`)
  and applies to every project from the next turn: the model's skill tools
  and the add-context box no longer offer it. Editing tags, per-project
  enabling, the community catalog and installing from GitHub are not ported.
- The host now gives every turn the bundled literature and biology
  connectors (`wisp-bio`: PubMed, bioRxiv, UniProt and the rest), which the
  WebView has always had. They only read, so they never ask for approval, and
  their schemas are deferred, so they do not fill the prompt. Turning one off
  uses the WebView's list (`disabled_connectors`). They follow the Network
  page's route for connectors. Per-conversation connector grants are not
  ported.
- Your own MCP servers are kept in this database (`desktop.mcp_servers`), not
  in the WebView's list, so each desktop has its own. A server is reached
  over HTTP, with an optional bearer token kept in the keychain, or started
  as a program on this computer; other headers, environment variables for
  the program, OAuth and per-server proxies are not ported. Connections are
  kept between turns and closed when a server is changed, switched off or
  removed, and when the app quits. The model calls a tool as
  `<server>__<tool>`; every such tool asks before it runs, and that question
  cannot be answered "for this conversation" or "always". A server that
  cannot be reached gives a turn no tools and says why in Settings; it never
  stops the turn. MCP Apps are not ported.
- Credentials holds the three keys the connectors read: the NCBI key and
  contact email, and the OpenAlex key. They go to the keychain under this
  data directory's own names (`desktop.service.<id>.<name>`), never into
  SQLite, are read at the start of each turn, and are not handed back to the
  window once saved. The WebView's keys are a separate set and are not read.
  Its other service keys and custom environment keys are not ported.
- Network keeps the WebView's value under its key (`network_settings`). A
  route is empty (follow the system and the environment), `none` (connect
  directly) or an `http`, `https` or `socks5` proxy address; an address with
  a user name or password in it is refused, because it would be kept in the
  database in the clear. Model requests and the bundled connectors take
  their route from the next turn, MCP servers reached over HTTP reconnect
  when the page is saved, and the code route is handed to the commands the
  tools start as the usual proxy variables. That last one is one value for
  the whole app, as in the WebView. A model request that cannot connect says
  which route it took. The mirror, the index and the CA bundle are advice in
  the prompt of every turn, not a rule; nothing checks that they exist. A
  route for one model, one MCP server or a subscription sign-in is not
  ported; the sign-in route a WebView database carries is kept as it is.
- Environments keeps SSH hosts in the WebView's list (`ssh_hosts`) and
  registers each as the execution context `ssh:<name>`, which is what the
  model passes to `run_in_context`. Every turn's prompt names them and the
  one new work goes to, in a conversation that was started earlier too. A
  host is a name plus either an address or nothing at all, in which case
  OpenSSH resolves the name from `~/.ssh/config`. Wisp keeps the path of a
  key file, never the key, and signs in with your keys, agent and config;
  password sign-in is not ported, so a host that needs one cannot be used
  from here. Work is refused on a host until a check of it has passed: Check
  connects once, without prompting, and records the system, processors,
  GPUs, scheduler and the Python and R it found, which is also what the
  model is told about the host; a check that fails records why, and the
  model is told not to try the host. The check is the WebView's probe
  (`wisp_runs::context_probe`). WSL, interpreter paths set by hand, and
  scratch and storage locations are not ported. Removing a host keeps the
  records of runs made on it and puts new work back on this computer if it
  was the default.
- A conversation may run its work somewhere other than the default: the
  composer shows where once a host is saved. The choice is kept the way the
  WebView keeps it (`frame_default_execution_context:<conversation>`), is
  carried into a branch and a copy, and is dropped when the host is removed.
  It decides which host the prompt names as the one work goes to, and so
  where the model starts runs; it is not a restriction, and the model may
  still name another saved host. The Python and R sessions and the shell
  stay on this computer whatever is chosen, because this host has no remote
  interpreter. Attaching hosts to one conversation only, the terminal, and
  the right panel's Environments tab are not ported.
- Conversation settings apply to every turn this host runs from the next one
  on, a paired phone's included. Steps per turn replaces the `WISP_MAX_ITER`
  the host was started with once it is saved; at the limit the model is asked
  for a last word without tools. Compaction uses the WebView's keys
  (`auto_compact`, `auto_compact_percent`). Auto-continue, compaction on a
  model switch and after idle hours are not ported.
- The sample project is four small text files with made-up numbers,
  written into a new folder ("Wisp sample project", numbered if that
  exists) under the folder you choose, never over anything. Unlike the
  WebView's example it has no prepared conversations and is not read
  only: it is an ordinary project, and asking about it uses your model.
  Removing it is removing a project.
- A host's folders are only listed: files there cannot be opened,
  downloaded, uploaded, renamed or removed from the Files tab, and
  nothing on the host is changed by looking. The host has to have passed
  a check in Settings, as for runs. Each folder is one SSH request over
  the run manager's connection with a twenty-second limit; at most 2,000
  entries are listed, fewer when names are very long, and the folder
  says when there was more. Hidden files are not listed. WSL is not
  offered.
- Origin is known only for files written by tool calls since this was
  added: each call that reports what it ran is kept in the store's
  execution log (the table the WebView fills) when its turn ends, with
  up to 64 KB of its code. A file you put in the folder yourself, one a
  shell command wrote without saying so, or one written earlier has no
  origin. The open conversation is looked in first, then the project's
  others; the latest call in the conversation it is found in is shown,
  not necessarily the latest overall. Deleting a conversation deletes
  what it knew.
- The terminal is the system's, not one inside the app: Terminal on
  macOS, `cmd.exe` on Windows, whatever `x-terminal-emulator` is on
  Linux, and a message when there is none. It opens in the folder the
  Files tab is showing, on this computer; what is typed there is yours,
  the model does not see it, and there is no terminal on SSH hosts.
- Whole log reads `stdout.log` and `stderr.log` from the run's own
  folder (`~/.wisp-science/runs/<id>`), the last 512 KB of each, when it
  is asked for; it does not follow a run that is still writing, so press
  it again to see more. Only runs on this computer have it: a run on an
  SSH host keeps its log on that host, and its last lines are what the
  Output shows. Once a run's folder is gone, so is its whole log.
- The update check runs only when its button is pressed: nothing is
  checked at start or in the background. It asks GitHub for the latest
  release of the project (the WebView's request, with a generic client
  name and nothing about you or your work), over the system's proxy
  settings, and gives up after eight seconds. Nothing is downloaded or
  installed: Open releases opens the project's release page in your
  browser, and only that page, whatever the answer contained. This
  desktop has no installer of its own yet, so the release found is the
  WebView desktop's.
- Tabs belong to the window and are not saved: a new window, or the app
  after a restart, starts with none, and the strip shows from the second
  conversation on. A window keeps twelve; one more closes the tab that
  has gone unvisited the longest. Closing a tab closes nothing else: the
  conversation and a turn running in it are untouched. Tabs cannot be
  reordered or moved between windows.
- Notifications are for the conversation that is open in a window, and
  only while that window is not the active one: a turn that ends in a
  conversation you are not looking at is shown by the sidebar, not
  announced. They are posted with what the system already has: on macOS
  through `osascript` (so the banner is filed under Script Editor, and
  the first one may ask for permission) and `afplay`; on Linux through
  `notify-send` and `canberra-gtk-play`, silently doing nothing when
  those are missing. Windows gets the sound only. The sound cannot be
  chosen. Nothing is posted about turns a paired phone started unless
  their conversation is open here.
- The Chinese interface is not complete. Translated: the fixed text of
  the pages, panels and dialogs, the names of panel tabs, palette commands
  and settings sections, the placeholders of text fields, statuses,
  counts, ages, durations, token figures, run statuses and the sentences
  with a value set into them (the trajectory, usage, storage, review,
  /context and Settings summaries among them). Dates read the Chinese
  way (2026年11月, 11月14日 星期六). The host's refusals are translated
  when they are fixed sentences; the ones that carry a value (a path, an
  address), tool questions and the model's own words stay as written.
  Short units (ms, s, KB, MB) are the same in both. A text without a
  translation shows in English rather than not at all. In Chinese the
  command palette matches the Chinese names of commands, not the English
  ones. The system's language is read from the macOS locale, or from
  `LC_ALL`, `LC_MESSAGES` or `LANG`; where none says, System is English.
  The model's replies are not affected by this setting.
- Delete folder too deletes for good: nothing goes to the Trash and there
  is no undo. It deletes only the folder registered for the project, and
  refuses one that is a whole disk or volume, sits at the top of a disk,
  holds your home folder, holds Wisp's own data or holds another project.
  A registered path that is a link loses the link and keeps the folder it
  points at. It is refused while a turn or a run of the project is going,
  and for the built-in workspace. If the folder cannot be deleted the
  project stays listed. The question counts up to 200,000 entries before
  it says "at least". A paired phone cannot ask for it.
- What the model is sent is read from what is saved, so it is the
  working set of the last turn: parts of the instructions that are
  refreshed when a turn starts (compute contexts, project rules, package
  sources) show as they were then, and a turn that is running shows only
  what it has saved. Token figures are the length in characters divided
  by four, not the model's own count; tool definitions are sent too and
  are not listed. A row's opened text stops at 16,000 characters. A
  compaction cannot be undone from here.
- The interpreter line is read when a conversation opens, about once a
  second while a turn runs, when a turn ends and after Stop; an
  interpreter that dies while nothing is running shows as stopped only at
  the next of those. Stop loses what the interpreter had in memory and
  keeps the files it wrote; a turn that is using it gets an error from
  that tool call. A stopped interpreter stays listed until dismissed, and
  the model's next call says it was stopped rather than quietly starting
  a new one. There is no list of variables, no restart button and no
  memory limit; interpreters are also stopped when the conversation or
  its project is removed and when the app quits.
- The Agents tab is a reading of the loaded transcript, like the
  Notebook: it lists `explore` calls, the only kind of sub-agent this
  host runs, from the messages that are loaded, and shows a helper's
  findings once it has reported back, not its steps along the way.
  Delegated tasks, child conversations and workflows are not run here,
  so they have nothing to list.
- The sub-agent model is the model the `explore` helper works on, the
  only sub-agent this host has. The order is the WebView's: a
  conversation's own choice for its sub-agents (its `frames` column,
  which this desktop reads but cannot set), then the choice in Settings
  (`subagent_model_id`), then the conversation's model. A choice naming
  a profile that is gone or unusable falls through to the next. It
  applies from a conversation's next turn.
- An expert is chosen before a conversation's first message and cannot
  be changed afterwards: its instructions are written into the system
  prompt with the first turn and stay as they were then, also if the
  expert is later changed or removed. The list and each conversation's
  choice are kept under the WebView's keys (`specialists`,
  `frame_specialist:<id>`), and what the WebView keeps on an expert that
  this desktop does not use (its own model, skill and connector lists) is
  left as it is but not applied here: a conversation started as an expert
  runs on the conversation's model with every enabled skill. The
  WebView's one-shot built-ins (reviewer, reader, archivist, recap) are
  not offered. The built-in Scientific Illustrator draws SVG; this host
  has no image-generation tool.
- A quick action here is a named request put into the message box; it
  sends nothing by itself and starts no workflow. The WebView's quick
  actions start multi-agent workflows, which this host does not run, so
  the two lists are separate: this desktop keeps its own under
  `desktop.quick_actions` and neither reads nor changes the WebView's.
  It starts with four that can be changed or removed; at most 40 are
  kept, a name is at most 60 characters and a request 8,000.
- The table preview reads the usual quoting (a quoted field may hold the
  delimiter, a line break or a doubled quote) and takes the first row as
  the header. It shows what the host sent, so of a file over 1 MB only
  the rows in the first 1 MB; cells are cut at 80 characters and show
  their first line. There is no sorting, filtering or column resizing.
- Only the latest turn's files can be undone, and only text files the
  write and edit tools changed, up to 10 MiB each: what a shell command
  or a script wrote or deleted is not known and stays. The earlier
  content is kept under the project's `.wisp/undo/` from the end of the
  turn, so a turn made before this was added, or one that was stopped
  before it ended, has nothing to put back. There is no preview of
  what will be undone before it is done; the note afterwards says what
  was. Files the turn created are moved to `.wisp/trash/`, not erased.
- `/timer` makes a scheduled task bound to the open conversation: the
  prompt is sent into it every so many minutes, hours or days while the
  app is open, as a new turn each time. It does not replace the turn
  before it as the WebView's timer does, cannot be paused, and a firing
  that finds a turn running there is recorded as failed rather than
  queued. `/timer off` removes the conversation's timers; they are also
  listed, marked, on the Scheduled tasks page.
- `/compact` archives the conversation's whole history under the
  project's `.wisp/history/` and shortens what the model is sent, as a
  turn does by itself when the context fills up. It is refused while a
  turn runs and in an archived conversation. There is no choice between
  pruning and summarising, no instruction for the summary and no undo;
  the transcript on screen keeps its length.
- A slash command is read only when it is the first word of the message
  and one of the known ones, so a message that starts with a path is
  sent as a message. There is no list that narrows as you type.
  `/fork` branches the conversation as it stands, `/rewind` asks about
  going back to the latest message, `/permission` switches full access
  (asking first when it turns it on), and `/share` exports the
  conversation as Markdown rather than publishing it anywhere.
- The browser bridge is off until it is switched on, and then listens
  on this computer only, on the port the extension dials
  (127.0.0.1:18765), so it and the WebView desktop cannot both hold it.
  Only the Wisp extension's origin is let in. It is a smaller bridge
  than the WebView's: four tools (`browser_setup`, `web_open_tab`,
  `web_scan`, `web_execute_js`), one session, and a conversation may
  read and drive only the tabs it opened (and tabs those open), never a
  tab the user has open. A script may be page JavaScript or the command
  that closes the conversation's own tabs; the extension's other
  commands (DevTools protocol, switching or creating tabs by command,
  downloads, screenshots, chat-site turns) are refused. Not ported:
  starting the browser, closing a turn's tabs when it ends, the
  human-verification prompt, the side panel, the isolated workspace
  session, updating the extension, screenshots and saving assets. The
  extension is loaded by hand from the folder the page names; a packaged
  app has to ship that folder for the page to find it. Every tool asks
  first unless it was allowed. The host lists are kept under the
  WebView's key (`browser_url_filters`); the switch is this desktop's
  own (`desktop.browser.enabled`). The bridge was tested against a
  stand-in for the extension, not against a real browser.
- A workflow here runs as one conversation: the host lays the tasks out
  in the stages their dependencies allow and the conversation's model
  carries them out in that order, with the usual approvals. There are
  no separate agents per task, no executors or per-task models, no
  permissions or output contracts per task, no approval policy, no
  retries of a single task and no run history; tasks of one stage are
  not run at the same time. A task can only need tasks listed before
  it, and there is no graph editor and no converting skills into a
  workflow. Because the WebView's templates describe agents this host
  cannot run, the two libraries are separate: this desktop keeps its
  own under `desktop.workflows` and neither reads nor changes the
  WebView's `workflow_templates`. It starts with two that can be
  changed or removed; a workflow has at most 20 tasks and there are at
  most 40.
- Remote web access is the only channel ported: there is no project
  sync through the relay, Feishu, WeChat, StickS3 or Wisp-Lab. A browser
  gets the commands a headless `wisp-server` offers (projects,
  conversations, messages, stopping a turn, answering an approval
  once), which is fewer than the WebView desktop's tunnel: no research
  calendar or journey, checklist, queue, file browser or picture
  upload. There is no QR code; the link and the code are copied
  instead. The tunnel dials the relay directly and does not use the
  proxy set under Network. Whether it is on and the relay's address are
  kept under the WebView's keys (`remote_access_enabled`,
  `remote_relay_url`); the relay token and the code are kept in the
  keyring under this data directory's own names
  (`desktop.remote.<id>.token`, `.code`), so they are typed here once
  and the code differs from the WebView's. The page looks at the
  tunnel for a few seconds after a change, not continuously: open it
  again to see the present state.
- A plugin is installed from a folder only: not from a zip archive or
  an address, and with no published checksum to compare against, so it
  is recorded as unverified. One version of a plugin is kept; a newer
  one replaces its files, and a project that had the older one on has
  to switch the new one on. A package with a symbolic link, more than
  10,000 files or 512 MB is refused. The MCP servers a plugin declares
  are listed with the command that would start them, but this desktop
  does not start them, so only a plugin's skills take effect. Those
  skills do not show on the Skills page and cannot be turned off one by
  one there. The records are the store's own (`plugin_installations`,
  `project_plugins`), shared with the WebView desktop; removing a
  plugin that desktop installed removes the record and leaves its
  files where that desktop put them.
- The decision model is used by `map_items` only; this host has no
  `desktop_autopilot`. `map_items` works on the sub-agent model. The
  form is kept under the WebView's `decision_model` key, but each
  service's key is kept in the keyring under this data directory's own
  name (`desktop.decision.<id>.<service>`), so a key saved in the
  WebView, including an older TypeSafe key, is not picked up and has to
  be typed here once. A change applies from a conversation's next turn.
- The pet follows the conversation on screen only, inside the window:
  there is no separate desktop pet window, it does not walk about or
  look at the pointer, and it does not follow runs or other
  conversations. The choice is kept under the WebView's keys
  (`pet_enabled`, `pet_directory`), so both desktops show the same pet.
  The folder is checked each time it is read (a 1536x2288 PNG or WebP
  sheet of at most 16 MB inside the folder); a folder that stops being
  a pet is reported in Settings and nothing is drawn.
- A research archive is written once and cannot be undone from here:
  the report and scripts go to `.wisp/research-archives/<id>/` in the
  project, the record goes to the store's `research_archives` table (the
  WebView reads both), and the conversation is read-only from then on,
  for this desktop, the WebView and a paired phone. The draft is one
  model request over the conversation's latest 80,000 characters, with
  tool output cut at 4,000 each, so a long conversation is drafted from
  its end; the model is given five minutes. Unlike the WebView nothing
  is snapshotted, moved or deleted: project files stay exactly as they
  are, the scripts in a draft are frozen as the model wrote them (their
  names are listed, their text is not editable here), and there is no
  "continue from this archive" link in a new conversation. Closing the
  dialog before freezing throws the draft away.
- Review turns is off until switched on, per conversation; Settings,
  Conversation, sets what conversations start with (the WebView's keys,
  `frame_auto_review:<id>` and `auto_review_default_enabled`). A
  conversation that was never switched follows the default; one that was
  keeps its choice. Unlike the WebView, a review that finds problems does
  not hold the turn open or correct it: the turn is over, the result is
  shown, and corrections are asked for with Ask Wisp to correct. A turn
  that failed, was stopped, ended on a question to you, or only said a
  little without tools is not reviewed. The result is kept in memory
  until the next turn starts. It applies to every turn this host runs,
  a paired phone's included, though a phone is not shown the result.
- A review is one model request without tools, made by the reviewer model
  chosen in Settings, Conversation, or by the conversation's own model
  when none is chosen or the chosen one was removed. The review button
  under an answer reviews the conversation up to the end of that turn,
  leaving out what came after. It reads the
  conversation's latest messages up to 80,000 characters, with each
  tool's arguments and output cut at 4,000, so a claim that rests on
  something older or longer cannot be traced and is not reported. The
  report is shown once and not kept: closing the dialog, switching
  conversations or restarting loses it. It is refused while a turn runs.
  Its tokens are not in the usage statistics or the trajectory. A model
  that does not reply with a report gives an error, not an empty review.
- Save as skill writes nothing by itself: it only puts the request in the
  composer, after whatever you had typed. The work is done by the bundled
  `skill-creator` skill in an ordinary turn, so it needs that skill turned
  on in Settings and asks for approval like any turn that writes files.
- A question asked aside is one model request without tools: the
  conversation's model reads the last twelve turns, with long messages and
  tool results cut short, the latest eight side exchanges, and the
  question (at most 4,000 characters). It can be asked while a turn runs
  and then reads what has been saved so far. The answer is the model's
  reading of the conversation, not a search of the project: there is no
  list of the evidence it used, and a question cannot be tied to one run
  or to selected text. The thread is kept per conversation in the window
  that asked, until the app closes; nothing is written to the database,
  and the tokens are not in the usage statistics or the trajectory.
- Follow-up questions cost one more, short model request after every turn
  that ends on an answer: the conversation's model reads the last four
  turns, with long messages and tool results cut short, and may write up to
  1,024 tokens. Those tokens are not in the usage statistics or the
  trajectory. The switch is the WebView's (`follow_up_questions`, on until
  turned off) and applies to every turn this host runs, so a paired phone
  is offered the questions too. Nothing is suggested after a turn that
  failed, was stopped, or ended on a question to you; a model that does not
  reply with a list suggests nothing, silently. The questions are kept in
  memory: they are gone when the next turn starts or the app restarts.
- "Always in this project" is kept in the database
  (`desktop.approvals.<project>`) and answers the standard tool question for
  every conversation of that project, in turns that start after it was given
  and for the rest of the one it was given in. It never answers the question
  a dangerous shell command asks. Taking it back in Settings applies to turns
  that start afterwards. The WebView's own approval rules (per-tool allow, ask
  or deny, the three scopes, global grants) are a different store and are not
  read. New conversations start in the mode Settings names only when the
  desktop makes them; one a paired phone makes starts in Ask first.
- Full access stops the questions before tools that change files or run
  commands. It does not cover the separate question a dangerous shell command
  asks; that one always appears. "Allow for this conversation" is offered only
  for the standard tool question, is kept in memory, and is forgotten when the
  app restarts; "Ask again" under the transcript revokes it sooner.
- Options changed while a turn runs apply to the next one. What each model
  call used is saved with the conversation when the turn ends, as the event
  the WebView writes, and the Usage page counts from those rows; turns made
  before this was added are not in it. The figures under the composer are
  still kept in memory only: after a restart they are empty until the next
  turn. Usage has no ranking of skills and MCP tools, no per-conversation
  breakdown and no year calendar.
- The Files panel changes files only inside the project and never in its
  `.wisp` or `.git` folders. A new folder or file, and a rename, replace
  nothing that is already there; a name is a single name, without slashes.
  Deleting does not erase: the entry is moved into `.wisp/trash/<time>-<n>/`
  in the project, which the file browser does not show, and the panel says
  where it went. Nothing empties that folder; remove it yourself when you
  want the space back, which the Storage page counts as part of `.wisp`.
  The WebView erases at once instead. A link is renamed or moved as a
  link; what it points at is left alone. Uploading into a folder, moving
  between folders, sorting and the grid view are not ported. A picture is
  shown whole, scaled to the panel, when it is at most 32 MB; there is no
  zoom. PDF, Office, notebook and table previews are not ported.
- A picture is drawn under an answer when the answer names it as a
  Markdown image (`![plot](figs/plot.png)`) and the target is a picture
  inside the project: PNG, JPEG, GIF, WebP, SVG or BMP, at most 32 MB. It is
  drawn after the answer's text, not where the link stands, at most four per
  answer and forty per project at a time, scaled down to fit. Web
  addresses, absolute paths and paths that leave the project are never
  fetched. A picture that only a tool wrote, or that the answer mentions
  without image syntax, is not shown; it is in the Artifacts tab and the
  Files panel. There is no zoom and no click to open.
- Scheduled tasks are the only part of the WebView's research assistant
  that is ported: there is no plan for the day, daily recap, memory
  tidy or mandate. Tasks fire only while the app is open, looked for
  every 30 seconds; a time that was missed fires once at the next look
  and the cadence keeps its place on the clock. A fired task is an
  ordinary turn in a new conversation named after the task: it uses
  the default model and waits for approvals like any other, and the
  sidebar shows the conversation the next time the project list is
  refreshed. A task cannot be bound to an existing conversation or to
  a skill, and cannot be edited, only deleted and made again. The rows
  are the store's own (`schedules`), shared with the WebView desktop;
  a conversation's own timer made there is neither listed nor fired
  here.
- Paper evidence keeps papers, versions, items and evidence in the same
  tables the WebView desktop uses, so a paper made in one shows in the
  other. From here a paper has the one version it starts with: a version
  cannot be finalized, published, copied into a new one, packed as a
  capsule or reproduced, and there is no readiness check. A version the
  WebView finalized is shown with its evidence and cannot be changed, and
  a paper with such a version cannot be removed. Evidence is a registered
  result of the project (bound at the version it has when chosen) or a
  run; passages of a conversation, tool calls and outside resources
  bound in the WebView are listed by kind but cannot be added here.
  Evidence is not reviewed or marked from here, and items are flat:
  they are shown in the order they were added and cannot be nested or
  reordered.
- The research journey reads the project's history from the same tables the
  WebView does, a month at a time, grouped into your computer's local days.
  A record of your own is progress, a finding, a decision or what comes
  next, and is dated when you save it; it cannot be dated otherwise, changed
  or removed here. A row that came from a conversation opens it. The
  relation graph, the daily recap the model drafts, the sources of a file,
  and the history of an exploration branch are not ported, and neither is
  the calendar across projects, which is a page of its own.
- The research calendar reads the same history as the journey for every
  project at once, a month at a time, in your computer's local days. It
  opens on today; another month opens on its last day with something on
  it. A project that cannot be read is named with the reason instead of
  being shown as empty. Which projects are left out is kept only while the
  app runs. A row that came from a conversation opens it. Records cannot be
  added from the calendar.
- Every window talks to the one host in the app, so projects, settings and
  conversations are the same in all of them, and each keeps its own page,
  open conversation, panels and queue of waiting messages. A window does
  not hear at once what another changed: lists catch up when they are next
  read or refreshed. Two windows on one conversation both follow its turn;
  the host still runs one turn at a time in it. Closing a window leaves
  the others; the app quits with the last. Windows are not restored at the
  next start, and a project or conversation cannot be sent to a new window
  from its row.
- A project archive is the WebView desktop's format (`wisp-project`,
  version 1: a manifest, a snapshot of the project's records, and the
  workspace files), written and checked by the same code, copied into the
  host (`wisp_server::project_archive`), so an archive made by either
  desktop opens in the other. Export is refused while a turn or a run of
  the project is going, and no turn starts while it is written. Import
  keeps the project's id, so a project that is already here is refused
  rather than merged, and the project is placed in a new folder under the
  one you choose, never inside the app's data. There is no progress
  display, a large project simply takes a while; the folder form of a
  package, relay sync and remote access are not ported.
- Trajectory reads what is saved, so a turn that is still running shows
  only as far as it has been saved, and its model calls appear when it
  ends. Saved times are whole seconds: model time and speed are rough for
  short calls, and a call is timed from the question or the latest tool
  result before it. A tool's own time is measured to the millisecond. A
  model call with nothing in its turn to be timed from is listed, but is
  left out of the model time and the speed.
  Conversations from the WebView desktop show their steps, but that
  desktop records tool outcomes differently, so their tools show no
  outcome or time here. A step's opened text stops at 4,000 characters.
- Capabilities counts what the Settings pages and the Memory tab already
  hold, in one place, and looks along the search path the app was started
  with for `python3`, `Rscript`, `uv`, `pixi`, `conda`, `git`, `ssh` and
  `node`. A tool installed where that path does not reach reads as not
  found although a shell may find it. An SSH host counts as usable only
  after a check of it has passed. The list of tools cannot be changed, and
  no interpreter or tool path can be set from here.
- Export writes one Markdown file: the title, the project, what you and
  the model said in full, and each tool call with what it was given and
  what it returned, each cut to about four thousand bytes. It is read from
  what is saved, the whole conversation and not only the loaded part; a
  turn that is still running is in it only as far as it has been saved.
  Pictures are not embedded, and there is no HTML, long image or share
  link.
- The notebook is a reading of the transcript, as in the WebView, not
  something saved: every `python`, `r` and `shell` call with its output,
  and every code block an answer shows; code that was shown and then run
  is one cell. It covers the part of the conversation that is loaded.
  Output is cut to fourteen lines in the panel. Whether a run failed is not
  marked, because the host does not record it on the row. Cells cannot be
  run or edited, and a run started with `run_in_context` is in the Runs
  tab, not here.
- The outline lists your own messages in the part of the conversation that
  is loaded, by their first line; earlier ones appear once they are loaded.
  It has no headings from the answers and no search of its own.
- Find looks through the part of the conversation that is loaded, ignoring
  case, in messages and in what tools printed. It marks whole rows, not the
  words inside them, and does not open a folded tool row. Earlier messages
  are searched once they are loaded; to search every conversation's whole
  text use the command palette. The place is kept while a turn adds to the
  transcript.
- A message written while a turn runs waits in a queue above the composer
  with what was attached to it, up to ten per conversation, and is sent when
  the turn ends; several go one turn at a time, in order. A waiting message
  can be taken back into the composer or dropped. The queue holds, and says
  why, when the turn ended with a question or a plan that waits for your
  own answer, or with an error; "Send the next one" overrides that. The
  queue belongs to the window: it is not saved, so quitting drops it; it
  moves only while its conversation is the one that is open; and a paired
  phone neither sees nor feeds it. While the turn runs, the lightning button
  on a waiting message without attachments gives it to the turn at once: the
  turn reads it at its next step, as your message, and answers it before it
  ends; one that arrives after the turn's last look is answered in the same
  turn. If the turn has already ended the message simply stays in line.
- Project settings saves the name and description in the database and the
  rules in the project's own `.wisp/WISP.md`, the file the WebView calls
  Agent Context. The file is the truth: it is read again at the start of
  every turn, so rules saved here, or edited in the file, reach
  conversations that are already open, and so does a change to the
  project's `AGENTS.md`. A name is one line of at most 120 characters, a
  description at most 2,000, the rules at most 20,000; the WebView has no
  such limits, so longer rules written there can be read here but not saved
  again unshortened. The description is for you and is not shown to the
  model. The built-in workspace can be renamed like any other. Sync, run
  retention and deleting a project together with its data are not ported.
- Storage measures Wisp's own files: the database, the favorites, and each
  project's `.wisp` and `uploads` folders (up to 50,000 entries each). It
  does not measure your data, and nothing can be moved or cleaned from the
  page. Thinking is kept as the model reported it; a model that reports
  none leaves no Thought. Fast mode is not ported.
- Plan mode uses the WebView's switch (`frame_plan_mode:<conversation>`) and its
  prompt section, which now lives in `wisp_tools::plan` for any host to use;
  `src-tauri` still carries its own copy of the same text. Approving sends
  "Approve and execute", or whatever is typed in the composer.
- Files dropped onto a conversation are attached the way picked ones are.
  A drop takes at most twenty files; folders are left out, and it says how
  many items it did not attach. Pasting a file or a picture from the
  clipboard is not ported.
- A file attached from outside the project is copied into the project's
  `uploads` folder (up to 100 MB, never overwriting); a file inside it is used
  where it is. The model receives project-relative paths and reads the files
  with its tools; image pixels are not sent. A referenced conversation is
  passed as its most recent exchange (about 12,000 bytes), and a skill as its
  instructions, for that turn only: neither is saved with the message. Paste,
  drag and drop, and typing `@`, `#` or `/` in the composer are not ported.
- Runs can be cancelled. Harvest, review of a remote workspace, transfers and
  the full log are not ported.
- Favorites are the WebView's library: a database of their own
  (`library.sqlite` in the data directory) that holds copies, so they outlive
  the conversation and the project they came from. A message is saved whole,
  as text, and is recognised as saved by its exact text, so two identical
  replies share one star. Saving a selected passage, saving a code cell and
  editing a saved snippet into versions are not ported; code saved by the
  WebView is listed and shown. A figure is an image file of the project, up to
  32 MB, copied when it is saved. The file is created the first time a desktop
  opens the favorites, never by a headless host.
- Memory uses the WebView's switch (`memory_enabled`), its habits table and
  the same note files (`.wisp/memory/<day>.md`), and the wording that shows
  habits to the model now lives in `wisp_core::global_memory_block` for any
  host; `src-tauri` still carries its own copy. The switch applies to every
  turn this host runs, a paired phone's included: before, the search tool
  was always on and habits were never shown. Saving a message opens it for
  trimming; nothing is suggested by a model, and nothing is saved until you
  choose where. The WebView's automatic proposals after a turn, the weekly
  tidy, failure analysis and "clear all notes" are not ported. One entry or
  habit holds 4,000 characters.
- Memos use the WebView's table and limits (a body of 1 to 20,000 characters,
  a title of up to 200), so both desktops see the same notes on the same
  database. There is no search within memos and no `/memo` command in the
  composer. Putting a memo into the message box appends it to what is there;
  nothing is sent until you send it.
- An artifact is a project file a tool wrote during a turn, recorded with the
  WebView's identity (`path:<relative path>`) and snapshot rules, so the two
  desktops would agree on the same database. Files outside the project, and
  under `.wisp` or `.git`, are not recorded. Versions, download, starring and
  provenance are not ported.
- Status refreshes every two seconds while any turn is running and once more
  when the last one stops; it is not pushed by the host.
- General has the language, appearance, send key, notifications, sound,
  update check and what the app opens on. The WebView's workspace directory,
  per-project storage, selection quick actions and tool paths are not
  ported, nor are accent colour, fonts and sizes. The send key is saved
  under the WebView's words (`enter`, `modifier_enter`) but in the host's
  settings, so the WebView's own choice, kept in its browser storage, is not
  read. Return sends through a key binding of the composer's own that
  outranks the input's new line, so it applies to the composer only. "Where
  I left off" reopens the last conversation that was opened, in its project; if that
  project or conversation is gone the app says so and stays where it is.
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
Turn Plan first on and ask for a change: a plan card appears and no file is
written; approve it and confirm the work then runs and Plan first is off. With a
model that reports reasoning, confirm the Thinking box appears and goes away,
and that the answer then has a folded Thought above it that opens to what
it thought, also after reopening the conversation.
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
Start a long run that is not waited for, open the Runs tab, confirm its status
moves on by itself after the turn ends, cancel it and confirm it ends. Have a
turn write a file, confirm it appears in Artifacts for the conversation and for
the project, and open it from there.
Start a turn, switch to another conversation and confirm its status and the
inbox update without pressing Refresh; open it and confirm it becomes read.
Start with an empty data directory and no `WISP_*` variables: Settings opens;
save an invalid form, then a valid one, and send a message without restarting.
Relaunch and confirm the model is still configured. Rename with Enter and with
the button; press Escape in the title editor and in the delete confirmation and
confirm only that layer closes. Pin, archive, find the archived conversation by
a word from one of its messages, restore it, then delete it.
Make two groups; rename one with Enter; move one up and down; put the open
conversation in a group from its bar and take it out again. Open the sort menu
and switch between Groups, Date and None and between Newest and Name; restart
and confirm the choice is kept. Fold a section. Enter select mode, pick one
conversation, Shift-click another further down and confirm the range is picked;
move the picks to a group; pick again and archive, then restore them from the
archive; ask to delete, press Escape (select mode stays), press Escape again
(select mode ends). Start a turn that waits for approval, include it in a batch
delete and confirm the line says it was left and why. Delete a group and confirm
its conversations remain. Copy the open conversation to another project and
confirm both exist; move it and confirm it left this project, arrived read in
the other, and asks before changing files there.
In a conversation with three turns, press Branch under the second reply: a new
conversation opens with two turns, listed under its source in the same group.
In the branch press Edit under your second message, press Escape (nothing
changes), press it again and confirm: one turn remains and the message is in
the composer; change it and send. Open the source and confirm all three turns
are still there. Try to delete the source and confirm it asks for its branches
to be deleted first. Start a turn and confirm Edit and Branch are disabled
while it runs.
Open Memos from the sidebar: write one with a title and one without, pin the
older one and confirm it leads; press Use and confirm its text is added to the
message box after what was there; edit one, press Escape in the editor (the
panel stays, nothing is saved); ask to delete, press Escape, then delete. Switch
project and confirm the other project's memos are its own.
Press the memory button under a reply: the Memory tab opens with its text;
trim it and save it for the project, and confirm it is in today's note. Save
another as a habit and confirm it is listed under Habits in a second project
too. Add a habit by hand, edit it, ask to delete it and press Escape. Open
today's note, change it and save. Turn memory off and on. With a real model,
ask something a habit should shape and confirm it does while memory is on and
does not while it is off.
Press the star under a reply: it fills; press it again: it empties. Save two
messages, open Highlights and use one in the message box. Open Favorites from
the sidebar, from the home page and from the palette; search for a word in one
item and for the project's name; filter by kind; open an item, press Remove,
press Escape (the page stays), press Escape again (the page closes). Open the
conversation an item came from. Have a turn write an image, save it from
Artifacts, delete the file and confirm the figure still shows in Favorites.
Delete a conversation and confirm what was saved from it is still listed.
On the home page press the flask button and choose a folder: a "Wisp
sample project" folder appears there and opens as a project with its
README, data and script. Cancel the folder dialog: nothing is made.
With a real SSH host that has passed its check, open the Files tab:
the host is offered beside This project. Choose it: its home folder is
listed, folders first; go into one and back Up; Reload. Choose This
project: the project's files are back as they were. With a host that has
not been checked, confirm the tab says a check is needed.
Have a turn write a file, open the file in the Files tab and press
Origin: the line names the tool and the conversation, the code is shown,
and Open the conversation goes there. For a file you made yourself it
says none is known.
In the Files tab go into a folder and press the terminal button: the
system's terminal opens in that folder. Do the same from the command
palette. With the project's folder moved away, confirm a message says it
is not there.
Have the model start a run on this computer that prints more than a
screenful, open the Runs tab, press Output and then Whole log: everything
the run wrote is shown, stdout and stderr apart. Press it again while a
long run is still writing and confirm more appears.
In Settings, General, press Check for updates: it reads Checking… and
then says this is the latest version or names a newer one, with Open
releases opening the project's release page. Without a network it says
it could not check.
Open three conversations, one of them in another project: tabs appear
above the conversation from the second on. Click a tab of the other
project: its project and conversation open. Close the tab in view: its
neighbour opens; close a tab that is not in view: nothing else changes.
Delete a conversation and confirm its tab goes.
Start a slow turn, switch to another app and wait: a notification names
the conversation when the turn ends, and another when a tool asks for
approval. Stay in Wisp and confirm nothing is posted. Turn Notify me off
and Play a sound on in Settings, General: only the sound comes.
After a turn that started a run, the run call in the transcript has a line
with the run's state that changes as the run does; Show in Runs opens the
Runs tab with that run's output open.
Open a Jupyter notebook in the Files tab: its text cells read as text, each
code cell shows its run number, its code and what it printed or returned, a
figure is named in its place and an error shows its name and message; Show
as text gives the file's JSON and Show as a notebook brings the cells back.
In the Files tab press the order button: By name becomes Newest first (the
file written last at the top, folders still first), then Largest first, then
By name again; restart and the last order is kept.
In a project, press the button left of the conversation's title: the
sidebar goes and the conversation fills the window; press Cmd+B and it
comes back, again and it goes. Quit and start again: it is as it was left.
In a conversation, type a line, press Shift+Return and type another: the
composer holds two lines; press Return and both are sent as one message.
In Settings, General, choose ⌘Return under Send with: the empty composer
says so; Return now starts a new line and ⌘Return sends. With a Chinese
input method, type pinyin and press Return: the letters are committed and
nothing is sent. Choose Return again before going on.
In Settings, General, choose 中文: the sidebar, the home page and the
composer read in Chinese at once, without a restart; choose English and
they read in English; choose System on a Chinese system and they read in
Chinese. Restart and confirm the choice was kept.
On a project you can lose (a folder made for the test), press its remove
button and then Delete folder too: the question names the folder's full
path, its file count and size. Type part of the name: the delete button
stays off. Press Escape: nothing is deleted. Open it again, type the
name, delete: the folder is gone from disk, the project and its
conversations are gone from the home page, and the neighbouring folders
are untouched. Register a folder that holds another project's folder and
confirm deleting it is refused with the reason.
After a turn, click Context N% by the composer: the dialog lists
Instructions first, then your messages, the answers and each tool result,
with about how many tokens each takes and its share. Open the
Instructions row and read the system prompt; open a tool row. Press
Escape: only the dialog closes. In a conversation long enough to have
been compacted, confirm a Summary row stands where the older turns were
and the line under the title says so.
Have a turn run Python code (the bundled worker and a Python are
needed): a "Python … · ready" chip shows above the composer, and reads
running while code runs. Point at it for its path and memory. Press
Stop: it reads stopped; ask the model to use the variable it had set and
confirm the reply says the interpreter was stopped. Press Dismiss: the
chip goes, and the next code call starts a fresh interpreter.
In Settings, Experts, add an expert with a name and instructions; a
name already in use is refused. In a new conversation choose it from the
composer's Expert menu, send a message, and confirm in What the model is
sent that the Instructions row ends with the expert's section; the menu
is then off and names the expert. Remove the expert in Settings: the
conversation still carries its instructions. Press Escape with the form
open: only the form closes.
In Settings, Quick actions, add one with a name and a request ending in
"Sample: "; a name already in use is refused. In a conversation type a
few words, open the composer's quick actions menu (the bolt) and choose
it: the request follows what was typed, the caret is at its end, and
nothing is sent. Choose it again: it is not written twice. Press Escape
with the menu open, then with the form open in Settings: each press
closes only that. Remove the action: it leaves the menu.
On the home page press the clock: Scheduled tasks says there are none.
Press the Weekly report template: the form opens with its prompt; choose
a project, Daily and a time a minute ahead, and save. The task is
listed with "Next:" at that time; within half a minute of it a
conversation named after the task appears in the project (refresh the
list) and the task's line says when it last ran; clicking that line
opens the conversation. Press Run now: another conversation is made and
"Next:" does not move. Switch it off: it reads Off. Give "25:00" as the
time in a new form: it is refused. Press Escape with the form open:
only the form closes. Delete the task: the question has to be answered.
In a project press Papers: the page says there is no paper yet. Press
New paper, give a title and save: it shows with the version "v1 ·
Initial submission" (初次投稿 in the Chinese interface). Add a figure: it is listed with "No source yet".
Press Add evidence, choose Runs or Project results, type why and click
one: it appears under the figure with the reason, and the summary line
counts it. Remove the evidence, change the figure's title, remove the
figure. With a form open press Escape: only the form closes; press it
again: the page closes. Rename the paper, then remove it: the question
has to be answered first.
Send `/timer 1m say the time`: a note confirms it, the Scheduled tasks
page lists it as a conversation's timer, and within a minute and a half
the prompt arrives in the same conversation as a turn. Send `/timer
off`: a note says one was cancelled. Send `/timer soon`: it is refused
and the text comes back.
After a few turns send `/compact`: a note says how many tokens there
were before and after, a file appears under `.wisp/history/`, and What
the model is sent is shorter while the transcript is not.
Ask the model to write a new file and to edit an existing one, with
Full access on. Point at the message, choose to go back, and press Go
back and undo its files: the new file is in `.wisp/trash/`, the edited
one reads as before, the message is back in the composer and a note
lists both. Repeat after changing the edited file by hand: it is left
as it is and the note says why.
Preview a `.csv` file: it shows as a grid with its header in bold, and
Show as text switches to the file's text and back.
Preview a file in Files: Copy path puts its full path on the clipboard,
Attach adds it to the message as a chip, Show in the file manager and
Open with the system's application do what they say.
In a conversation type `/trajectory` and send: the trajectory opens and
nothing is added to the transcript. Send `/btw why did that fail?`: the
Aside tab opens and the question is asked there. Send `/fork`: a
branch opens under the conversation. Send
`/data/reads.fq looks odd`: it is sent as a message. Press the "/"
button, then Escape: only the list closes.
In Settings, Browser, the line reads Off. Switch it on: it reads
"Waiting for the extension to connect" (or says the port is taken while
another Wisp runs). Load the extension from the folder the page names:
after Refresh the line shows its version and the pages open. Add
example.com to the blocked hosts with a reason, and "not a host": the
second is refused and stays in the field. In a conversation ask for a
page to be opened and read: each tool asks first; the tab opens in your
browser, and the answer quotes the page. Ask it to read a tab you
opened yourself: it is refused. Ask it to open the blocked host: it is
refused with the reason. Switch the tools off: the next turn no longer
has them.
In Settings, Workflows, two are listed with their tasks and stages. Press
New workflow, give a name and a goal, add a second and a third task and
mark the third as after tasks 1 and 2: saved, it reads "3 tasks · 2
stages · up to 2 independent". Change it and remove the first task:
the remaining dependency follows the task it pointed at. Leave a title
empty: it is refused and the form stays. Press Escape with the form
open: only the form closes. With a project open press Run: Settings
closes and a conversation named after the workflow shows the goal and
the tasks under Stage 1 and Stage 2. Remove the workflow: the question
has to be answered.
In Settings, Remote access, switch it on with nothing filled in: it is
refused. Give a relay address that is plain http on another machine: it
is refused. With a relay of your own running, give its https address
and token and switch it on: the line reads Connecting, then Connected,
and a code shows. Copy the link and open it in a browser: the projects
are listed and the line counts one browser. Press Reset code, then
Escape: only the question closes. Reset it: the browser is dropped and
the old link no longer works. Give a wrong token: the line says why it
is not connected. Switch it off: the line reads Off. Confirm with
`sqlite3` that neither the token nor the code is in the settings table.
In Settings, Plugins, press Install from a folder and choose a folder
that is no plugin: it is refused and nothing is listed. Choose a plugin
folder: it is listed with its version, skills and any MCP servers, off.
With a project open switch it on, start a conversation and ask which
skills there are: the plugin's are among them; another project does not
have them. Press Remove, then Escape: only the question closes. Remove
it: it leaves the list, and the folder it was installed from is
untouched.
In Settings, Decision model, turn it on with TypeSafe Jev and no key and
press Save: it is refused. Choose StartLux (self-hosted): the address,
model and timeout change to that service's. Press Test connection with
nothing listening: the error names the address; with a server running
it names the model that answered and how long it took. Save: the page
says it is in use, and What the model is sent lists `map_items` with a
`decide` step from the next turn. Type a key for a hosted service and
save: the field empties, the page says a key is saved, and Forget key
removes it.
In Settings, Pet, turn the pet on without a folder: it is refused. Choose
a folder that holds a version 2 pet and turn it on: its name shows on
the page and the pet idles in the transcript's lower right corner. Click
it: it waves once. Send a message: it works while the turn runs, waits
while an approval is open, and jumps when the turn ends. Turn it off:
it goes.
In a conversation with results press Research archive: the dialog says a
draft is being written, then shows a title and a report to edit and the
scripts that go with it. Change a sentence and press Freeze: the dialog
shows the frozen report and where it is; `.wisp/research-archives/<id>/`
holds REPORT.md with your change and the scripts; the composer no longer
sends in this conversation, and a new conversation works as before.
Press Escape on a draft: nothing is written.
Switch Review turns on in the composer and have a turn run a tool: when
it ends the line above the composer reads Reviewing the answer…, then the
result. Press See: the findings open; Ask Wisp to correct puts them in the
composer. Send a short message: no review follows and the old result is
gone. Put a result away with its close button. In Settings, Conversation,
turn Review turns on and confirm a new conversation starts with the
switch on while one you switched off stays off; choose a Reviewer model
and confirm the Review dialog names it.
Under an earlier answer press its review button: the dialog is titled
Review of one turn and what it reports does not draw on later turns.
In a conversation where a tool ran, press Review: the dialog says it is
reading, then lists what was found, each with its claim, evidence and
fix, or says nothing was found. Press Ask Wisp to correct: the dialog
closes and the corrections are in the composer, after anything already
typed, unsent. Press Review while a turn runs: the button is off. Open it
and press Escape at once: only the dialog closes. Confirm the transcript
is unchanged by a review.
Press Save as skill in a conversation: the request is in the composer and
nothing is sent; with a draft in the composer the request follows it; a
second press adds nothing. Send it and confirm the model reads the
`skill-creator` skill.
Ask something that makes the model call `explore`, then open the Agents
tab: the helper is listed with its question, reads Running until it
reports back, then shows its findings; the arrow goes to its row in the
conversation.
Open the Aside tab in a conversation with an answer in it, ask what the
answer was, and confirm the reply is about that conversation while the
transcript is unchanged. Ask a second question that leans on the first
("and why?"). Switch to another conversation: its thread is its own; switch
back and the first thread is still there. Clear empties it. Ask while a
turn runs and confirm the turn is undisturbed. Press Escape with the caret
in the question field: the panel closes.
After an answer, wait a moment: up to three questions show above the
composer. Click one: it is in the composer, not sent. Send it and confirm
the list is gone while the turn runs and a new one follows the answer. Put
the list away with its close button and confirm it stays away after
switching conversations and back. In Settings, Conversation, turn Suggest
what to ask next off: the next answer is followed by no list and the model
endpoint receives no extra request.
In Settings open Conversation: type a letter into Steps per turn and save (it
is refused), then 50, and confirm it is still 50 after a restart; turn
compaction off and on; move the threshold to both ends. Open Permissions:
choose Full access and confirm a new conversation starts in it while an
existing one keeps Ask first. Let a tool ask, answer Always in this project,
and confirm a second conversation of the project is not asked, another
project's is, and the tool is listed in Settings; take it back and confirm
the next conversation is asked again.
Open Skills from the home page and from inside a project: the bundled skills
are listed, and the project's own only in the second case. Search by a word
from a description and by a tag. Turn one off, confirm the count says so and
that the add-context box no longer offers it; turn it on again. Add a skill
folder to the project, press Reload and confirm it appears.
Open Connections: unfold a connector and confirm its tools and links are
shown; search by a tool's name; turn one off and confirm the count of tools
drops. With a real model, ask for a PubMed search and confirm a connector tool
runs without an approval; turn PubMed off and confirm the model no longer has
it.
Send a message, open Usage and confirm today's bar, the model's share and the
project's totals moved; hover a bar for its day and count. Open Storage, press
Show, and confirm the folder that opens is the one named.
Open Credentials: save a contact email and confirm the field empties and says
Saved; restart and confirm it is still Saved; press Forget. With a real NCBI
key saved, run several PubMed searches in a row and confirm none is refused
for rate.
In Connections add an MCP server over HTTP and one as a local program (for
example with a quoted argument); press Test on each and confirm the tools are
listed, or the reason when it fails. Ask the model to use one of the tools:
it asks first, then runs. Switch the server off and confirm the model no
longer has the tool. Change its name and confirm Test lists the new names.
Press Remove, then Escape (Settings stays), then remove it.
Open Environments: add an SSH host with a name and an address and confirm it
is listed as `user@address`; press Add again, then Escape (the form closes,
Settings stays). With hosts in `~/.ssh/config`, add one from the list and
then the rest with Add all; a `Host *` pattern is not offered. Press Check on
a host that cannot be reached and confirm it says so with OpenSSH's reason;
on a reachable host confirm it lists the system, processors, GPU, scheduler
and interpreters. Choose a host under "New work runs on", ask the model
where it would run a job and confirm it names that host; start a run on a
checked host, and confirm one on an unchecked host is refused. In a
conversation open the menu beside Effort that names where it runs: choose a
host and confirm the next message's runs start there while another
conversation still follows the default; press Escape with the menu open and
confirm only the menu closes. Press Remove,
then Escape, then remove the host and confirm new work is back on this
computer.
Open Network: save a model route that cannot be reached (for example
`http://127.0.0.1:9`) and send a message: it fails and names that route.
Write `none`, save, and the next message is answered. Save an address with a
password in it and confirm it is refused, names the field, and keeps what was
typed. Give the code route an address and have the model run
`echo $HTTPS_PROXY`: the command prints it. With a real proxy, set the
connector route and run a PubMed search, and press Test on an HTTP MCP
server. Set a Python package index, ask the model to install a package and
confirm it uses the index. Restart and confirm the page still shows what was
saved.
Have the model write a figure into the project and show it with Markdown
image syntax: the picture appears under the answer, and is still there after
the conversation is reopened. An image link to a web address shows nothing.
Drag two files and a folder from the system's file manager onto an open
conversation: the pane is tinted while they are over it, the two files
become chips on the message being written, and a line says one item was not
attached.
In Files open a PNG or a JPEG: the picture is shown, with its size above
it, and Escape returns to the listing.
In Files make a folder and, inside it, a file; try a name with a slash and
confirm it is refused and the field keeps what you typed. Rename the folder
with the pencil. Press the trash on it, then Escape (the question goes, the
panel stays); press it again and confirm: the folder leaves the list, the
panel says where it went, and it is there under `.wisp/trash` in the project
with its file.
On the home page press the calendar: the month is a grid, each day that
has records says how many, and today is chosen with its records listed
below. Choose another day; leave a project out and confirm the counts
drop; go to an earlier month and back. Press a row that came from a
conversation and confirm it opens in its project. Press Escape with the
calendar open: it closes.
In a project press Journey: this month's records are listed under their
days, newest first. Type a word in the filter and confirm the count says
how many of them are shown. Go to an earlier month and back with the
arrows; the later arrow is off in the present month. Press Add a record,
choose Finding, give it a line and save: it is the newest record of today.
Save one without a title and confirm it is refused and the form stays.
Press a row that came from a conversation and confirm that conversation
opens. Open Journey again and press Escape: the page closes.
On a project's card press Export, choose a place, and confirm a zip is
written and the home page says where. On another data directory (or after
removing the project from Wisp) press Import on the home page, choose the
zip and then a folder: the project appears with its conversations, and its
files are in a new folder there. Import the same zip again and confirm it
is refused as already present. Open the zip in the WebView desktop's
import, and one of its exports here.
In a project press Capabilities: the counts match the Settings pages, the
tools on this computer are listed with where they were found, Settings on a
line opens that page, and Escape closes the dialog.
In a conversation that ran a tool press Trajectory: the totals are at the
top, each turn is headed by its number, time and cost, and the steps read
You, Tool, Model call, Wisp in the order they happened. Click a tool step:
its input and result open under it; click again to close. A failed tool is
marked failed. Press Escape: only the dialog closes. Open it from the
command palette as well, and in a conversation with nothing asked yet
confirm it says so.
Press Cmd/Ctrl+Shift+N: a second window opens a little down and to the
right of the first, on the home page, with the same projects. Open a
different conversation in each, send a message in one and confirm the other
is undisturbed; close one and confirm the other stays.
Press Export on a conversation, choose a place, and open the file: it has
the title, every message in order and each tool call; cancel the dialog and
confirm nothing is written.
In a conversation where the model ran commands open the Notebook tab of the
side panel: each run is a cell with its output, and a code block that was
only shown is marked so. Press the arrow on a cell and confirm the
transcript scrolls to it; press the star and confirm the code is in
Favorites; press it again to take it out.
In a conversation with several messages press the outline button in the
header: your messages are listed in order; choose the first and confirm the
transcript scrolls to it; open the outline again and press Escape, and
confirm only the outline closes.
In a conversation press Cmd/Ctrl+F and type a word that appears several
times: the count is shown, the newest match is marked strongest, Return and
the up arrow go to earlier ones and around again, and Escape closes the bar,
removes the marks and leaves the conversation as it was. Type a word that
does not appear and confirm it says so.
Start a turn that takes a while and, before it ends, write two messages and
press Queue for each: both are listed above the composer and the composer is
empty again. When the turn ends the first is sent by itself, and the second
after that. Queue one more, take it back with the pencil and confirm it is in
the composer again; queue it and drop it with the cross. Queue a message
during a turn that ends with a question card and confirm it waits until you
answer or press "Send the next one". Start a turn that waits for an approval,
queue a message and press its lightning button: it leaves the queue; approve,
and the message shows after the tool's result with the answer to it below,
in the same turn.
On a project's card press Project settings: clear the name and Save (it is
refused and the dialog stays), then give a name, a description and a rule
and Save: the card and the recent conversations show the new name, and
`.wisp/WISP.md` in the project holds the rule. Send a message in a
conversation that was already open and confirm the model follows the rule;
empty the rules, save, and confirm the file is gone. Press Escape with the
dialog open and confirm only the dialog closes. Inside a project, find
Project settings in the command palette.
Press the book on the home page, and Documentation and Report a problem in
Settings and in the command palette: each opens in the system's browser, the
first two the tutorials, the last the issue tracker, and the app stays where
it was.
In Settings type a word in the search field above the pages, for example
"proxy" or "ssh": only the pages about it stay listed, Return opens the
first, and a word no page is about says so. Close Settings and open it again:
every page is listed.
Open General: choose Dark, Light and System and confirm the whole window
changes at once; with System chosen, switch the system's appearance and
confirm the window follows, and with Dark chosen that it does not. Choose
Where I left off, open a conversation, quit and start again: the conversation
is open. Choose Home page and confirm the next start opens on Home.
Restart and confirm persisted messages. Close during a running tool and verify
worker cleanup. Check missing credentials and invalid workspace errors. Validate
each target OS; a macOS build does not establish Windows/Linux behavior.
