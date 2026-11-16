# GPUI desktop migration

The desktop direction is one Rust/GPUI frontend for macOS, Windows and Linux.
The SwiftUI/WinUI desktops and their dedicated packaging, release, design-sync
and parity-ledger jobs are retired. iOS and Android remain. Shared `native_*`
Rust services, DTOs and fixtures remain because mobile, Tauri and server hosts
still consume them.

`apps/gpui` implements the first slices, not full replacement parity. The
[parity ledger](gpui-parity.md) lists every WebView feature, what GPUI has, and
the order the rest is ported in. How it should look, and where each rule lives
in code, is in the [design language](gpui-design-language.md). WebView
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
Bring over on the home page copies the WebView desktop's projects, their
conversations and the settings that mean the same here from its database,
which it only reads.

## Implemented slice

| Surface | Behavior |
| --- | --- |
| Home | Opens first: every project and the newest conversations across them, with status |
| Projects | Persisted list, New project (a name, a description and the standard research layout, then where its folder goes: a folder named after it is made there), native folder picker, reopen registered roots, running and waiting counts, star to keep first, remove from Wisp (files stay); Project settings for the name, a description, and the rules the model follows in the project |
| Look | Wisp's light and dark palettes, following the system or fixed to one in Settings; the shared icon set |
| Conversations | Create, select, restore messages, running / needs-you status |
| Conversation management | Rename, copy the title, pin (shown first), archive and restore, delete with confirmation, and save what it sends the model as a JSON debug request |
| Groups | Named groups per project: create, rename, reorder, delete (conversations stay); put a conversation in one from its bar; fold a section |
| Sort and group | The list by groups, by last activity or flat, newest first or by name; remembered across launches |
| Select mode | Pick conversations by click, Shift-click for a range, or all; move them to a group, archive or restore them, or delete them after one confirmation; a line reports what changed and why anything was left |
| Copy or move | Copy or move the open conversation's transcript to another project; a move can take its artifacts along (with their files), after a preview of what goes and what stays because another conversation uses it or it was changed since |
| Edit and send again | Under a message of yours: after one confirmation the conversation goes back to just before it and its text returns to the composer |
| Branch | Under a reply: a new conversation holding everything through that turn, opened at once and listed under the one it came from |
| Search | Titles and message text across the project, archived conversations included |
| Command palette | One box over everything: projects, files by name in every project, conversations by title or message text, and the window's commands, the WebView's among them (appearance, text and code size, the side panel's tabs, import and export, skills, updates, the first-run guide); arrow keys, Return, Escape |
| Large pictures | When the model asks to look at a picture larger than it takes, a question of its own asks whether to send a smaller temporary copy (the file is not changed): Resize and continue, or Cancel, for that once only |
| Privacy mode | ⌘⇧H (or the palette's Privacy mode) chooses projects to hide while the screen is recorded or shared: their cards, their conversations in the recent list and their rows in the palette are left out until Restore all. The choice is the WebView's, kept in the store, so both desktops honour it |
| Pictures in answers | When an answer shows a picture of the project with Markdown image syntax, the picture is drawn under the answer, and so are the pictures its turn wrote with `write` or `edit` that it does not show (under the turn's last answer); clicking one opens it in the Files tab |
| WebView projects | Bring over on the home page (or the palette's command) lists the WebView desktop's projects from its database, found where the WebView keeps it or chosen, and read without being changed: each with its folder and its conversations, the ones already here or whose folder is gone held back. The ticked ones come in one press, and a line says how many came and why any did not: one kept in the WebView's database is copied with its conversations and goes on using its folder where it is, one that keeps its own database in its folder is opened there, so both desktops share it; a starred one stays starred, and a copied one keeps its own settings and its conversations'. Under Settings the WebView's own settings that mean the same here are offered by group (network routes, skills switched off and tags, experts, ACP agents, command hooks, your credentials' names and more), each only while none of it is set here, so nothing made here is replaced. Under Saved models the WebView's models are listed with their model ids and whether their key comes: one brought over keeps its endpoint, limits and route, its key goes from the keychain to the keychain under this desktop's name, and the WebView's everyday model becomes the default while there is none (not while `WISP_*` variables set the model) |
| Project archives | Export on a project's card writes the project, its records and its files, as one zip; its folder-package button writes them as a folder another computer opens in place, in a new folder named after the project. Import on the home page takes either: a zip is put in a folder you choose, a folder package is opened where it is. While either runs, a line on the home page says the step, the files and bytes done of all, and the file it is at |
| Project sync | Project settings, Sync: Sync now publishes the project through a relay or a shared folder (set in Settings, Remote access) or brings in another device's changes, Copy device code, and after a conflict Use the other device's copy or Keep this device's copy, each asked first; a project in a cloud-drive folder can save itself there instead. Join a project from another device on the home page takes a device code and a folder |
| Capabilities | A dialog for the open project: how many skills, connectors, MCP servers and SSH hosts are on or usable, the state of memory, and which command-line tools were found on this computer |
| Follow-up questions | After an answer, up to three questions you could ask next show above the composer; picking one puts it in the composer to send or change, and the list can be put away |
| Sample project | The home page's flask button makes a small sample project in a folder you choose and opens it: made-up counts for twelve samples, a script, and a README of things to ask. Its play-file button opens Example conversations: the research recorded with Wisp that ships with it, read only, and Copy to a project, which adds one to a project as a conversation to go on with |
| Host folders | With an SSH host saved, the Files tab offers it beside the project: its folders are listed, to walk through; a file's download button copies it to a place on this computer you choose, and Send files here… copies files of this computer into the folder shown, each as a run of the open conversation whose progress the transfer tray shows; nothing there is replaced |
| File origin | A file's preview has Origin: which tool call last wrote it, with the code it ran, the project files it read and the conversation, which it can open |
| Terminal | Terminals drawn in the window, in a dock at the foot of the conversation (Terminals in the top bar shows or hides it; hidden, they keep running): the Files tab's terminal button (or the command palette) opens one in the folder being browsed, + one in the project's folder, the host button one on a saved SSH host, and Terminal on a host's row in Environments or over a host's folder in Files one there; several at once as tabs, each closed with its ×; the dock's last button opens the system's terminal instead |
| Updates | Wisp looks for a newer release when it starts (unless turned off) and shows a note only when there is one; Settings, General, has Check for updates and the At start switch, says whether a newer release has been published and opens the release page |
| Tabs | The conversations you open in a window line up as tabs above the conversation, across projects; click one to go back to it, close one to drop it from the strip |
| Sidebar | The project's sidebar hides for more room and comes back with the button left of the conversation's title or Cmd/Ctrl+B; it stays as it was left, across restarts. Cmd/Ctrl+Shift+F brings it back if hidden and puts you in its search, which finds conversations by their title or by any word in their messages |
| Notifications | When a turn ends or a tool waits for approval while Wisp is in the background, a system notification is posted and, if chosen, a sound is played (Settings, General); a recent conversation that is not open is announced too when it stops working |
| Language | Settings, General, chooses the interface language: System (Chinese on a Chinese system, English otherwise), English or 中文 |
| Delete a project's folder | Removing a project offers Delete folder too: after you type the project's name, the project is removed and its folder is deleted with every file in it |
| What the model is sent | Clicking the context percentage by the composer (or the command palette) lists the conversation's working set: the instructions, the summary that stands in for compacted turns, and each message with about how many tokens it takes; above them, the same set by kind (system prompt, rules, skills, sub-agent definitions, conversation), with the core's estimate and each one's share, as the WebView's context usage splits it, tool definitions excepted |
| Interpreters | The Python and R interpreters a conversation keeps between tool calls show above the composer with their version and state; Stop ends one, Dismiss takes a stopped one off the list; Console opens one with its variables, code of your own to run (printed output and plots) and Restart, and the command palette starts the conversation's Python or R console |
| Experts | Settings, Experts, keeps personas (a name and instructions); the composer's Expert menu starts a new conversation as one, and its instructions hold for the whole conversation |
| Quick actions | Settings, Quick actions, keeps requests you make often under a name; the composer's quick actions menu puts one into the message box, after whatever is already typed, to finish and send. An action can instead start a workflow of the library: from the menu it is drafted as agents on what is typed (its request, if any, told to every task first) in a new conversation that opens on its Agents tab |
| Undo a turn | Going back to the latest message can also undo its files: a file the turn edited gets its earlier content, a file it created goes to the project's trash, and a file changed again since is left and named |
| Table preview | A `.csv` or `.tsv` file is previewed as a grid of its first 200 rows and 30 columns, with a switch to its text |
| File actions | A previewed file's path can be copied, the file attached to the message, shown in the file manager or opened with the system's own application |
| Slash commands | A message that is only a command does something instead of being sent: `/review`, `/trajectory`, `/context`, `/btw <question>`, `/plan`, `/save-as-skill`, `/skills`, `/files`, `/upload`, `/memo`, `/remember`, `/archive`, `/compact`, `/timer 30m <prompt>`, `/fork`, `/rewind`, `/permission`, `/share`, `/help`; typing "/" where a word starts, or the composer's "/" button, lists them with the project's skills, narrowing as you type |
| Browser | Settings, Browser, switches on a bridge to the Wisp extension in your own Chrome or Chromium; conversations can then open a tab, read it and run a script in it, only in tabs they opened themselves, and hosts can be blocked or preferred |
| Workflows | Settings, Workflows, keeps templates of a goal broken into tasks, each saying which earlier tasks it needs and what it may do as an agent, with an approval policy; Run starts a conversation in the open project and gives the model the tasks stage by stage, and Run as agents drafts them as a batch of agents in a new conversation, approved or discarded in its Agents tab |
| Chat apps | Settings, Chat apps, binds a WeChat bot by scanning a QR code (and a second one for the research assistant), and a Feishu (or Lark) bot by scanning to make its app or by entering an App ID and App Secret, with its owner confirmed here; the owner's messages then run in the project chats go to and the answer comes back (on Feishu as a card that shows the turn's progress), with `/project`, `/session`, `/new`, `/status`, `/stop` and, in WeChat, approvals answered by code (`/approve`, `/reject`); and Wisp Lab: found a lab on a wisp-relay or ask to join one with an invite code, see the members (the leader approves, refuses or removes them and makes invite codes), choose the project that answers other members' agents, replace the relay token, leave |
| Remote web access | Settings, Remote access, opens this desktop's projects in a phone or a browser through a wisp-relay of your own: it dials out (no inbound port), shows whether the tunnel is up and how many browsers are connected, and gives a link and a code that can be copied or replaced; the same page turns on the StickS3 desk display's listener (one IPv4 address and port of this computer, a token to show, copy or replace), which shows the device what the agent is doing and the pet, and opens the conversation the device asks for |
| Plugins | Settings, Plugins, installs a plugin from a folder in the Claude plugin layout (`.claude-plugin/plugin.json`, `skills/*/SKILL.md`) into the app's data without running anything in it, lists what each brings, switches one on for the open project, whose conversations then have its skills, and removes it |
| Decision model | Settings, Decision model, sets up one fast endpoint (TypeSafe Jev, StartLux-Decision hosted or self-hosted, or a custom System One server) with its key, thinking mode and timeout, and tests it; while one is in use the model's `map_items` tool, which runs one step over many items, also offers its `decide` step, and an MCP server whose tools are Cua Driver's also gives `desktop_autopilot`, which drives the computer step by step with the decision model (asking before it runs) |
| Pet | Settings, Pet, turns on an animated companion drawn from a folder you choose (pet.json and a sprite sheet, Codex-compatible version 2); it stands in the transcript's corner and shows whether the conversation is working, being reviewed, waiting for you, done or failed |
| Research archive | A conversation's Research archive button (or the command palette) has the model draft a report of its findings with the scripts behind them; you change the draft and freeze it, which writes it into the project and makes the conversation read-only; Continue research then opens a new conversation, linked to the archive, that starts from its report |
| Review turns | The composer's Review turns switch has each turn that used a tool, or said a good deal, reviewed when it ends; the result shows above the composer, and See opens the findings |
| Review | A conversation's Review button (or the command palette) has the model read the conversation as a reviewer and lists claims its own tool output does not support, each with the evidence and a fix; Ask Wisp to correct puts the corrections in the composer |
| Save as skill | Save as skill in a conversation's More menu (or the command palette) puts a request in the composer that asks Wisp to distill the conversation into a reusable skill; you send it, changed or not |
| Trajectory | A conversation's Trajectory button (or the command palette) shows every turn's steps in the order they happened: what you asked, what the model said, each tool with how it went and how long it took, and each model call with its tokens; the totals are at the top |
| Windows | New Window (the app menu, Cmd/Ctrl+Shift+N, or the command palette) opens another window on the same projects, each with its own page and conversation; a project's card or palette row opens it in a window of its own, which opens again at the next start until it is closed |
| Export | Export in a conversation's More menu saves the whole of it as a Markdown file where you choose; Share as HTML (or `/share`) saves what was said as one page to open in any browser |
| Notebook | A tab of the side panel: the conversation's code as cells, with what each run returned; copy, save to favorites, or show it in the conversation |
| Research calendar | A page across every project: a month as a grid with how many records each day holds, and the chosen day's records; projects can be left out |
| Scheduled tasks | A page across projects (the clock on the home page, or the command palette): a prompt is sent into a new conversation of its project every few hours, daily or weekly at a time, while the app is open; each task can be run now, switched off or deleted, and its latest firing opens the conversation it made |
| Paper evidence | A page for the open project (Papers in the sidebar, or the command palette): the papers it is writing, each with its versions and, in a draft version, figures, tables and claims with the project results and runs they rest on and why each was chosen. Finalize… checks a draft for whom the evidence is for (private, restricted or public, with the reviews public needs) and lists what stands in the way, warnings and what the record leaves out; a finding that may stand is kept with a reason; when nothing is left in the way the version is locked under a SHA-256 fingerprint. New version starts a draft from any version with its items and evidence |
| Research mandates | A page of standing goals (the gauge on the home page, or the command palette): each project's agent works on its mandate in rounds on a cadence in one conversation, asks you through the research assistant when it needs something only you can give, and writes reports; answer, run now, pause, report, edit, close or delete from its card |
| Research assistant | One conversation of its own (the robot on the home page, or the command palette) that reads what was recorded across projects and each project's progress, keeps your dated plan, and hands work to a project conversation, whose outcome it reports back; a bar over it shows today's plan to tick off or drop |
| Research journey | A page for the open project: one month of what happened, by day (what you asked, runs, files, decisions, your own records), a filter, and a form to add a record of your own. Each day can have a recap the model drafts from its records, with what was done, found, went wrong and comes next, which you change, confirm or dismiss; Scheduled tasks drafts the last days' recaps each morning |
| Outline | A button in the conversation's header lists what you asked, in order; choosing one scrolls to it |
| Find in conversation | Cmd/Ctrl+F, or the command palette: a bar over the transcript marks every message, and every tool call, that contains the words; Return and the arrows move between them |
| Help | Documentation (home page, Settings, palette) opens the tutorials and Known problems (Settings) the issue tracker in the system's browser; Report a problem (Settings, palette) starts a conversation, in the open project or the built-in one, with the app's version, system and model in the composer, for the model to draft the report and a prefilled GitHub link |
| Settings | A search field narrows the pages to those about what you type. Pages: General (language; appearance: system, light or dark; a light and a dark palette from the WebView's; the size and font of the interface's text and of code; the send key; the workspace folder, used from the next start; whether each new project keeps its own database; notifications, sound and which of the system's sounds; update check; what the app opens on), Network (a route for model requests, for connectors and MCP servers, and for the code the model runs; a conda mirror, a Python package index and a CA bundle the model is told to use), Models (saved models with provider, endpoint, key and limits; presets, a default, a connection test; and subscriptions: sign in to ChatGPT, ChatGPT through Codex or SuperGrok to use their models without a key), ACP agents (programs on this computer that speak the Agent Client Protocol: add, change, remove, and Test, which starts one and shows its name, version and how it signs in, with a button for each way: signed in by the agent itself, or in the system's terminal), Conversation (steps per turn, continuing cut-off answers, automatic compaction and its threshold, suggesting what to ask next, reviewing turns, the reviewer model, the sub-agent model and the picture model for a model that cannot see), Skills (every skill with where it comes from and its tags, search, on or off, and Tags to give it your own tags in place of the ones it came with, kept under the WebView's `skill_tags`), Connections (your own MCP servers: add over HTTP or as a local program, test, switch off, remove; and the bundled literature and biology connectors with their tools and links, search, on or off, and, for the open project, each tool's Allow, Ask first or Deny and a connector's Skip approvals), Credentials (the service keys the connectors use, and keys of your own under an environment variable you name, all kept in the system keychain and given to the interpreters a conversation starts), Environments (Python, R and Node.js installed into Wisp's own data folder with progress, a log and Stop; the SSH hosts work can run on: add, change, remove, add from `~/.ssh/config`, check the connection and see what the host is, and which one new work goes to), Permissions, Hooks (your shell commands run at points of a turn: add, change, switch off, remove; and the open project's `.wisp/hooks.json` to review and trust), Usage (tokens in total, per day over the last thirty, by model and by project, and the skills and MCP tools the model called most, counted from its messages, the first nine and the rest as Other) and Storage (where the data is and how much room it takes) (how new conversations start, the open project's tool approval mode and the tools with a rule, tools that are always allowed); applied without restart |
| Inbox | Conversations in any project that are waiting for you; opens across projects |
| Read state | A finished answer is marked read once it is on screen in the active window |
| Transcript | Virtualized Markdown, selectable text, paginated history |
| Tool calls | One folded line each with outcome and elapsed time; click for the verbatim input and output; interrupted calls stop claiming to run |
| Runs | A Runs tab: runs started by the conversation with kind, status, exit code, command and output tails; a run that has not ended can be cancelled, and is followed until it ends. One that has ended and kept a workspace where it ran can have it deleted after a question (its logs are saved into the project first, and outputs never brought back are not lost without a second question); a cleaned run, or one whose cleanup failed, says so. A run that moves files between machines shows its phase, file, bytes, rate and time left on its card, and while it moves (and a few seconds after) in a Transfers tray above the composer that folds to one line. A finished run on an SSH host has a folder button: a dialog lists what it left there a folder at a time, with a name filter and More… for long folders, and the files and folders chosen (across folders) are brought into the project as the conversation's results, a folder as one archive, or deleted there after a question; the whole workspace can be deleted from it too. In the transcript, a call that started or watched a run shows the run's state now, and Show in Runs opens its output there |
| Artifacts | Files a turn wrote are recorded when it ends; an Artifacts tab lists them for the conversation or the project and opens them, and lists each one's versions, any of which can be saved as a copy from its snapshot (checked against its checksum, up to 512 MB); By folder puts them under their folders, and each one's trail button says which tool call wrote it, with the code and Open the conversation |
| Side panel | Tabs: Files, Runs, Environments (this computer and the saved SSH hosts: attach one to the conversation so its runs may go there, take it away, check it, open a terminal or its folders, and set where the project keeps things there: uploaded data, run folders and the results folder), Artifacts, Notebook, Provenance (the loaded part's tool calls with how each went, opening to what each was given and returned, with Show in the conversation), Agents, Highlights, Memos, Memory, Aside; the open tab is named, the others are icons. − beside the open tab takes it away and + adds one back; the tabs kept are remembered, and opening a tab from elsewhere brings it back |
| Agents | The Agents tab lists the helpers the conversation sent off to explore: what each was asked, how it went, and what it found |
| Aside | The Aside tab (or the command palette) asks the conversation's model a question about the open conversation; the answer shows beside it and nothing is added to the conversation. The latest turns are quoted whole; passages from before them that match the question go along labelled [E1]… and are listed under the answer, with who said them and the words they matched. Ask aside under a message opens it with that message quoted in the question |
| Quoting | Quote under a message puts it in the message box as a quotation after what you were writing, ready to answer; a selected passage is copied with Cmd/Ctrl+C |
| Favorites | A star under any message saves it as it reads now; an image in Artifacts can be saved as a figure. Highlights lists what this conversation saved; the Favorites page lists everything from every project, with search, a kind filter, the item in full, its source conversation and removal after a confirmation |
| Memory | One switch; habits the model is shown in every project; this project's notes by day, which the model can search; add, edit and delete either, and save any message to one or the other, or press Suggest for the reviewer model's draft of what is worth keeping from its turn (the WebView's `turn_memory` prompt), with where it suggests keeping it; a weekly tidy that proposes merging or retiring habits for you to apply or dismiss |
| Memos | Your own notes per project: add, edit, pin, copy, delete after a confirmation, and put one into the message box. The model is never shown them |
| Files | Project file browser, available before the first conversation, with a field that finds the project's files by name and Upload, which copies files you pick into the folder shown (never over a file that is there: a taken name gets `_1`, `_2`…); read-only text and Markdown preview, pictures (PNG, JPEG, GIF, WebP, SVG, BMP), and Jupyter notebooks as their cells with what each printed or returned (a figure is named, not drawn; Show as text gives the file itself); a PDF as its first twelve pages, drawn on the CPU by `hayro` (pure Rust, the same on macOS and Windows; a protected PDF is opened with the system's application), with Show the text for its text and tables; an alignment (Clustal, Stockholm, aligned FASTA) or a FASTA file as its sequences, one per row, residues coloured by kind (amino acids by property, nucleotides by base), a hundred columns at a time, with Show as text; a molecule (MOL, SDF, MOL2, a crystal's CIF with its asymmetric unit placed in its cell, or SMILES laid out flat from the notation, implicit hydrogens left out) as its bonds, double, triple and aromatic ones drawn as such, with its heteroatoms coloured, its name, and which of the file's molecules it is, and a structure (PDB or mmCIF) as the trace of its chains through their Cα (or P) atoms, each chain its own colour, turned by dragging, with Face on and Show as text; Word, Excel, PowerPoint, OpenDocument, RTF and EPUB documents as their text and tables, without their layout or pictures; an HTML page laid out as a document (its text, headings, lists, tables and links; no scripts run and no styles or pictures are loaded, as the app has no network client) with Show the source; new folder, new file, rename, and move to the project's trash; listed by name, newest first or largest first, folders first, in the order last chosen; as rows or as tiles, a picture of at most 16 MB as itself, the first 400 entries of a folder, remembered across launches |
| Composer | Native multiline input. Return sends and Shift+Return starts a new line, or, with the other choice in Settings, General, Return starts a new line and Cmd/Ctrl+Return sends; Cmd/Ctrl+Return always sends. While an input method is composing, Return goes to it. While a turn runs the same key queues the message: it is sent when the turn ends, or given to the running turn at once with the queue's lightning button |
| Plan first | The model investigates and submits a plan as a card; nothing is changed until you approve it, which leaves plan mode and starts the work |
| First-run guide | On a first run a dialog walks through four steps: welcome, what Wisp can do, a model (a DeepSeek key sets up deepseek-v4-flash as the default and deepseek-v4-pro beside it, or a link to Settings, Models for another provider), and the tools on this computer (install Python with uv). Get started or Escape puts it away for good, in this desktop and the WebView one |
| Import from Codex or Claude Code | The import button in the conversation list's header lists the conversations Codex CLI (`~/.codex/sessions`) and Claude Code (`~/.claude/projects`) keep on this computer, newest first, with the first thing asked and the folder it was had in. Import brings one into the project in a group named `codex` or `claude`, with its messages and tool calls; importing it again brings in only what was added since, and leaves a conversation continued here alone. Only this computer is looked at, not SSH or WSL hosts |
| Delegated tasks | The model can hand a batch of dependent tasks to temporary agents and wait for their results; a plan card asks first when a task can change files, run code or use a remote model. Each task works in a watch-only conversation under the one that started it. The Agents tab lists the batches with each task's status, summary and conversation, and stops a running one |
| Subagents | The model can start a subagent: a conversation of its own for one focused task, listed under the conversation that started it, with a bar saying it is watched and Open the parent; its composer is closed. When it ends, a report is added to the parent, whose agent goes on to handle it |
| Explorations | On an answer, Start an exploration from here names one and continues the conversation in its own copy of the project; more candidates of the same round start from the same answer. The source shows a bar with each candidate and Abandon the round; a candidate shows its status, View changes and keep (what it changed, what the mainline changed meanwhile, why it cannot be kept), Discard, Back to the mainline and Abandon the round; when it cannot be kept, Show both folders and Finish by hand |
| Bringing a branch back | In a branch, Bring back in the conversation's toolbar has the model summarise what the branch did since it split off; you read and change the summary, and Bring it back adds it to the conversation the branch came from, which reads it from then on, and opens that conversation, where it shows as a Merged branch result card naming the branch: press it for the summary and Open the branch. A branch is brought back once |
| Approving a plan | When the model sets out a new plan it waits for you: the plan's steps show above the composer with Approve, Turn down, and a box to say what should change; Send feedback (or Return in the box) turns the plan down with your words, which the model reads to revise it |
| Working plan | When the model sets out or updates its plan for a long piece of work, the update is a checklist in the transcript (done, in progress, to do, dropped), and while steps remain a line above the composer says how many are done and which is next |
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
| Shortcuts | Cmd/Ctrl+K opens the command palette; Cmd/Ctrl+P opens it with only its commands, as the WebView's action palette; Cmd/Ctrl+N starts a conversation in the open project; Cmd/Ctrl+, opens Settings |
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

- Models are API models, subscription models, or an ACP agent for a
  whole conversation (see ACP agents). Image or video models are not
  ported.
- Subscriptions follow the WebView (`codex_login.rs`): Sign in with
  ChatGPT opens the browser and listens on `127.0.0.1:1455` for the
  answer, or takes the address the browser ended on when it cannot reach
  this computer; ChatGPT through Codex also offers a code to enter on
  another device, and SuperGrok only that. The OAuth exchanges are
  `wisp-llm`'s and go by the Network page's subscription route. The
  credentials are kept in the system keychain under the WebView's names
  (`chatgpt_subscription`, `codex_subscription`, `xai_subscription`), so
  either desktop's sign-in serves both, and one sign-in serves every
  model of its subscription. Keeping a sign-in adds a model of the
  subscription when there is none (its default model, made the default
  when nothing else is); the model can be changed in the form without a
  key. Before each turn the access token is refreshed when it is within
  minutes of expiring, and the rotated token is written back. Signing out
  keeps the models, marked as waiting for a sign-in. ChatGPT through
  Codex keeps a pool of accounts as the WebView's `codex_accounts.rs`
  does (each under `codex_account:<id>`, their order under
  `codex_accounts`): every sign-in joins it, one account is in use for all
  Codex models, Use switches, Remove asks first and hands over to the next
  account (or signs out when none is left), Check usage asks ChatGPT for
  each account's short and weekly windows, and Import from Codex CLI
  brings in `$CODEX_HOME/auth.json` (else `~/.codex/auth.json`) and
  CLIProxyAPI's `~/.cli-proxy-api/codex-*.json`, the newest copy of each
  account. Usage is asked for on request rather than whenever the page
  opens. Nothing is offered while models come from `WISP_*`
  variables. A broken model chosen for a
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
- Subagents follow the WebView's `subagent_tool.rs` and `dispatch.rs`. A
  conversation's agent gets `dispatch_subagent` (a new subagent, or a
  follow-up to an idle one it started), `subagent_status` and
  `stop_subagent`; only the conversation that started a subagent may use
  them on it. Each dispatch gives the child the parent's execution
  contexts and default compute target, and the configured subagent model
  (or the parent's with `model_tier: inherit`); the instruction arrives
  with a brief saying the researcher only watches. A subagent's own turns
  get none of those tools, nor `ask_user` or `map_items`, and a send from
  the desktop or a chat to its conversation is refused. When its turn
  ends, the parent's model reviews the outcome into a report (a plain one
  if it does not answer within a minute), which is added to the parent
  once that conversation is at rest; the parent then resumes without a
  new message, told what came back and the request it originally
  answered. A child that stopped at its step limit holds
  `dispatch_subagent` in that resumed turn, so the researcher is asked
  first. Dispatching asks first, like other tools that set work going,
  unless the conversation has full access; a subagent's turns keep that
  asking when its parent's turn came from a chat. Not ported: the side
  model's automatic answer to a subagent's approvals (its own approval
  card is answered in its conversation).
- Delegated task batches follow the WebView's `delegation_tool.rs`,
  `dynamic_workflow.rs` and the native path of `delegation_runtime.rs`.
  A top-level conversation's agent gets `delegate_tasks`: a goal, shared
  context and up to eight tasks, each with an id, an instruction, the
  tasks it depends on and the capabilities it needs (reasoning, reading
  the project, also writing it, running code in an execution context,
  an independent review; looking at images when a vision model is set
  up, and making figures with Python or R when the runtime is
  installed). The batch is resolved by the same capability policy as the
  WebView's and kept in the same agent-workflow tables. Independent
  tasks run two at a time and a task runs once what it depends on has
  succeeded, with their results passed in; one whose prerequisite failed
  is blocked. Each task works in a conversation of its own under the
  one that started the batch, named after its id and instruction,
  watched and not written to, on the subagent model (or the
  conversation's own with `inherit`). It has only the tools its
  capabilities grant and uses them without asking; before anything runs
  the researcher approves the batch on a plan card listing each task
  and why it needs approval (changing files, running code, a remote
  model), unless the conversation has full access, and `delegate_tasks`
  is not asked about a second time. A refused batch is not kept. The
  agent gets back each task's status, summary, small data, evidence and
  usage in the same turn, and reads a full result with
  `get_delegated_result`. Stopping the turn stops the batch and its
  tasks. With `background` the agent gets a handle at once and the turn
  ends; the batch runs on (its card's Stop ends it), and when it is over
  its result is added to the conversation once nothing runs there, shown
  as what came back rather than as a message of the researcher's, and
  the agent is resumed to sum it up. A batch the app closed on is marked
  failed at the next start; the desktop then delivers what its tasks
  left, and every half minute delivers any result not yet delivered.
  The window follows a turn the host started in the open conversation by
  itself.
  Batches are not offered inside an exploration, and a frozen project
  lets only read-only tasks run. A task can be done as one of the
  researcher's experts (`specialist_id`), which gives it the expert's name
  and instructions, as the WebView's specialists do; it runs on the
  task's own model rather than the expert's, and
  its card says which expert it was. A batch that failed or was stopped can
  be run again from its card: it is checked against the models and tools
  there are now (and refused if they changed), tasks that succeeded keep
  their results, the others run again in the background, and the result
  is delivered like a background batch's. Before that, each task that did
  not finish shows its token budget with Change: the number typed there
  (0 for no limit) is the task's budget for the rerun, resolved again as
  if the task had asked for it, as the WebView's retry budget is; nothing
  else of the plan changes. A task granted the `delegation` capability
  (approved on the plan card like other grants) may start one batch of
  its own from its conversation, as the WebView's nested delegation does:
  that batch is resolved within what the task was granted (its other
  capabilities, model, permissions, context and budget, never delegation
  again), counts against the root batch's limits, runs while the task
  waits (never in the background, and asking no one), and its tasks are
  named `task/subtask`, work in conversations under the task's, and are
  listed, marked Nested, in the task conversation's Agents tab; it runs
  again only with the task that started it. In a clean Git project a task
  can be `isolated`, as in the WebView: it works in a temporary Git
  worktree of its own (its card says Own worktree); what it leaves running
  there is stopped, the artifacts it recorded are kept with the project,
  and when it succeeds its changes are cherry-picked into the project
  after a conflict preflight, otherwise (or on a conflict) they are kept
  as a patch artifact of its conversation and the project is left as it
  was; the worktree is removed either way. Not ported: an expert's own
  model, the other budget limits (tool calls, cost), ACP executors, and
  literature and web research capabilities.
- Method search follows the WebView's `method_search.rs`. Its first half
  is here: `prepare_method_search`, offered to a conversation's agent and
  to delegated tasks that may run code, freezes a Python target symbol,
  its evaluator and inputs (and an optional final-verification file) as
  artifact versions, runs the baseline the asked number of times with the
  local Python and its `wisp_evaluate:` protocol, proves a candidate is
  reachable with a failing sentinel, and returns the frozen spec and audit
  report ids. The search coordinator follows `method_search_coordinator.rs`
  too: a durable `method_search` Run that proposes candidates with the
  model (the host supplies the provider), evaluates each in a fresh
  workspace, keeps its budgets of candidates, cost and wall time, pauses,
  resumes and cancels at candidate boundaries, runs the final
  verification and promotes the top candidates as artifacts. The host
  answers for its Run as the WebView's commands do: its details (the
  frozen contract and audit, candidates, strategies, outputs), Start once
  the contract was read (refused without the model the starting batch
  step approved, and inside an exploration), Pause at the next candidate
  boundary, Resume from the checkpoint, and Cancel (also the Runs tab's
  Cancel); candidates are proposed by the conversation's sub-agent model,
  or its own, capped at 8192 output tokens. A search the app closed on is
  paused at the next start, and one running at shutdown pauses. A
  delegated batch starts a search as the WebView's run activity: a task of
  `task_kind: "run_activity"` names a local `method_search`, the task that
  prepares the contract (a direct dependency whose `output_schema`
  requires `method_search_spec_artifact_version_id`) and bounded budgets
  (1–50 candidates, at most a week of wall time, an evaluator of at most
  300 s, a cost ceiling). The step is sealed to this computer's execution
  context and the batch's default model, so such a batch is always
  approved first; when the step is reached its Run waits in Draft until it
  is started, and when the search ends the batch carries on from it and
  its result reaches the conversation. In the Runs tab a method search
  has a Search card, as the WebView's: its frozen contract (objective,
  target, evaluator, metric and direction, the audited baseline and noise
  floor, whether a candidate is reachable, budgets, protected files,
  guardrails, constraints and whether a held-out check follows), Start
  for a draft, Pause while it runs and Resume once paused (Cancel is the
  run's own), its progress (phase, best score, candidates scored and
  failed, cost, strategy, last checkpoint), the files it promoted (the
  selected method, the top candidates, the candidate history and the
  verification report, each readable in place from its checksummed
  snapshot), every candidate with its rationale or error, the strategies
  it leans on, and the durable record's state. The card reads the host
  through the shared `wisp-dto` details and follows its run as the
  conversation's snapshot moves on.
- Explorations follow the WebView's `exploration_commands.rs`,
  `exploration_promotion.rs` and `exploration_workspace.rs`, whose
  services and tests are carried over. The first candidate of a round
  freezes one checkpoint: the conversation up to the chosen answer, the
  project's records, and a snapshot of its files as they are (small
  regular files copied into a content-addressed store under the data
  folder, the rest referenced and listed in the copy's
  `.wisp/exploration-references.json`; symlinks are not followed). Each
  candidate gets its own copy of the files under the data folder's
  `explorations/` and its own conversation, whose turns work in the copy,
  read only inside it, may not name the mainline's folder and are told so.
  The source conversation is frozen, and the project's other
  conversations keep to tools that change nothing until the round ends;
  rewinding, branching, moving, archiving or deleting the source or a
  candidate is refused meanwhile, as are project sync and removal.
  Keeping one is fast-forward only: it is refused when the mainline
  changed what the candidate changed, its records moved on, or a
  referenced file changed, and the preview's guard must still match;
  otherwise its files are applied through a journal that is rolled back
  (never over an outside edit) or finished at the next start, its
  conversation continues the source conversation, and the other
  candidates go. Discarding the last candidate or abandoning the round
  leaves the mainline as it was. Candidates are not listed in the sidebar
  (the session lists leave them out, as the WebView's do): they are opened
  from the bar over the source. A conversation run by an ACP agent, a
  branch, or a candidate cannot start an exploration. The Files tab shows
  the mainline's files, not a candidate's copy.
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
  missed. A project found there has a button that opens it in a new
  window; while a project is open, a file of that project or another
  conversation has one that adds it to the message being written.
- The desktop now loads the skills that ship with Wisp (`skills/` in a
  checkout; an installed build has to name its resource directory) after any
  directory in `WISP_SKILLS_PATH` and your own skills in `~/.wisp/skills`
  (the WebView's personal folder, which wins over the bundled ones), then
  the project's own `skills/` and `.wisp/skills/`. A headless host reads
  no personal folder. Before, only the last two and the variable were read, so a
  desktop without the variable had no bundled skills. A headless host is
  unchanged. Turning a skill off uses the WebView's list (`disabled_skills`)
  and applies to every project from the next turn: the model's skill tools
  and the add-context box no longer offer it. Per-project enabling is
  not ported.
- The host now gives every turn the bundled literature and biology
  connectors (`wisp-bio`: PubMed, bioRxiv, UniProt and the rest), which the
  WebView has always had. They only read, so they never ask for approval, and
  their schemas are deferred, so they do not fill the prompt. Turning one off
  uses the WebView's list (`disabled_connectors`). They follow the Network
  page's route for connectors. Per-conversation connector grants are not
  ported.
- Your own MCP servers are kept in this database (`desktop.mcp_servers`), not
  in the WebView's list, so each desktop has its own. A server is reached
  over HTTP, with an optional bearer token and headers of your own (a
  header named Authorization is sent in place of the token), or started as a
  program on this computer with environment variables of your own. The
  names are saved with the server; their values are kept together in the
  keychain (`desktop.mcp.<key>.<server>.extra`) and are never shown again: a
  name written with no value keeps the saved one, and a name left out is
  forgotten. OAuth and per-server proxies are not ported. Connections are
  kept between turns and closed when a server is changed, switched off or
  removed, and when the app quits. The model calls a tool as
  `<server>__<tool>`; every such tool asks before it runs, and that question
  cannot be answered "for this conversation" or "always". A server that
  cannot be reached gives a turn no tools and says why in Settings; it never
  stops the turn. MCP Apps are not ported.
- Credentials holds the keys the connectors read (the NCBI key and contact
  email, and the OpenAlex key) and the WebView's two for command-line
  tools: InfiniSynapse (`INFINISYNAPSE_API_KEY`) and SCIMaster
  (`SCIMASTER_API_KEY`). They go to the keychain under this data directory's
  own names (`desktop.service.<id>.<name>`), never into SQLite, are read at
  the start of each turn, given to the interpreters under those names, and
  are not handed back to the window once saved. As in the WebView, saving
  the InfiniSynapse key runs `agent_infini init` with it (refused, with how
  to install it, when the CLI is not found on the PATH or in
  `~/.infini/bin`), and the SCIMaster key is also written into
  `~/.scimaster/config.json` (readable by you alone), where `sci` reads it,
  and taken out of it when cleared. The WebView's keys are a separate set and
  are not read.
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
  the prompt of every turn, not a rule; nothing checks that they exist. The
  subscription sign-in route is used for signing in to ChatGPT, Codex or
  SuperGrok and for renewing that sign-in. A saved model can have a route of
  its own (its Route field: empty follows the page, `none` connects
  directly), which wins over the page's model route for every request made
  with it; so can an MCP server over HTTP, over the connector route.
- Environments keeps SSH hosts in the WebView's list (`ssh_hosts`) and
  registers each as the execution context `ssh:<name>`, which is what the
  model passes to `run_in_context`. Every turn's prompt names them and the
  one new work goes to, in a conversation that was started earlier too. A
  host is a name plus either an address or nothing at all, in which case
  OpenSSH resolves the name from `~/.ssh/config`. Wisp keeps the path of a
  key file, never the key, and signs in with your keys, agent and config,
  or with a password: Sign in with, A password keeps it in the system's
  keyring under the WebView's name (`ssh_password:<name>`), never in the
  database, and the WebView's one-shot askpass answers the host's prompt
  with it for runs, checks and terminals; the list says whether one is
  saved, the form never shows it, a password left empty keeps the saved
  one, and going back to a key or removing the host drops it. Work is
  refused on a host until a check of it has passed: Check
  connects once, without prompting, and records the system, processors,
  GPUs, scheduler and the Python and R it found, which is also what the
  model is told about the host; a check that fails records why, and the
  model is told not to try the host. The check is the WebView's probe
  (`wisp_runs::context_probe`). WSL, interpreter paths set by hand, and
  scratch and storage locations are not ported. Removing a host keeps the
  records of runs made on it and puts new work back on this computer if it
  was the default.
- The first-run guide follows the WebView's: it is shown while the
  shared `onboarding_done` setting is unset, its model step sets up
  DeepSeek only (the key goes to the system keychain through the same
  path as Settings, Models) and is skipped when models come from `WISP_*`
  variables, and Open the DeepSeek console opens the system browser. Its
  tools step starts the same install as Settings, Environments. With no
  model at all Settings also opens, underneath the guide. The WebView's
  local environment panel (tools detected on the system) is not ported.
- Tools on this computer is the WebView's managed toolchain, ported into
  the host (`src-tauri/src/toolchain.rs`): a checksum-pinned pixi installs
  uv, Python 3.12, R (with jsonlite) and Node.js 24 from conda-forge under
  `<data>/toolchain`, and uv makes Wisp's Python environment at
  `<data>/python/.venv` with the bundled analysis packages. Nothing is
  installed until Install is pressed, it needs no administrator rights,
  and the user's own pixi, shell profile and `PATH` are left alone.
  Downloads take the Code requests route, the conda mirror, the Python
  index and the CA bundle from Network. When it ends the paths are saved
  on the Local environment the WebView reads, the managed folders lead
  `PATH` for shell commands and runs, and conversations start Python from
  the managed environment first; a conversation's R interpreter likewise
  starts the R installed here before one on `PATH`, checked each time it
  starts, so an install takes effect without a restart. There is no
  first-run wizard and no detection of tools installed elsewhere; one
  install runs at a time and the page follows it while it is open.
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
  (`auto_compact`, `auto_compact_percent`), and so does Continue cut-off
  answers (`auto_continue`, `auto_continue_limit`, 1 to 100 and ten by
  default): an answer stopped at the model's output limit is carried on by
  the agent itself, at most that many times a turn. Compaction on a model
  switch and after idle hours are not ported.
- The sample project is four small text files with made-up numbers,
  written into a new folder ("Wisp sample project", numbered if that
  exists) under the folder you choose, never over anything. It is an
  ordinary project, and asking about it uses your model. Removing it is
  removing a project.
- Example conversations are the WebView's example project: the
  recordings in the bundled `seed/` folder (`manifest_*.json`, and the
  figures and tables of two of them in `assets_*.tar.gz`), found where the
  bundled skills are. Reading one asks no model and writes nothing.
  Copying one into a project writes its notes and files into the project
  folder (the archives' files at its top, by name), skips any path that is
  already there, and only takes plain files from the archives; the conversation keeps the recorded
  answers and tool results (the plans are left out, as the WebView leaves
  them) and is named after the example. The title is the first sentence
  of the recorded request.
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
- Terminals follow the WebView's `terminal_sessions.rs`: the host keeps
  each as a PTY (ConPTY on Windows) running this computer's login shell
  in the project's folder (`$SHELL -l`, PowerShell on Windows), or the
  interactive `ssh` of a saved host (a stored password answered through
  the one-shot askpass), or `wsl.exe -d` for a WSL distribution, with
  `TERM=xterm-256color`. The window draws its screen with `vt100` (the
  xterm colours, bold, italic, underline and inverse, the cursor, 5,000
  lines to scroll back with the wheel) at the size that fits the dock,
  and types into it while it has focus: Return, Backspace, Tab, arrows
  (application mode included), Home, End, Page Up and Down, Delete, F1–F4,
  Ctrl chords and Alt as Escape; Cmd+V (Ctrl+Shift+V elsewhere) pastes,
  bracketed when the program asks, and Cmd+C (Ctrl+Shift+C) copies the
  screen's text. While a terminal has focus it keeps Escape, and off
  macOS every Ctrl chord, for its shell rather than the window. The host
  keeps the last 4 MB each wrote, so a terminal shown again is redrawn;
  closing a tab stops its terminal, and all stop when the app quits. What
  is typed there is yours: the model does not see it. Not ported: input
  through the system's input method (Chinese, Japanese) and selecting
  part of the screen.
- Whole log reads `stdout.log` and `stderr.log` from the run's own
  folder (`~/.wisp-science/runs/<id>`), the last 512 KB of each, when it
  is asked for; it does not follow a run that is still writing, so press
  it again to see more. A run on an SSH host keeps its log on that host,
  and its last lines are what the Output shows, until its workspace is
  cleaned up: the log is then saved into the project's `runs/<id>`
  folder first, and Whole log reads it from there. Once a local run's
  folder is gone, so is its whole log, unless it was saved the same way.
- The update check runs once when the app starts, unless At start is
  turned off in Settings, General, or Don't check at start is pressed on
  the note it shows (the WebView's `update_check_enabled`, so either
  desktop honours it), and whenever its button is pressed; nothing is
  checked in the background. At start it says something only when a
  newer release exists, in a note in the window's corner. It asks GitHub for the latest
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
- Notifications are posted only while the window is not the active one:
  for the conversation open in it when its turn ends or a tool waits, and
  for a conversation among the twelve most recent that is not open when
  it stops working, as answered or as needing you (an answer to read or a
  question to answer; the host does not say which). They are posted with
  what the system already has: on macOS
  through `osascript` (so the banner is filed under Script Editor, and
  the first one may ask for permission) and `afplay`; on Linux through
  `notify-send` and `canberra-gtk-play`, silently doing nothing when
  those are missing. Windows gets the sound only. The sound cannot be
  chosen.
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
  are not listed. A row's opened text stops at 16,000 characters. When
  the working set is compacted, Undo the compaction puts back what the
  compaction replaced, as the WebView's undo does: only the compaction
  shown, only while nothing has been said after it, and not while a turn
  runs; the model is sent everything again from the next turn.
- The interpreter line is read when a conversation opens, about once a
  second while a turn runs, when a turn ends and after Stop; an
  interpreter that dies while nothing is running shows as stopped only at
  the next of those. Stop loses what the interpreter had in memory and
  keeps the files it wrote; a turn that is using it gets an error from
  that tool call. A stopped interpreter stays listed until dismissed, and
  the model's next call says it was stopped rather than quietly starting
  a new one. The console follows the WebView's runtime cards: it lists
  the variables the interpreter holds (name, type, a summary, size),
  runs code you type in it as your own (outside the approval of the
  model's tools, but refused while the project is frozen for an
  exploration and kept to an exploration's copy), shows what it printed
  and the plots it drew, and Restart starts it afresh. Starting one from
  the palette starts the same interpreter the model's `python` or `r`
  tool uses in that conversation, on this computer only. There is no
  memory limit; interpreters are also stopped when the conversation or
  its project is removed and when the app quits.
- The Agents tab shows first the latest work over many items
  (`map_items`) the conversation ran, as the host follows it from the
  tool's progress (the WebView's swarm progress, merged the same way):
  its phase, how many items are done and failed, each part's status and,
  once done, its summary. It is kept while the app runs, until the next
  such work, and is not saved.
- The Agents tab then lists the open conversation's delegated batches,
  newest first, from the host's records: each with its status, how many
  tasks are done, its goal, and per task its status, instruction, what
  it waits for, its summary or error, what it used, and a button that
  opens the conversation it worked in. A running batch has Stop, which
  stops it and its tasks, and one that failed or was stopped has Run
  again; the list refreshes while the conversation's turn or any of its
  batches runs, and when the turn ends. Below them come the helpers, a reading of
  the loaded transcript like the Notebook: the `explore` calls from the
  messages that are loaded, with a helper's findings once it has
  reported back, not its steps along the way.
- The sub-agent model is the model the `explore` helper works on, the
  only sub-agent this host has. The order is the WebView's: a
  conversation's own choice for its sub-agents (its `frames` column, set
  at the end of the composer's model menu: as in Settings, this
  conversation's model, or a saved model), then the choice in Settings
  (`subagent_model_id`), then the conversation's model. Choosing this
  conversation's model outranks Settings. A choice naming
  a profile that is gone or unusable falls through to the next. It
  applies from a conversation's next turn. Subagents and delegated tasks
  use the choice in Settings, or the conversation's model when there is
  none or they ask to inherit it.
- ACP agents keeps the WebView's list (`acp_agent_profiles`, same
  shape), so an agent added in either desktop is in both. A program is
  shown as not found when it is neither a path to a file nor a name on
  `PATH`. Test starts the program with its arguments through `wisp-acp`,
  waits up to 30 seconds for it to introduce itself, and stops it.
  Signing in starts the agent once more: one it signs in to by itself
  is asked to (it may open the browser; up to five minutes), and one it
  signs in to in a terminal opens the system's terminal running the
  agent's program with its own arguments and the method's, in the open
  project's folder or the home folder (on macOS through a `.command` file
  in the temporary folder, every word quoted). A key read from an
  environment variable is only named; it is set outside Wisp.
- Once the agent has answered, the modes and options it reports (such as
  a plan mode or a choice of model) are buttons in the composer: a list
  for one of several values, a switch for one that is on or off. They are
  set through the agent and kept as it reports them while its process
  runs; before the first answer and after a restart until the next one
  they are not shown, as nothing is cached between runs. In such a
  conversation every message goes to the agent as typed, a leading `/`
  included, so Wisp's own slash commands are not run there; the "/"
  button lists the commands the agent reported instead, and picking one
  writes it into the message box.
- A conversation is given to an ACP agent from the composer's agent
  button before its first message, for good (the store refuses another
  choice; Wisp needs a new conversation). Its first message starts the
  program in the project folder and opens a session with it; the binding
  (`acp_sessions`) is the WebView's, and after a restart the session is
  resumed, or loaded when the agent cannot resume. Each message is one
  prompt: the agent's text and thoughts stream in, its tool calls are
  saved as `acp:<title>` rows and its last plan as a `wisp:plan` row (the
  WebView's shapes), and its permission questions are the usual approval
  card, answered with its allow-once or reject-once option; with Full
  access they are allowed without asking. Stop cancels the prompt and the
  conversation goes on with the same process. Wisp's model, effort, plan
  first, expert, where work runs and turn review do not apply and are not
  shown; follow-up questions and reviews are not asked for. The agent
  is also given Wisp's own science tools over MCP, under the name
  `wisp-science`: the skill catalog and skills, memory search while
  memory is on, runs (`run_in_context` and the tools that follow a run),
  the switched-on literature and biology connectors and the user's MCP
  servers; not files or the shell, which the agent has itself. The
  skills, memory and run tools are listed; the many connector and MCP
  tools are kept behind `wisp_search_tools` and `wisp_use_tool` (the
  WebView's names, arguments and scoring), so the agent is not handed
  hundreds of tools at once. The host
  serves them on a loopback address with an unguessable path per
  conversation (offered again each turn, so switching a connector applies
  from the next message); an agent that speaks MCP over HTTP is given the
  address, and one that only starts programs is given the desktop's own
  executable in relay mode (`--wisp-mcp-relay <address>`), which carries
  stdio to it. Calls run in the project folder without a question of
  Wisp's (the agent asks its own), and what would need one is refused.
  Not ported from the WebView: the bridge's capabilities, artifact,
  research graph and delegation tools,
  questions it asks the user, file and conversation
  references sent as resource links, and a conversation run by an agent
  inside an exploration.
- An expert is chosen before a conversation's first message and cannot
  be changed afterwards: its instructions are written into the system
  prompt with the first turn and stay as they were then, also if the
  expert is later changed or removed. The list and each conversation's
  choice are kept under the WebView's keys (`specialists`,
  `frame_specialist:<id>`). An expert can have a model of its own (the
  WebView's `model_id`), chosen in its form: a conversation started as it
  runs on that model, and its header says so, as long as the model is
  still saved; otherwise it runs on the conversation's model. Its skill and
  connector lists are the WebView's (`skills`, `connectors`; absent is
  every one, empty is none), set in its form with Every skill or Only
  these, and the same for connectors: a conversation started as it can use
  only those skills, and only those bundled connectors (by domain) and MCP
  servers (by id), from every turn, as the WebView's specialists do. The
  catalog the model browses still names every skill; one it was not given
  cannot be used. The
  WebView's one-shot built-ins (reviewer, reader, archivist, recap) are
  not offered. The built-in Scientific Illustrator draws SVG; this host
  has no image-generation tool.
- A quick action here is a named request put into the message box; it
  sends nothing by itself and starts no workflow. The WebView's quick
  actions start multi-agent workflows; here that is Run as agents in
  Settings, Workflows, so the two lists are separate: this desktop keeps its own under
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
  pruning and summarising and no instruction for the summary; it can be
  undone from What the model is sent until the conversation goes on. The
  transcript on screen keeps its length.
- A slash command is read only when it is the first word of the message
  and one of the known ones, so a message that starts with a path is
  sent as a message. There is no list that narrows as you type.
  `/fork` branches the conversation as it stands, `/rewind` asks about
  going back to the latest message, `/permission` switches full access
  (asking first when it turns it on), and `/share` saves the
  conversation as an HTML page, as Share as HTML does, rather than
  publishing it anywhere.
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
- A workflow runs in one of two ways. Run makes it one conversation: the
  host lays the tasks out in the stages their dependencies allow and the
  conversation's model carries them out in that order, with the usual
  approvals. Run as agents follows the WebView's Workflow Studio: each
  task is its own agent, given only the capabilities the template grants
  it (think only, read the project, change it, run code, review work;
  none given is reading the project) and, under Agent settings, the
  expert it works as, the sub-agent model or the conversation's own, a
  Git worktree of its own, a token budget, a time limit and the JSON
  Schema its result must match; and the tasks are drafted as a
  delegated batch in a new conversation named after the workflow. The
  batch waits in that conversation's Agents tab, with what each task may
  do, for Approve and run or Discard; under the approval policy "Start at
  once when it only reads" (the WebView's auto-safe) a batch that changes
  nothing and runs no code starts at once. It then runs in the background
  like a delegated batch (Stop and Run again included) and its result is
  delivered to the conversation, whose model sums it up. Run as agents
  takes at most 8 tasks, the delegation limit. Not ported: ACP executors
  and a named model profile per task (a task picks the sub-agent model or
  the conversation's), tool-call and cost budgets, dragging tasks about on the map,
  skills that bring scripts of their own as conversion sources, and literature and web research
  capabilities.
  A task can only need tasks listed before it; besides the numbered
  After task buttons, the form shows the workflow as a map, each task in
  the column of its stage with an edge to each task that needs it, where
  clicking a task and then another makes the later one need the earlier
  one (or no longer need it). A workflow can also carry a context
  (scope, conventions, how results are reported) that every task is
  told. From skills follows the WebView's `plan_skill_portfolio`: the
  sub-agent model reads the picked skills' method documents in full
  (or first picks at most three for the request) and writes a workflow
  of agents from them through the shared `workflow_conversion` checks,
  repaired once if the host rejects it; it opens in the form to review,
  and nothing is kept until it is saved. The two libraries are
  separate: this desktop keeps its own under `desktop.workflows` and
  neither reads nor changes the WebView's `workflow_templates`. It starts
  with two that can be changed or removed; a workflow has at most 20
  tasks and there are at most 40.
- Chat apps follow the WebView (`src-tauri/src/channels/`): a text from
  a chat drives an ordinary conversation through the same turn path as a
  send from the desktop, and its answer (the turn's `attempt_completion`
  result or last assistant text) goes back to the chat, cut at 8000
  characters. Every channel shares one target under the WebView's
  setting `channel_last_message_route`: `/project` picks the project,
  `/session` one of its conversations, `/new` starts one with the next
  message; a send from the desktop only records its project's latest
  conversation. A turn from a chat asks before every tool that changes
  something, even in a conversation with full access; the approval is
  sent to the chat with an eight-character code, and `/approve <code>`
  or `/reject <code> [reason]` answers it (once; WeChat only). Replies to
  the chat are in Chinese, as the WebView's are. WeChat binds through
  Tencent's iLink service (`ilinkai.weixin.qq.com`): only the person who
  scanned is answered, one to one; the bot token is kept in the keychain
  as `weixin_bot_token`, the binding, the poll cursor and whether it is
  on under `weixin_binding`, `weixin_sync_buf` and `weixin_enabled`, so
  either desktop's binding serves both (run one at a time: both would
  poll the same bot). The bot runs while the app is open. The research
  assistant has a WeChat bot of its own, as in the WebView's
  `channels/assistant.rs`: a second card binds a different bot, kept
  under `assistant_weixin_bot_token`, `assistant_weixin_binding`,
  `assistant_weixin_sync_buf` and `assistant_weixin_enabled` (one bot
  cannot serve both). Every message there is a turn of the assistant's
  conversation, never the shared route; `yes` and `no` answer what it
  waits on, `full` approves it and turns on the assistant conversation's
  full access (kept with the conversation, `full off` turns it off),
  and `/approval`, `/approve`, `/reject`, `/status`, `/model [number or
  name]`, `/resume` (the last request whose turn failed), `/mandates`
  and `/stop` work as there; other commands are refused. A mandate's
  notice to the assistant is also queued for that WeChat and goes out
  with the owner's next message, the window iLink replies in. Feishu follows
  `feishu.rs`: a self-built app's bot over Feishu's long connection
  (endpoint discovery, then pbbp2 frames over a WebSocket, each
  acknowledged at once and repeated deliveries dropped), replies over
  REST. Scanning makes the app through Feishu's device flow
  (`feishu_registration.rs`) and makes the one who scanned its owner;
  an App ID and App Secret made in the developer console can be entered
  instead. Only the owner is answered; with none, the first to write is
  kept as pending until Settings makes them owner or refuses them. An
  ordinary message gets a CardKit card that shows the answer so far and
  the latest tools (names and how they went, never their output or the
  model's reasoning), redrawn about once a second from the turn as the
  host polls it, and ending as the answer. Approvals are not answered
  from Feishu; the turn waits for the desktop. The App Secret is kept in
  the keychain as `feishu_app_secret`, the rest under `feishu_app_id`,
  `feishu_international`, `feishu_enabled`, `feishu_owner_binding` and
  `feishu_pending_owner`. Not ported yet: the research assistant's own
  WeChat, and `/mandates` with the notices research mandates send the
  owner (mandates themselves are not ported). Feishu answers approvals
  nowhere but the desktop, as in the WebView.
- Wisp Lab follows `lab.rs` and `lab_mail.rs` (docs/wisp-lab.md): the
  identity (relay, member key, lab key, name) is one keychain secret,
  `lab_identity`, and the relay token `lab_relay_token`, both under the
  WebView's names. A member computer's agents get `lab_members` and
  `lab_send` and are told about the lab every turn; a question is sealed
  with the lab key, waits in the recipient's mailbox on the relay, and
  the mailbox is looked at every 15 seconds while the app is open. A
  received question runs one turn in a conversation kept per sender
  (`Lab: <name>`) in the project chosen for lab mail and is answered by
  mail; the answer arrives in the asking conversation as a new message
  and runs a turn there. With no project chosen a question is answered
  at once that this computer does not take lab mail. A turn a mail
  started asks before every tool but reading this project's files,
  skills, the plan, `lab_members` and finishing; it reads only inside its
  project and leaves the user's memory out. At most four mails follow
  one another without a person writing. Up to four project files of 32
  MiB each travel along, encrypted, and arrive under `uploads/lab/`. The
  leader publishes a project synchronized through the lab's relay as the
  knowledge base (its device code, sealed with the lab key); a member
  downloads it like a synced project, and from then on member agents are
  told its folder every turn but a mail's.
- The StickS3 desk display follows `device_bridge.rs` and `device_hub.rs`:
  an opt-in HTTP listener on one concrete IPv4 address (never 0.0.0.0),
  port 18766 unless another is given, whose routes but `/health` want
  the token in the `x-wisp-device-token` header; the token is kept in
  the keychain as `sticks3_device_bridge_token` and the address under
  `device_bridge_enabled`, `device_bridge_bind_ipv4` and
  `device_bridge_port`, the WebView's names. `/state` says what the
  agent is doing (idle, working, review, needs you, done, failed, the
  conversation that most needs attention first); the WebView folds live
  events into it, this host reads each conversation's live state when the
  device asks, and a turn that ended since it last asked shows done or
  failed until the device acknowledges it. `/pet/manifest` and
  `/pet/frame` draw the chosen pet's frames at 120×130 as PNG. The
  actions are `ping`, `focus_session` (the window comes forward on that
  conversation) and `acknowledge`; the last 50 are listed at `/actions`.
  Turning the listener off forgets the token. The relay transport the
  WebView reserves is not offered there either.
- Project sync follows `project_sync.rs`, whose core is carried over
  unchanged: the project's records (its database, exported whole) and its
  workspace files are encrypted with a per-project key and published as
  revisions signed with it, through a wisp-relay (`sync_relay_url`, the
  token in the keychain as `sync_relay_token`) or a shared folder
  (`sync_folder`, revisions under `Wisp Sync`), chosen by `sync_backend`.
  The key is kept in the keychain as `sync_project_key:<project>`; the
  device code carries it, with the project's id, name and relay. A sync
  pushes when only this device changed, pulls when only the other did,
  and when both did says so and changes nothing until a side is chosen.
  Files over 64 MiB, or with names Windows or macOS cannot hold, stay on
  the device and are listed. A sync waits while anything in the project
  runs, waits on an approval or is reviewed; there is no lock that holds
  new turns back meanwhile. A pull drops the conversations' live state.
  Cloud-drive folder projects keep their own snapshots in their folder
  (the store's folder snapshots): this device's edits are published every
  20 seconds once nothing runs, and opening the project takes in a newer
  version when this device has nothing unpublished. Its card on the home
  page says where its latest changes stand: saved to the folder, not yet
  saved, a newer version in the folder, a version conflict, or waiting for
  the drive. Sync keeps the WebView's names, so a project synchronized on
  either desktop syncs on the other.
- Remote web access is the only tunnel ported. A browser
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
- A plugin is installed from a folder, from a `.zip` (at most 256 MB,
  unpacked under the same limits; an entry that leaves the package or is
  a symbolic link refuses it), or from an HTTPS address with the SHA-256
  its publisher gives: downloaded on the connector route, following only
  HTTPS redirects, and installed only when the archive has that checksum,
  when it is recorded as checksum-verified; otherwise it is unverified.
  One version of a plugin is kept; a newer
  one replaces its files, and a project that had the older one on has
  to switch the new one on. A package with a symbolic link, more than
  10,000 files or 512 MB is refused. The MCP servers a plugin declares
  are started for the turns of the projects that switch it on, as the
  WebView starts them: a program inside the plugin's folder or on the PATH,
  with `${CLAUDE_PLUGIN_ROOT}` standing for that folder, run in it with a
  cleared environment that keeps only the system's plumbing, the plugin's
  own variables and the connector route; their tools are named after the
  plugin and the server and ask before they run. One that cannot be started
  is listed with why among the plugin's problems. A plugin's skills do not
  show on the Skills page and cannot be turned off one by
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
  keeps its choice. A review that finds problems has them corrected once,
  as the WebView does: the conversation is resumed with the findings, told
  to that turn only (nothing is added as said by you), and what it then
  says is reviewed again without a second correction. Unlike the WebView,
  the first turn has ended by then, so the correction is a turn of its
  own; one started meanwhile by you takes its place. A turn
  that failed, was stopped, ended on a question to you, or only said a
  little without tools is not reviewed. The line above the composer
  stays until the next turn starts; the report is also kept with the
  conversation (below). It applies to every turn this host runs,
  a paired phone's included, though a phone is not shown the result.
- A review is one model request without tools, made by the reviewer model
  chosen in Settings, Conversation, or by the conversation's own model
  when none is chosen or the chosen one was removed. The review button
  under an answer reviews the conversation up to the end of that turn,
  leaving out what came after. It reads the
  conversation's latest messages up to 80,000 characters, with each
  tool's arguments and output cut at 4,000, so a claim that rests on
  something older or longer cannot be traced and is not reported. Every
  finished review, asked for or automatic, is kept with the conversation
  in the WebView's `session_reviews`, in its report shape, and shown in the
  transcript after the last message it read, with what it found and See
  to open the report again; going back to an earlier message drops the
  reviews of what came after. It is refused while a turn runs.
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
  list of the evidence it used. Ask on a run card ties the next questions
  to that run: its live snapshot (status, timing, exit code, command,
  progress and stored output tails) goes along as the newest source, as
  the WebView's run-card questions do, until it is put away or another
  conversation opens; the run's server is not asked anything. A question
  cannot be tied to selected text. The thread is kept per conversation in the window
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
  that start afterwards. "Always, in every project" does the same for every
  project and is kept in the WebView's global grants (`approval_grants`,
  its `global` list), whose other entries are left as they are; a tool the
  WebView allowed there is allowed here too. New conversations start in the mode Settings names
  only when the desktop makes them; one a paired phone makes starts in Ask
  first.
- A project's tool approval mode and its rules for single tools are the
  WebView's, read and written under its keys (`approval_scope:<project>`,
  `tool_approvals:<project>`, and the keys alone as the default for a project
  without its own). Per-tool rules keeps this desktop's question before
  changes and applies each tool's rule: Ask first asks before every call,
  even one that changes nothing; Deny never runs it. Auto-approve skips the
  question before changes and the Ask first rules; a dangerous shell command
  still asks. Full bypass skips every question, the dangerous command's too.
  Deny holds in every mode, and another lab member's mail asks before
  changes whatever the mode. Rules are set on the Connections page for the
  bundled connectors' tools, whose names are known, as in the WebView; the
  mode is chosen on Permissions; both for the project that is open, from
  the next turn. A connector's Skip approvals (`skip_approval_connectors:
  <project>`) lets all its tools run whatever their rules, a denied one
  included, as in the WebView.
- Where the tools are, on the Environments page, keeps the WebView's paths
  for the local environment's executables on its `local` execution context
  (`python_executable`, `rscript_executable`, `uv_executable`,
  `node_executable`, `npm_executable`, `sci_executable`, `pixi_executable`,
  `cua_driver_executable`), which is also where installing a tool records
  it. A path must name a file; nothing is run to check it. An empty one is
  looked for on the search path the app started with, shown as the hint.
- Command hooks are the WebView's, under its keys (`command_hooks`, and
  `project_hooks_trust` for the SHA-256 of each project's trusted
  `.wisp/hooks.json`, in Claude Code's layout). A hook runs in the project
  folder with the event as JSON on stdin (`session_id`, `cwd`,
  `hook_event_name`, and the prompt, the tool's name, input and response,
  or `stop_hook_active`), under `sh`, or PowerShell on Windows, for at most
  its timeout (a minute when none is given). UserPromptSubmit runs before a
  new message is taken: exit 2 refuses it, and what it prints is context
  for that turn. PreToolUse runs before a tool's approval: exit 2 or a
  deny blocks the call with the reason as its result, ask makes it ask, and
  a hook that fails or times out blocks it too. PostToolUse and
  PostToolUseFailure feed what they say back with the result. Stop runs
  when a turn has ended well and any correction the review asked for has
  run: exit 2 has the turn go on once with the reason; if it still blocks
  after that, it is only said. A failed hook, or a project file that was not
  run because it is not trusted as it is, is shown over the composer until
  the next turn. A project file runs only while its content is the one
  trusted; a script it calls is not checked. An ACP agent's turns run no
  tool hooks.
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
- The research assistant follows the WebView's `research_assistant.rs`:
  one conversation in a hidden project, made on first use, run through
  the ordinary turn pipeline but with its own system prompt and only its
  own tools: the projects, a week of recorded activity with the days'
  recaps, a project's progress memory, a project's conversations and
  execution contexts, the dated plan (add, list with earlier open items,
  open/done/dropped), dispatching work to a new or named conversation of
  a project (optionally on an execution context), and a conversation's
  latest answer. Each turn is told the local date and time. Dispatched
  work runs as an ordinary turn of that conversation, which keeps the
  name the assistant gave it; when it ends, the assistant's model
  reviews the outcome into a report added to the assistant's
  conversation once it is at rest, without resuming it. Dispatching and
  changing the plan ask first unless the conversation has full access.
  The progress memory follows `project_status.rs`: the daily recap run
  folds each project's complete days into it with the default model, up
  to a week of days per run, and only the assistant reads it. Not
  ported: privacy mode (every project is visible to the assistant),
  the side model that answers approvals of dispatched work, and a
  selected project as the message's scope. It is reachable on WeChat
  too (see Chat apps). Its
  `research_mandates` and `reply_to_mandate` list the mandates and pass
  the researcher's words on.
- Research mandates follow the WebView's `mandates.rs` and reuse
  `wisp-app`'s mandate logic: a mandate is a standing goal of a project
  whose agent works in rounds, on its cadence and daily cap, in one
  conversation of the project named after it. Every turn there, a due
  round or one the researcher writes, is given the mandate's brief and
  its `end_round` and `request_assistance` tools, and asks before
  anything that changes state unless the mandate allows it. A message
  the researcher writes there while it waits answers its request. A
  request is added to the research assistant's conversation; chat apps
  are not told. A round that ends without `end_round` is still recorded
  from its last answer, an ended period closes the mandate, and closing
  withdraws an open request. Rounds run while the app is open, looked
  for with the scheduled tasks. Reports follow `mandate_report.rs` (its
  tests are carried over): on the mandate's report cadence, on closing,
  or on request, the rounds of the period and the project's records
  become a cited report on the default model, from the rounds' own words
  when the period was quiet or the draft fails; it is kept on the
  mandate and added to the research assistant's conversation. The
  Research mandates page (the gauge on the home page, or the command
  palette) lists every project's mandates with their status, KPIs, last
  round, next round and latest report; an open request is answered on
  its card; a mandate can be run now, paused and resumed, reported on,
  edited, closed and reopened, opened in its conversation, and deleted
  after a question. The form takes the project, the goal, a name, one
  KPI as "name, target, period", the cadence of rounds and of reports,
  and whether it asks before changes. End dates, more than one KPI and
  the constraint notes the WebView's form also takes are not offered.
- Scheduled tasks and the daily recap are the other parts of the
  WebView's research assistant that are ported. Tasks fire only while the app is open, looked for
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
  other. A draft can be checked and finalized here with the WebView's
  own rules (the check is ported into the host from
  `src-tauri/src/publication_freeze.rs`), findings can be let stand with
  a reason (recorded as written by "Local user"), and New version copies
  any version into a draft. A version cannot be published, packed as a
  capsule or reproduced from here. A finalized version is shown with its
  evidence and cannot be changed, and a paper with such a version cannot
  be removed. Finding messages come from the check in English; the few
  fixed ones are translated. Evidence is a registered
  result of the project (bound at the version it has when chosen) or a
  run; passages of a conversation, tool calls and outside resources
  bound in the WebView are listed by kind but cannot be added here.
  Evidence is not reviewed or marked from here, and items are flat:
  they are shown in the order they were added and cannot be nested or
  reordered.
- On an SSH host a terminal runs the WebView's interactive `ssh` for that
  host (port, key or password prompt, jump hosts), with the browsed
  folder as a quoted `cd` when there is one; a WSL distribution opens with
  `wsl.exe -d` in the project folder. Every press opens another tab.
- Debug request follows the WebView's `debug_request.rs` in its
  stored-messages form (this host keeps no agent between turns): the saved
  working set with each message's size and token estimate, the system
  prompt's share, the model, the turn step limit and how the last turn
  ended; tool definitions are not included.
- The research journey reads the project's history from the same tables the
  WebView does, a month at a time, grouped into your computer's local days.
  A record of your own is progress, a finding, a decision or what comes
  next, and is dated when you save it; it cannot be dated otherwise, changed
  or removed here. A row that came from a conversation opens it. The
  relation graph, the sources of a file, and the history of an
  exploration branch are not ported, and neither is the calendar across
  projects, which is a page of its own.
- Daily recaps follow the WebView's `research_recap.rs`: a day's records
  (runs with their errors, files, decisions and notes, what was asked in
  conversations) go to the default model as a bounded digest whose
  records have short handles, and the draft it returns keeps only the
  handles that exist, as sources. Draft a recap (or Draft again) on a
  day of the journey replaces that day's recap; Confirm and Dismiss
  review it, and a dismissed day can be drafted again. The daily run,
  on the Scheduled tasks page (on at 09:00 at first, like the WebView),
  drafts each project's last three complete days that have records and
  no recap, never replacing one, while the app is open; Run now starts
  it at once. Change on a recap opens its headline and its four sections,
  one item a line, to edit; an item left as it was keeps its sources,
  and Escape closes the form alone. Sources are not opened from the
  card, and the WebView's Recap specialist model is not ported: the
  default model drafts them.
- The research calendar reads the same history as the journey for every
  project at once, a month at a time, in your computer's local days. It
  opens on today; another month opens on its last day with something on
  it. A project that cannot be read is named with the reason instead of
  being shown as empty. Which projects are left out is kept only while the
  app runs. A row that came from a conversation opens it. Records cannot be
  added from the calendar.
- Every window talks to the one host in the app, so projects, settings and
  conversations are the same in all of them, and each keeps its own page,
  open conversation and panels; waiting messages are the host's, so every
  window on a conversation shows the same queue. A window does
  not hear at once what another changed: lists catch up when they are next
  read or refreshed. Two windows on one conversation both follow its turn;
  the host still runs one turn at a time in it. Closing a window leaves
  the others; the app quits with the last. A project's card and its row
  in the palette open it in a new window. Such a project window is kept
  (as the preference `project_windows`) until it is closed: quitting keeps
  it, and it opens again in that project at the next start, if the project
  is still there. Windows opened on the home page are not kept, and a
  conversation cannot be sent to a new window from its row.
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
  Pictures are not embedded. Share as HTML writes one page instead, with
  the WebView's share stylesheet (copied into `apps/gpui/assets`, a test
  keeping it equal while the WebView sources exist): your messages and the
  answers, light or dark as the window is, without tool calls or
  reasoning. Answers are rendered from Markdown with raw HTML shown as
  text and links that could run code dropped. It shares the whole
  conversation; there is no picking messages, long image or share link.
- The notebook is a reading of the transcript, as in the WebView, not
  something saved: every `python`, `r` and `shell` call with its output,
  and every code block an answer shows; code that was shown and then run
  is one cell. It covers the part of the conversation that is loaded.
  Output is cut to fourteen lines in the panel. Whether a run failed is not
  marked, because the host does not record it on the row. Cells cannot be
  run or edited, and a run started with `run_in_context` is in the Runs
  tab, not here.
- The outline lists your own messages in the part of the conversation that
  is loaded, by their first line. While earlier messages are not loaded it
  says so, and Load earlier loads every earlier page, so the whole
  conversation is listed (and in the transcript). It has no headings from
  the answers and no search of its own.
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
  own answer, or with an error; "Send the next one" overrides that. As in
  the WebView the host keeps the queue, in memory: it moves on whether or
  not its conversation is open, in any window, and quitting drops it. A
  correction an automatic review asks for goes before it. Deleting or
  moving the conversation drops what waits in it, and a paired phone
  neither sees nor feeds it. While the turn runs, the lightning button
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
  model. The built-in workspace can be renamed like any other. Its
  automatic server cleanup is the WebView's: how many days a succeeded
  run's folder on a server is kept after its results came back, a failed or
  cancelled run's, and server files nothing refers to any more, each from
  1 to 3,650 days or empty to keep them; they are kept with the project and
  the run manager's sweep, which runs while the app is open, applies them.
  A refused form saves none of its fields.
- Storage measures, as the WebView's page does, the database, the
  favorites, the Python, R and Node.js installed into the data folder, the
  plugins, the rest of the data folder and its total, and for each project
  its whole folder (your own files included) with its `.wisp` and
  `uploads` parts, each walked up to 50,000 entries (a larger folder reads
  "over" what was counted). Nothing can be moved or cleaned from the page,
  in the WebView either. Thinking is kept as the model reported it; a model that reports
  none leaves no Thought. Fast mode is a lightning button in the composer
  for a conversation on OpenAI's own API, its Responses API or a ChatGPT
  sign-in; it is saved as the WebView saves it (`service_tier=priority`)
  and sent from the next turn. There is no per-model default for it.
- Plan mode uses the WebView's switch (`frame_plan_mode:<conversation>`) and its
  prompt section, which now lives in `wisp_tools::plan` for any host to use;
  `src-tauri` still carries its own copy of the same text. Approving sends
  "Approve and execute", or whatever is typed in the composer.
- Files dropped onto a conversation are attached the way picked ones are.
  A picture pasted into the composer with Cmd+V (Ctrl+V elsewhere) is kept
  as `uploads/pasted-<date>-<time>.<format>` and attached the same way;
  when the clipboard also holds text, the text is what is pasted.
  A drop takes at most twenty files; folders are left out, and it says how
  many items it did not attach. Pasting a file (rather than a picture)
  from the clipboard is not ported.
- A file attached from outside the project is copied into the project's
  `uploads` folder (up to 100 MB, never overwriting); a file inside it is used
  where it is. The model receives project-relative paths and reads the files
  with its tools. A model marked as seeing pictures also gets the attached
  pictures themselves (PNG, JPEG, GIF, WebP), as the WebView sends them:
  one over 5 MB is sent smaller without asking first, and the file is not
  changed. A model that cannot see gets the paths and what the picture
  model saw in them, as with the WebView's image model: the saved model
  chosen as Picture model on the Conversation page of Settings, or else the
  first saved model that can see, looks at each picture with the message,
  and what it says is added after the message. With no saved model that
  can see, the paths are all it gets. Whether a model can see is the model
  catalog's word for its ID. A referenced conversation is
  passed as its most recent exchange (about 12,000 bytes), and a skill as its
  instructions, for that turn only: neither is saved with the message, so
  the chips a message shows for them last while the window that sent it
  is open, as in the WebView. Paste,
  drag and drop work, and typing `@` or `#` where a word starts opens the
  add-context picker (the character is taken back out). A slash command
  runs when it is the whole message. Typing `/` where a word starts opens
  the composer's `/` menu, as the WebView's `/` opens its skill picker:
  the commands and the project's skills that begin with what follows the
  `/`, narrowing as you type. A command picked there runs, or is written
  out when it needs words after it (`/btw`, `/timer`); a skill is attached
  to the message; either way the typed word goes. A word with another `/`
  in it is a path and opens nothing. Escape shuts the menu until that word
  is gone. The `/` button opens the same menu.
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
  trimming, and Suggest has the model draft it; nothing is saved until you
  choose where. After a turn that asked Wisp to remember something, or
  whose tool calls failed often (Settings, Conversation, Draft a memory
  when tools fail: off until turned on, at 30% and two failures, the
  WebView's `auto_failure_analysis`), the reviewer model drafts a memory
  with the WebView's prompts; a line above the composer says why, and
  Review opens it in the editor with the scope it suggests. It is put away
  when the next turn starts. "Clear all notes" is not ported. One entry or
  habit holds 4,000 characters.
- The habit tidy follows the WebView's `memory_tidy.rs` (its tests are
  carried over): once a week while memory is on (on at first, looked for
  with the scheduled tasks while the app is open), or with Run now, the
  default model is shown the habits and proposes merges and retirements;
  a proposal naming a habit it was not shown, or one an earlier proposal
  took, is dropped. Nothing changes until a proposal is applied, and one
  whose habits were edited since is no longer offered and cannot be
  applied. A merge keeps the newest habit with the merged text. Project
  notes are not tidied.
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

`apps/gpui/Cargo.lock` holds `libc` at 0.2.186, the version the root workspace
uses. `libc` 0.2.191 no longer has the Linux `ENOATTR` alias that `xattr` 0.2.3
(pulled in by Zed's `zed-async-tar`) references, so it compiles on macOS and
Windows and fails only on the Linux job. If `cargo update` moves it, restore it
with `cargo update --manifest-path apps/gpui/Cargo.toml -p libc --precise 0.2.186`
until that dependency moves to `xattr` 1.x.

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
press Suggest: the text is replaced by the model's draft, and a line says
where it suggests keeping it. Trim it and save it for the project, and
confirm it is in today's note. Save
another as a habit and confirm it is listed under Habits in a second project
too. Add a habit by hand, edit it, ask to delete it and press Escape. Open
today's note, change it and save. Turn memory off and on. With a real model,
ask something a habit should shape and confirm it does while memory is on and
does not while it is off. With two habits that say the same thing, press Run
now under Tidy habits: after a moment a Merge proposal lists both with what
they become; Apply leaves one habit with that text.
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
On the home page press the play-file button: Example conversations opens on
the first recording. Choose another; open a tool row and a thinking row
and close them again. Press Copy to a project, press Escape (the menu
closes, the dialog stays), press Escape again (the dialog closes). Copy
the RNA-seq example into a project: its conversation opens there, named
after it, and the counts matrix is in the project folder; copy it again
and confirm a file you had changed is left as it was.
With a real SSH host that has passed its check, open the Files tab:
the host is offered beside This project. Choose it: its home folder is
listed, folders first; go into one and back Up; Reload. Choose This
project: the project's files are back as they were. With a host that has
not been checked, confirm the tab says a check is needed.
On the host, press a file's download button and choose a place: the
Transfers tray shows it coming, and the file is there when it ends. Press
Send files here…, pick two small files: they show as uploads, and Reload
lists them in the folder. Send one of them again: it is refused, not
replaced.
Have a turn write a file, open the file in the Files tab and press
Origin: the line names the tool and the conversation, the code is shown,
and Open the conversation goes there. For a file you made yourself it
says none is known.
In the Files tab go into a folder and press the terminal button: a
terminal opens in the dock at the foot of the conversation, in that
folder. Type `ls` and Return: the listing shows, coloured. Press Ctrl+C
during `sleep 10`: it stops. Open a second one with +: two tabs. Hide the
dock with Terminals in the top bar and show it again: both are still
there with what they wrote. Close one with its ×. With the project's
folder moved away, confirm a message says it is not there.
In a conversation open the command palette and choose Python console:
the console opens over the composer with an interpreter chip. Run
`x = 41 + 1` and then `x`: it prints 42 and the variables list shows `x`.
Run code that draws a matplotlib figure: the plot shows under it. Press
Restart: the variables list is empty. Press Escape: only the console
closes.
With an SSH host saved, press Terminal on its row in Settings,
Environments: a terminal opens in the dock and connects to it (a password
is asked there, unless the host signs in with a saved one). Change the
host to sign in with A password and type one: the row reads password;
open it again and confirm the field is empty with Saved in it; switch it
back to A key and confirm the row no longer says password. Browse the host's folders in Files, go into one and press
Terminal: the shell starts in that folder. Press it again: a second
terminal opens.
Have the model start a run on this computer that prints more than a
screenful, open the Runs tab, press Output and then Whole log: everything
the run wrote is shown, stdout and stderr apart. Press it again while a
long run is still writing and confirm more appears.
When that run has ended, press its trash button: a question opens under
the card; press Escape and only the question closes. Press it again and
Clean up: the card says its workspace was cleaned and the button is gone.
For a run that declared outputs which were never brought back, Clean up
asks again, saying they would be lost, and only Clean up anyway deletes it.
On a saved SSH host, have the model start a run that writes a few files and
a folder, and wait for it to end. Press the run's folder button: the
dialog lists the top of its workspace. Open the folder, choose a file, go
up and choose another, and press Bring into the project: both appear in the
Artifacts tab. Choose one, press Delete chosen… and then Escape: only the
question closes. Delete it: it is gone from the list. Type part of a name
and press Return: only matching names stay. Press Delete the whole
workspace… and confirm: the dialog closes and the card says it was cleaned.
Press Output and Whole log on that card: the run's log is read from the
project's `runs/<id>` folder.
Open the side panel's Environments tab in a new conversation: This computer
is listed as always available and the saved host as Not attached. Ask the
model to run `hostname` on that host: it is refused as not selected. Press
Attach and ask again: it runs there. Choose the host in the composer's
Runs on menu: the card says New work runs here, and Take away is refused
until another place is chosen. Press its terminal and folder buttons: a
terminal opens on the host, and the Files tab shows its folders.
Press its storage button: the form shows the defaults and says so; press
Escape and only the form closes. Open it again, type `../x` as the results
folder and Save: it is refused and the form stays. Type `/scratch/lab` as
the data folder and Save: the form says it is saved; open another project
and the same host shows the defaults again.
Ask the model to download a large file from that host into the project:
while it moves, a Transfers tray above the composer shows the file, bytes,
share done, rate and time left; fold it to one line and unfold it. When it
ends, it says Download complete and leaves a few seconds later; the run's
card in the Runs tab keeps the same meter.
In Settings, General, press Check for updates: it reads Checking… and
then says this is the latest version or names a newer one, with Open
releases opening the project's release page. Without a network it says
it could not check. Turn At start off, quit and start again: no check is
made; turn it on and start a build older than the latest release: a note
in the corner names the newer version, and Don't check at start turns
the switch off.
Open three conversations, one of them in another project: tabs appear
above the conversation from the second on. Click a tab of the other
project: its project and conversation open. Close the tab in view: its
neighbour opens; close a tab that is not in view: nothing else changes.
Delete a conversation and confirm its tab goes.
Start a slow turn, switch to another app and wait: a notification names
the conversation when the turn ends, and another when a tool asks for
approval. Stay in Wisp and confirm nothing is posted. Turn Notify me off
and Play a sound on in Settings, General: only the sound comes.
Press the import button in the conversation list's header: conversations
from Codex and Claude Code on this computer are listed; Import one, and it
appears in a group named after the tool, reads as it did there, and is not
listed under Needs you; press the row's button to open it. Import it again:
it says there is nothing new.
Branch a conversation from an answer, send a message in the branch, then
press Bring back: the dialog says how many messages the branch added; Write
the summary fills the field with the model's summary, which can be changed;
Bring it back opens the source conversation with the summary as its last
row, a Merged branch result card with the branch's title; press it to read
the summary and Open the branch to go there. Press Bring back in the branch
again: it says it was brought back already. Escape closes the dialog at any
step.
When the model proposes a new plan, its steps show above the composer: write
what should change in the box and press Return; the plan call shows as
refused and the model is told your words. Approve on another one and the
work starts.
Ask for a long piece of work that the model plans: its plan update shows as
a checklist, and the line above the composer says how many steps are done
and names the current one; it goes once every step is done.
After a turn that started a run, the run call in the transcript has a line
with the run's state that changes as the run does; Show in Runs opens the
Runs tab with that run's output open.
Open a PDF in the Files tab: after a moment its pages show as they look,
each labelled, under a line that says how many there are (a long one says
only the first twelve are drawn); Show the text gives its text and tables,
and Show the pages brings the pages back. Open a Clustal `.aln` file:
its sequences show one per row with their names, residues coloured by
kind, under a line with how many sequences and columns there are; the
arrows step through the columns a hundred at a time, and Show as text
gives the file. Open a `.fa` file of reads: the same, coloured by base. Open a `.sdf`
file: its bonds show with oxygen red and nitrogen blue; drag across it and
it turns; Face on turns it back. Open a `.pdb` file of a protein: each
chain shows as a coloured trace, and the line says how many residues.
Open a Word file: after a moment
it shows its text (and a spreadsheet its sheets as tables) under a note
that layout and pictures are not shown.
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
With the sidebar hidden press Cmd+Shift+F: it comes back with its search
field ready; type a word that only appears inside an answer and the
conversation holding it is listed.
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
Point an ACP agent at a program that speaks ACP. In a new conversation
press the composer's Wisp button: Wisp and the agent are listed; press
Escape (only the menu closes), open it again and choose the agent. The
button names the agent and is fixed, and the model, effort, plan first,
expert and review buttons are gone. Send a message: the agent's answer
streams in with its thoughts folded, its tool calls as rows and its plan
as a checklist with the plan line above the composer, and the title bar
names the agent. Ask it to do something that needs permission: the
approval card shows its tool and input; Reject, and the agent is told so.
Send something long and press Stop: it stops and nothing is added; send
again and it answers in the same session. With an agent that reports
modes or options, after its first answer the composer shows Mode and its
options; choose another mode or value: the button follows, and the next
message runs that way. Press "/": the agent's commands are listed, and
one is written into the message box.
In Settings, Models, the Subscriptions list names ChatGPT, ChatGPT
(Codex) and SuperGrok, each Not signed in. Press Sign in for ChatGPT: the
browser opens on the OpenAI sign-in and the panel waits, with a field for
the address the browser ends on; finish in the browser and the list says
Signed in as your account and a ChatGPT model is added (the default when
there was none). Send a message on it. Press Sign in with a code for
ChatGPT (Codex): a code shows with Open the sign-in page. Sign out: the
model stays, marked as not signed in. Started with `WISP_*` model
variables, the list says so and offers nothing.
With `codex login` done on this computer, press Import from Codex CLI
under ChatGPT (Codex): the account is listed with its plan and In use,
and the row says Signed in. Sign in with a second account: both are
listed; press Use on the other and send a message on a Codex model.
Press Check usage: each account shows its short and weekly windows with
the part used and when it resets. Press Remove, then Escape: the question
closes and Settings stays; Remove again and answer it: the next account
is in use.
In Settings, ACP agents, add an agent with a name and a program that is
not installed: it is listed with Program not found, and Test says its
program was not found on this computer. Change it to an installed ACP agent (or adapter)
with its arguments, one of them quoted: Test shows its name, version,
`ACP v1` and how it signs in. Press Add, then Escape: the form closes and
Settings stays. Remove it: the question has to be answered first.
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
With a workflow saved in Settings, Workflows, add a quick action, choose
Starts and that workflow, and save: its row says it starts the workflow.
In a conversation type a question and choose the action from the bolt
menu: a new conversation named after the workflow opens on its Agents tab
with the batch waiting for approval, and the composer is empty.
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
In a draft with a figure and no evidence press Finalize…: Private is
chosen. Press Check this version: "no_selected_evidence" and the figure
are listed under Stands in the way and Finalize and lock the evidence
stays off. Choose Public: the check is cleared and two reviews appear;
check again and both reviews are listed until they are ticked. Bind a
finished run to the figure and check: what the run lacks is listed;
press Let it stand… on each, give a reason and save: the check runs
again and shows the reason. When the verdict reads Ready to finalize,
finalize: the panel closes, the version reads Finalized with its
fingerprint, and Add evidence and Change are gone. Press New version:
"v2 · Version 2" opens as a draft with the same figure and evidence.
With the reason field open press Escape: only the field closes; press it
again: the panel closes and the page stays.
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
Open with the system's application do what they say, and Keep as an
artifact of this conversation lists it in the Artifacts tab (again: a new
version of it).
In the Artifacts tab press By folder: the files are under headings, the
top of the project first and files outside it last. Press a file's trail
button: the line names the tool and the conversation, with the code; press
it again and the line goes.
In a conversation type `/trajectory` and send: the trajectory opens and
nothing is added to the transcript. Send `/btw why did that fail?`: the
Aside tab opens and the question is asked there. Send `/fork`: a
branch opens under the conversation. Send
`/data/reads.fq looks odd`: it is sent as a message. Press the "/"
button, then Escape: only the list closes. Type `/re`: the menu lists
`/review`, `/remember` and `/rewind` and the skills that begin with
"re"; type `v`: only `/review` is left; press Escape and type more: the
menu stays shut. Type `please /` and pick a skill: it is attached and
"please " is left. Type `/tim` and pick `/timer`: the composer reads
`/timer `.
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
In Settings, Chat apps, press Bind with a QR code: a QR code shows with
a note to scan it. Scan it with WeChat on your phone and confirm: the
line reads Bound · off. Press Escape while the code shows: Settings
closes and the code is put away. Turn Answer messages on: the line
reads Connected. From the phone send `/help`: the commands come back.
Send a question: the reply asks for `/project`; send `/project 1`, then
the question again: the conversation shows in the desktop and the
answer comes back. Ask for a file to be written: an approval with a
code arrives; send `/approve` with it and the answer follows. From
another WeChat account send a message: nothing is answered. Press
Unbind, then Escape: only the question closes; Unbind again and answer
it: the line reads Not bound.
Under Research assistant on WeChat, bind a second bot the same way (the
first bot is refused here) and turn it on. Send `/help`: the assistant's
commands come back. Send a question about your projects: it appears in
the research assistant's conversation and its answer comes back. Send
`full`, then `/status`: full access reads on; `full off` turns it off.
Run a mandate that asks for something, then write to the bot: the
mandate's notice arrives with the reply. Unbind it and Escape at once:
only that card's question closes.
On the same page, under Feishu, turn Answer messages on with nothing
filled in: it is refused. Press Make the app by scanning and scan the
code with Feishu: the line reads Bound · off and the Owner field holds
your open_id. Turn it on: the line reads Connected. Send `/project 1`,
then a question: a card shows the progress and ends as the answer.
Empty the Owner field and save, then write to the bot: it says no owner
is confirmed, and Settings shows your open_id waiting with Make owner
and Refuse; press Make owner and write again: it answers. From another
account write to the bot: it says it answers only its owner. Unbind,
then Escape: only the question closes; Unbind and answer it: the line
reads Not bound and the App ID field is empty.
Under Wisp Lab, with a wisp-relay of your own started with `WISP_LAB=1`,
leave the invite code empty, give the relay's address, its token, your
name and a lab name, and press Found the lab: the line says you lead it
and you are listed. Make an invite code and copy it. On a second
computer (or a second data folder) paste the code, the token and a name
and press Ask to join: it waits for approval, and the first computer
lists it as waiting; Approve it. On the second computer choose a project
under Answer lab mail in. On the first, ask a conversation to send Lin a
question with `lab_send`: within a quarter minute the second computer
has a conversation named Lab: and the answer comes back into the asking
conversation. Choose None and ask again: the answer says that computer
takes no lab mail. Remove the member, then Escape: only the question
closes; remove it: the second computer reads removed and offers Forget
the lab.
In Settings, Remote access, switch it on with nothing filled in: it is
refused. Give a relay address that is plain http on another machine: it
is refused. With a relay of your own running, give its https address
and token and switch it on: the line reads Connecting, then Connected,
and a code shows. Copy the link and open it in a browser: the projects
are listed and the line counts one browser. Press Show the QR code and
scan it with a phone: the same page opens there. Press Reset code, then
Escape: only the question closes. Reset it: the browser is dropped and
the old link no longer works. Under StickS3 desk display give 0.0.0.0
and turn it on: it is refused. Give this computer's LAN address and turn
it on: the line reads Listening at that address; Show token and enter it
on the device (or `curl -H 'x-wisp-device-token: …' http://<address>:18766/state`):
the state is idle, working while a turn runs, done after it. Send the
device's focus action for that conversation: the window comes forward on
it. New token: the old one is refused. Turn it off: the line reads Off. Give a wrong token: the line says why it
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
Press Continue research: a conversation named "Continue · <title>" opens
with the report and where the materials are as its first message, and
the composer sends there.
Press Escape on a draft: nothing is written.
Switch Review turns on in the composer and have a turn run a tool: when
it ends the line above the composer reads Reviewing the answer…, then the
result. Press See: the findings open; Ask Wisp to correct puts them in the
composer. Send a short message: no review follows and the old result is
gone. Put a result away with its close button. In Settings, Conversation,
turn Review turns on and confirm a new conversation starts with the
switch on while one you switched off stays off; choose a Reviewer model
and confirm the Review dialog names it. With a model that cannot see as
the conversation's and one that can saved beside it, attach a picture and
ask what it shows: the answer describes it. Picture model lists only the
models that can see.
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
Under an answer press Quote: the message box keeps what was typed and
adds the answer as a quotation with the caret below it. Press Ask aside
under a multi-line answer: the Aside tab opens with the answer quoted on
one line in the question field, and nothing is sent until you press Ask.
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
the next conversation is asked again. Answer Always, in every project:
another project's conversation is not asked, Settings lists the tool under
Every project, and taking it back makes both ask again. In a project, open Connections,
unfold PubMed and set one of its tools to Deny: ask for a search that needs
it and confirm the call is blocked without a question; Permissions lists the
rule, and pressing it allows the tool again. Set it to Deny again and press
Skip approvals on PubMed: its rule buttons grey out and the search runs.
Choose Auto-approve and ask
for a file to be written: it is written without a question; choose Per-tool
rules again and it asks.
In Settings, Hooks, add a PreToolUse hook for `write` with the command
`exit 2` and ask for a file to be written: the call is blocked and the
transcript says why; switch the hook off and it is asked about again. Add
a UserPromptSubmit hook `echo use GRCh38` and ask which genome to use: the
answer names it. Change the hook to `exit 3`: the next message still goes
and the composer shows that a hook failed. In a project, write a
`.wisp/hooks.json` with a PreToolUse hook: the Hooks page lists it as not
trusted and a turn says it was not run; Trust it as it is now, and it
runs; change the file and it reads as not trusted again. Press Add a hook,
then Escape: only the form closes.
In Settings, Network, set the subscription sign-in route and save; open
the page again and confirm it is kept. Give a saved model the Route `none`
while the Model API route names a proxy that is not running: that model
still answers, another one does not.
In Settings, Connections, change an MCP server over HTTP and add the header
`X-Api-Key: <a made-up key>`; Test reaches it with the header; open the form
again: the line reads `X-Api-Key:` with no value, and saving it so keeps the
key. Remove the line and save: the header is no longer sent. For a server
that is a program, add `LAB_TOKEN=<a made-up value>` and confirm the program
sees it.
On the Plugins page install the same plugin from a `.zip` of its folder; press
From an address…, then Escape: only the form closes; give an HTTPS address
with a wrong SHA-256 and confirm it is refused.
Install a plugin whose `.mcp.json` names a server inside it, switch it on
for one project, and ask for its tool there: it asks, then answers; in
another project it is not offered.
In Settings, Skills, under Get skills, press Look at it on an entry of the
directory: the package shows with its repository and commit; Install puts it
in `~/.wisp/skills` and it is listed as Your own. Press From GitHub…, then
Escape: only the form closes. Refresh the directory with the network off:
the shipped directory stays, with a line saying why.
Open the side panel on Memos and press − beside it: Memos leaves the row and
the tab before it shows. Press +, then Escape: only the menu closes, the panel
stays. Press + and choose Memos: it is back in its place. Take every tab away:
the panel closes, and the header button opens it again on the last one. Take
Aside away, quit and start again: it is still away until + or a run's Ask
brings it back.
In Settings, Environments, under Where the tools are, each hint shows where
the search path finds that program. Type a path that does not exist for
Python and Save paths: it is refused and named; type the real one and save,
leave the page and come back: it is kept. Clear it and save: the hint is
used again.
In the Files tab press the tiles button: the folder shows as tiles, its
pictures as themselves; open a folder and a file from their tiles; restart
the app and the Files tab is still tiles; press the list button to go back.
In Settings, Experts, change an expert to Only these skills and pick one,
and Only these connectors with none picked: its row says so. Start a
conversation as it and ask for a skill it was not given and for a PubMed
search: neither is available, while a conversation without the expert has
both.
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
On a fresh data folder the first-run guide opens over the first screen:
Next walks through welcome, what Wisp can do, the model step and the
tools step; Back goes back. With a DeepSeek key typed, Next saves it and
Settings, Models lists both DeepSeek models with Flash as the default;
with nothing typed the button reads Set up later. Started with `WISP_*`
model variables, the model step says there is nothing to do. On the last
step press Escape: the guide closes and does not come back at the next
start.
Open Environments: Tools on this computer says nothing is installed and
names its folder under the data folder. Untick everything: Install is off.
Tick R, press Install: the steps count up with a log (Getting pixi, then
Installing R) while the page stays usable; press Stop: it reads Stopped
and Install is back. With a network, install Python and wait: it reads
Installed, Installed here lists uv and Python, and a conversation's
Python interpreter starts without a system Python.
Add an SSH host with a name and an address and confirm it
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
With a model set, press Draft a recap on today: a Draft card appears under
the day with the finding; Confirm turns it Confirmed, Dismiss removes it
and the day offers Draft a recap again. On the Scheduled tasks page the
Daily recap card shows On at 09:00; set 25:00 and Save: it is refused;
set 07:30 and Save: the field keeps 07:30 after the page is opened again.
With a model that can use tools and full access by default, press the
robot on the home page: the Research assistant conversation opens with a
bar saying what it is for and that nothing is planned. Ask it to plan
"check the alignment" for today: the bar lists the item; press its check
mark: it shows Done. Ask it to have a project count its files: a
conversation named for the work appears in that project, and when it is
answered a report appears in the assistant's conversation. New
conversation is off in the assistant's sidebar.
Press the gauge on the home page: Research mandates says there are none.
Press New mandate, give a goal and Save: a card shows it Active with its
next round. Press Run now: a conversation named after it appears in its
project; Open its conversation opens it. Press Edit, then Escape: only the
form closes. Press Delete, then Escape: only the question closes; press
Delete and confirm: the card goes and the conversation stays.
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
Press the folder-package button on a project's card and choose a folder: a
new folder named after the project is written there, and a second export
makes a second folder. On another data directory press Import and choose
that folder (not a file): the project opens where it is, with its
conversations. Do the same with a folder package the WebView exported.
Export a project with a few hundred megabytes of files: the home page's
line goes through Preparing, Looking through the files and Writing with
the files and bytes counting up and the file named, then Saved to.
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
confirm only the outline closes. In a conversation long enough that its
start is not loaded, open the outline and press Load earlier: the first
message you sent is listed as 1 and choosing it scrolls to it.
In a conversation press Cmd/Ctrl+F and type a word that appears several
times: the count is shown, the newest match is marked strongest, Return and
the up arrow go to earlier ones and around again, and Escape closes the bar,
removes the marks and leaves the conversation as it was. Type a word that
does not appear and confirm it says so.
Start a turn that takes a while and, before it ends, write two messages and
press Queue for each: both are listed above the composer and the composer is
empty again. When the turn ends the first is sent by itself, and the second
after that. Queue one more, take it back with the pencil and confirm it is in
the composer again; queue it and drop it with the cross. Queue a message,
open another conversation before the turn ends, and come back: the message
was sent when the turn ended. Queue a message
during a turn that ends with a question card and confirm it waits until you
answer or press "Send the next one". Start a turn that waits for an approval,
queue a message and press its lightning button: it leaves the queue; approve,
and the message shows after the tool's result with the answer to it below,
in the same turn.
In Settings, Remote access, under Project sync, give a relay of your own
and its token and Save. Open a project's settings: Sync reads Not
synchronized; press Sync now: it reads Published, then Synchronized
through the relay, just now. Copy device code. On a second computer (or
a second data folder) set the same relay, press Join a project from
another device on the home page, paste the code and choose a folder: the
project opens with its files and conversations. Change a file on each,
Sync now on one, then on the other: the second says both changed and
offers Use the other device's copy and Keep this device's copy; press
one, then Escape: only the question closes; answer it: the files match
the side chosen.
With a model that can use tools and full access in a conversation, ask
it to start a subagent that lists the project's files: a conversation
named after the task appears under it, with the instruction and a bar
saying it is watched; its composer does not send. When it ends, the
parent shows a report and its agent answers it without a new message of
yours.
With such a model and without full access, ask it to have two temporary
agents read the project, one after the other: a plan card lists the
tasks; approve it (a batch that only reads with a local model is not
asked about). Open the side panel's Agents tab while it runs: the batch
shows Running, one task after the other turns Done with its summary,
and each task's chat button opens a conversation named after the task
whose composer does not send. The answer sums up both. Ask for another
batch and press Stop on its card: the batch and its running task show
Stopped.
Ask for the same work in the background: the turn ends at once saying
it started; a little later a delegate_tasks_completion row appears in
the conversation and the agent answers again without a new message of
yours.
On the batch you stopped, press Change under a task that did not finish
and press Escape at once: only the field closes. Press Change again, type
`50000` and press Return: the line reads Runs again with up to 50000
tokens. Type `lots` instead: an error asks for a whole number. Press Run
again: its tasks that had finished stay Done, the others run, the changed
task's line reads Up to 50000 tokens, and its result arrives in the
conversation the same way.
With a real model, ask for a batch in which one task leads the work and
may hand parts of it on (the `delegation` capability): the plan card lists
it; approve. Open that task's conversation from its card: its Agents tab
lists a batch marked Nested whose tasks are named after it, and each of
those opened shows no further delegation.
In a project that is a clean Git checkout, ask for a task that changes a
file to run isolated: its card shows Own worktree, and once it is Done
the file is changed in the project and `git log` shows a new commit by
Wisp Science Agent. Change the same line yourself before such a task
ends: the task fails, the project keeps your line, and its conversation
has an `isolated-agent-….patch` artifact.
In a project with a file `notes.txt`, ask something and wait for the
answer; on it press Start an exploration from here, keep the offered
name and Start: a conversation opens with the same history and a bar
naming the exploration. Ask it to change `notes.txt`. Back to the
mainline: the bar there lists the exploration, the composer refuses to
send, and the mainline's `notes.txt` is unchanged. Start a second one
from the same answer, open it and Discard, then Escape: only the
question closes; Discard and answer: it goes. Open the first, View
changes and keep: `notes.txt` is listed as modified; Keep it as the
mainline: the mainline conversation opens with the exploration's turns
and `notes.txt` changed. Start one more and Abandon the round: the
mainline is as it was.
On a project's card press Project settings: clear the name and Save (it is
refused and the dialog stays), then give a name, a description and a rule
and Save: the card and the recent conversations show the new name, and
`.wisp/WISP.md` in the project holds the rule. Send a message in a
conversation that was already open and confirm the model follows the rule;
empty the rules, save, and confirm the file is gone. Type 0 in Failed
runs and Save: it is refused and nothing is saved; type 14 and Save, open
the dialog again and confirm 14 is there, and that the WebView's Project
settings shows the same. Press Escape with the
dialog open and confirm only the dialog closes. Inside a project, find
Project settings in the command palette.
Press the book on the home page, and Documentation and Known problems in
Settings, and Documentation in the command palette: each opens in the
system's browser, the first two the tutorials, the last the issue tracker,
and the app stays where it was. Press Report a problem in Settings, and
again from the home page through the palette: a new conversation opens (in
the built-in project from the home page) with the Wisp version, system,
architecture and model in the composer, and nothing is sent until you add
what happened and send it.
In Settings type a word in the search field above the pages, for example
"proxy" or "ssh": only the pages about it stay listed, Return opens the
first, and a word no page is about says so. Close Settings and open it again:
every page is listed.
In General, type a relative path as the workspace folder and Save: it is
refused. Choose… a folder and Save: the note says it is used from the next
start; quit and start again (without `WISP_GPUI_WORKSPACE`): the built-in
project's Files tab shows that folder. Clear the field and Save to go back.
Turn Each project its own database on, make a new project: its folder
holds `.wisp/project.sqlite`; turn it off and the next one does not.
With the WebView desktop's data on this computer (or with its
`wisp.sqlite` copied under `$HOME/Library/Application
Support/science.wisp-science/wisp-science/` of a scratch `HOME`), press
Bring over on the home page: its projects are listed with their folders and
conversation counts, the built-in one marked Already here, and under
Settings the groups the WebView has set and this desktop has not, and
under Saved models its models with whether their key comes (start without
`WISP_*` model variables, with a scratch `HOME`, so no real key moves).
Untick one project and one group and press Bring over: the line says how
many projects and models came and which settings, the home page lists
them, their conversations open, the settings show on their pages, and
Settings → Models lists the models. Open the dialog
again: the projects are now marked Already here and the groups that came
are gone. Press Escape: it closes. The WebView's database file is
unchanged.
In a conversation of more than twelve turns, ask aside about something
said early on, by a word it used: the answer cites [E1] and Earlier
passages lists that message with the matched words. Ask with only common
words ("why was it?"): no passages are listed.
Press ⌘⇧H: the privacy dialog lists the projects; press Escape and it
closes. Open it again, tick one and Hide selected: its card and its recent
conversations leave the home page, and the palette no longer finds it.
Restore all brings them back. Turn it on here and open the WebView
desktop: the same project is hidden there.
In the command palette type "dark": Dark appearance switches the window;
type "larger": Larger text grows the text and is kept; type "environments"
inside a project: the side panel opens on that tab; "first-run" shows the
guide again from its start.
In General, under Which sound, press Submarine: it plays, and the next
notification sound is Submarine.
In General choose the GitHub light palette: the window's accent, page and
text change at once; switch to Dark and choose Gruvbox: the dark window
wears it. Press the text size's + twice: the interface's text grows at
once; Reset goes back. Type a font installed on this computer for code and
Apply: code blocks use it. Quit and start again: all of it is kept.
Open General: choose Dark, Light and System and confirm the whole window
changes at once; with System chosen, switch the system's appearance and
confirm the window follows, and with Dark chosen that it does not. Choose
Where I left off, open a conversation, quit and start again: the conversation
is open. Choose Home page and confirm the next start opens on Home.
Restart and confirm persisted messages. Close during a running tool and verify
worker cleanup. Check missing credentials and invalid workspace errors. Validate
each target OS; a macOS build does not establish Windows/Linux behavior.
