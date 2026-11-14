# GPUI parity ledger

This is the working register for replacing the WebView desktop (`ui/` +
`src-tauri/`) with the GPUI desktop (`apps/gpui`). The goal is functional
parity: every function a WebView user has must be reachable in GPUI. The two
do not have to look alike. [GPUI desktop](gpui-desktop.md) describes how the
GPUI build works; this page records what is and is not ported.

Update the row in the same change that ports a feature. Do not mark a row
`Done` until it has been used in the running app, not only compiled.

## How the baseline was taken

The WebView column was recorded on 2026-11-14 by clicking through the
installed macOS release, **v1.24.0**: the home page, both project kinds (the
read-only sample project and a normal project), every sidebar entry, every
top-bar button, the right panel launcher, both composer menus and all twenty
Settings sections. Nothing was changed in that installation. Screens that
need live data to show anything (a running turn, an SSH host, a paired phone)
were read from `ui/src` instead and are marked *(source)*.

The walk-through was then checked against the sources: the 489 commands in
`generate_handler!` (`src-tauri/src/lib.rs`), the string table in
`ui/src/i18n.rs` and the spec names under `ui-tests/tests`. Features found
only that way are in section 10.

Chinese names in brackets are the labels the WebView shows, to make rows easy
to find in the running app.

## Status words

| Word | Meaning |
| --- | --- |
| Done | Usable in GPUI and exercised in the running app |
| Partial | Some of it works; the gap is written in the row |
| Missing | Not in GPUI |

**Host** says where the logic GPUI needs lives today:

| Word | Meaning |
| --- | --- |
| served | `wisp-server` already answers it |
| store | Persistence exists in `wisp-store`; needs a typed method in `crates/wisp-server/src/desktop.rs` |
| shared | A shared crate has the logic (`wisp-runs`, `wisp-skills`, `wisp-mcp`, `wisp-sync`, …); needs wiring |
| tauri | Only in `src-tauri`; must move to a host-independent crate first |
| ui | No host work; the feature is client-side |

## 1. Home page

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Project list with conversation and artifact counts, last-used time [项目] | Yes | Done | served |
| Open a project | Yes | Done | served |
| Star a project so it stays first [星标项目并置顶] | Yes | Done | store |
| New project [新建项目] | Name, folder | Partial: opens an existing folder; no name or description form | served |
| Import project [导入项目] | Archive import | Partial: a zip archive, placed in a folder you choose; no folder packages and no progress display | tauri |
| Export project [导出项目] | Archive export | Partial: as a zip archive in the WebView's format; no folder packages and no progress display | tauri |
| Project settings [项目设置] | Name, description, sync | Partial: name, description and the rules for the model (`.wisp/WISP.md`), from the project's card and the command palette; no sync or run retention | store |
| Remove a project from Wisp, files kept [删除] | With confirmation | Done | store |
| Delete a project together with its local data | Second choice in the same dialog, with a countdown | Done: second choice in the same question; the project's name is typed instead of a countdown, and a folder that holds more than the project is refused | tauri |
| Open in a new window [在新窗口打开] | Yes | Partial: a new window opens on the home page; a project cannot be sent to it from its card | ui |
| Recent conversations across projects with status [最近会话] | Yes | Done | store |
| Sample project, read-only [示例项目] | Yes | Missing | tauri |
| Research assistant [科研助理] | Scheduled tasks and plans | Missing | store |
| Research calendar [研究日历] | Month grid, per-project filter, day detail | Done: month grid with counts, projects can be left out, and the chosen day's records | store |
| Favorites library [收藏库] | Saved code, figures, highlights; search and kind filter | Done: a page of its own; saved snippets cannot be edited into versions | store |
| Command palette [搜索, ⌘K] | Projects, files, conversations, commands | Partial: finds projects, files and conversations in every project and runs GPUI's own commands; no open-in-new-window or attach-from-result, and the commands for features not ported yet are absent | served + ui |
| Documentation [文档] | Opens the docs | Done: from the home page, Settings and the command palette, in the system's browser | ui |
| Settings entry | Yes | Done | ui |

## 2. Workspace sidebar

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Back to home, project name, collapse sidebar | Yes | Partial: no collapse | ui |
| New conversation [新建会话, ⌘N] | Yes | Done | served |
| Search conversations by title and message text | Yes | Done | served |
| Conversation list with status | Yes | Done | served |
| Needs-you inbox across projects | Bell with count [N 个待处理] | Done | served |
| Pin | Yes | Done | store |
| Rename | Yes | Done | store |
| Archive and restore [已收起的对话] | Yes | Done | store |
| Delete with confirmation | Yes | Done | store |
| Folders [新建分组]: create, rename, reorder, move a conversation | Yes | Done: reorder is one place at a time, no drag | store |
| Multi-select and batch actions [选择] | Yes | Done: move to group, archive or restore, delete | store |
| Sort and group options [排序与分组] | Yes | Done: date sections are elapsed spans, not calendar days | ui |
| Copy or move a conversation to another project | Yes *(source)* | Partial: transcript only; moving its artifacts along is not ported | store |
| Files entry [文件] | Opens the Files panel | Done | served |
| Research journey [研究历程] | Section 7 | Partial: see section 7 | store |
| Memos [备忘录] | Section 6 | Done | store |
| Paper evidence [论文证据] | Section 7 | Missing | store |
| Favorites [收藏库] | Section 1 | Done | store |
| Capability summary [能力]: skills, MCP, memory counts, local tools found | Dialog | Done: counts of skills, connectors, MCP servers, SSH hosts and memory, and the tools found on the search path | shared |
| Feedback [反馈] | Yes | Partial: "Report a problem" in Settings and the command palette opens the issue tracker; the report the model drafts with the app's version and system is not ported | ui |

## 3. Composer

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Multiline text, IME, drafts kept per conversation | Yes | Done | ui |
| Send and stop | Yes | Done | served |
| Send key choice (Enter or modifier+Enter) | Setting | Partial: modifier+Enter only | ui |
| Queue a message while a turn runs | Yes *(source)* | Partial: up to ten per conversation, sent in order as turns end, with take back and drop; kept in the window only, and no cut-in that steers the running turn | tauri |
| Attach files, images, PDF [附加文件] | Yes | Partial: any file through the system dialog, copied into `uploads/` when it is outside the project; the model gets the paths, not the image pixels | served |
| Browse project files to attach [你的文件] | Yes | Done: found by name in the add-context box | served |
| Paste or drop files | Yes | Partial: files dropped onto the conversation are attached; no paste from the clipboard | tauri |
| `@` artifact, `#` conversation, `/` skill mentions | Yes | Partial: conversations and skills, picked from the add-context box rather than typed inline; a conversation is passed as a recent excerpt, not through the reader agent; no artifacts, environments, runtimes, workflows or memos | served |
| Model picker per conversation | Yes | Done: saved API models, or follow the default | served |
| Reasoning effort / fast mode | Yes | Partial: effort per conversation, kept across restarts; no fast mode | store |
| Permission mode [默认 / 完全权限] | Yes | Done: Ask first or Full access per conversation, with a question before it turns on; dangerous shell commands still ask | served |
| Plan first [先计划] | Yes | Done: the model investigates and submits a plan; tools that change things refuse until the plan is approved or plan mode is left | served |
| Expert (persona) for the conversation [专家] | Yes | Partial: chosen in the composer before the first message; its instructions join the system prompt; an expert's own model and skill or connector lists are not applied | tauri |
| Automatic review toggle and review model [自动审查] | Yes | Partial: per conversation, with a default and a reviewer model in Settings; the result shows above the composer and corrections are asked for by hand, not made automatically | tauri |
| Sub-agent model [子智能体模型] | Yes | Missing | store |
| Compute environment for the conversation [计算环境] | Yes | Partial: the default, this computer or a saved SSH host, for where runs start; Python, R and the shell stay on this computer | store |
| Context usage meter and manual compaction | Yes | Partial: how full the context is, as a percentage that opens the list of what the model is sent, message by message; no manual compaction | served |
| Request a review of the conversation [请求审查] | Yes | Partial: a Review dialog lists what the reviewer model finds and puts the corrections in the composer; the report is not kept with the conversation | tauri |
| Share as long image or HTML [分享为长图] | Yes | Partial: Export saves the conversation as a Markdown file; no long image or HTML | ui |
| Save the conversation as a skill [保存为技能] | Yes | Done: puts the request for the `skill-creator` skill in the composer, after anything already typed | shared |
| Interactive runtime status (Python / R) [Local · Python · R] | Yes | Partial: the conversation's Python and R interpreters above the composer with their state, and Stop; no variable list, restart or memory limit | shared |

## 4. Transcript

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Markdown, code, tables, selectable text | Yes | Done | ui |
| Paginated history | Yes | Done | served |
| Tool calls: input, output, status, time | Collapsible cards | Done: one folded line each, click to unfold | served |
| Approvals: allow once / deny | Plus allow for session / always | Partial: once, or for the rest of the conversation (forgotten on restart, can be revoked); no project-wide or permanent rules | served |
| Agent question card with free-text answer [Agent 需要你的决定] | Yes | Done: options answer with one click, or type the answer | served |
| Follow-up suggestions | Yes | Done: up to three questions above the composer after an answer; picking one puts it in the composer | served |
| Thinking blocks | Yes | Partial: shown above the composer while the model thinks; not kept in the transcript afterwards | served |
| Images and figures inline | Yes | Partial: project pictures an answer shows with Markdown image syntax, drawn under the answer; not pictures only a tool wrote, and no zoom | served |
| Token usage per turn | Yes | Partial: the running or latest turn only, and not after a restart | served |
| Skill and attachment chips on the user message | Yes | Partial: file chips; references are context for that turn only and are not shown afterwards | served |
| Copy message [复制] | Yes | Done: copies the message as Markdown | ui |
| Rewind to a message [回溯 / 撤销] | Yes | Partial: goes back and returns the text to the composer; written files are not undone | store |
| Branch from a message [分支] | Yes | Partial: a copy through that turn, nested under its source; no branch card or merge back | store |
| Save to memory [记忆] | Yes | Partial: the message is opened for trimming and saved for the project or as a habit; no model-written suggestion | store |
| Review a turn [审查] | Yes | Partial: a review button under each answer reviews the conversation as it stood when that turn ended; the report is not kept with the turn | tauri |
| Select text: add to chat, ask aside, highlight | Yes | Partial: a whole message is saved with its star; a selected passage cannot be, and there is no add-to-chat or ask-aside | store |
| Full record / model view toggles [完整记录 / 模型视角] | Yes | Partial: the transcript is the full record; What the model is sent lists the model's working set in a dialog, with each message's share; no context breakdown by kind and no undo of a compaction | served |
| Conversation outline [展开对话目录] | Yes | Partial: your messages in the loaded part of the conversation, to jump to | store |
| Find in conversation | Yes *(source)* | Done: Cmd/Ctrl+F over the loaded transcript, with a count and next and previous; marks rows, not words | ui |
| Sub-agent and delegation cards | Yes *(source)* | Missing | store |

## 5. Top bar

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Conversation tabs | Yes | Done: the conversations opened in a window show as tabs above the conversation, across projects; not kept across restarts | ui |
| Share conversation [分享对话] | Yes | Partial: as an exported Markdown file; no share link | tauri |
| Trajectory [轨迹] | Yes | Done: a dialog with the totals and every turn's steps; a step opens to its whole text | store |
| Research archive [研究归档] | Yes | Missing | store |
| Pending decisions [N 个待处理] | Yes | Done (sidebar inbox) | served |
| Terminal [打开终端] | Yes | Missing | tauri |
| Toggle right panel [切换面板] | Yes | Partial: Files, Runs and Artifacts tabs; the other tools are not ported | ui |

## 6. Right panel

The WebView panel is tabbed; a launcher [开始] opens each tool as a tab.

| Tool | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Workspace files [工作区文件]: browse, preview, before any conversation exists | Yes | Done | served |
| Files: name search | In the panel | Partial: through the command palette, across projects | served |
| Files: upload, new file, new folder, rename, delete, sort, grid view | Yes | Partial: new folder, new file, rename, and delete as a move to the project's own trash; no upload into a folder, sort or grid | tauri |
| Files: other locations (SSH / WSL contexts) | Yes | Missing | shared |
| Files: image, PDF, Office, notebook, table previews | Yes | Partial: pictures (PNG, JPEG, GIF, WebP, SVG, BMP) up to 32 MB; no PDF, Office, notebook or table preview | ui |
| Agents [Agents]: sub-agents, parallel exploration, workflows | Yes | Missing | store |
| Artifacts [产物]: figures, tables, outputs, star, open | Yes | Partial: files a turn wrote are recorded with a snapshot, listed per conversation or project, and open in Files; no grouping, versions, download, star or provenance | served |
| Environments [环境]: conversation environment, attach servers, probe, terminal | Yes | Partial: the conversation's environment is chosen in the composer and hosts are checked in Settings; no panel tab, attaching or terminal | store + shared |
| Runs [运行]: list, output | Yes | Done: a Runs tab with status and output tails | served |
| Runs: cancel | Yes | Done | served |
| Runs: harvest, transfer, full log, workspace review | Yes | Partial: the whole log of a run on this computer; no harvest, transfer or workspace review, and no whole log for runs on SSH hosts | shared |
| Notebook [Notebook]: code cells and outputs | Yes | Done: a panel tab with the loaded conversation's cells and outputs, copy, save to favorites and show in the conversation | store |
| Highlights [划线] | Yes | Done: lists what the conversation saved, with use, copy and remove | store |
| Provenance [溯源]: tool evidence behind a claim | Yes | Missing | store |
| Side question [侧边问答]: ask without changing the main thread | Yes | Partial: an Aside tab where the conversation's model answers from its latest turns; no evidence list, no question scoped to a run or to selected text | tauri |
| Memos [备忘录]: private notes the model cannot read | Yes | Done: no search, no `/memo` command | store |

## 7. Project pages

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Research journey [研究历程]: by day, relation graph, search, add record, recap, artifact sources | Yes | Partial: a month at a time by day, a filter, and adding a record of your own; no relation graph, recap or artifact sources | store |
| Paper evidence [论文证据]: papers, versions, figures and tables with their sources | Yes | Missing | store |

## 8. Settings

| Section | WebView | GPUI | Host |
| --- | --- | --- | --- |
| General [常规]: language, workspace directory, per-project storage, resume last conversation, send key, selection quick actions, notifications, sound, update check, local tool paths | Yes | Partial: resume the last conversation at launch; the rest is not ported | tauri |
| Network [网络]: proxy per channel (model API, account login, MCP, code requests), package mirrors | Yes | Partial: routes for model requests, connectors and MCP, and code; conda mirror, Python index and CA bundle; no sign-in route, and no route of its own for one model or one MCP server | tauri |
| Conversation [对话]: iteration limit, auto-continue, auto-compaction and threshold, follow-up questions | Yes | Partial: steps per turn, automatic compaction and its threshold, follow-up questions, review of turns and the reviewer model; no auto-continue | tauri |
| Appearance [外观]: system/light/dark, theme, accent, fonts and sizes, live preview | Yes | Partial: system, light or dark with the Wisp palettes, applied at once; no other themes, accent, fonts or sizes | ui |
| Pet [宠物] | Yes | Missing | ui |
| Models [模型]: API models with provider, URL, key, limits, effort | Several profiles, presets, default | Done: any number of models, the six presets, a default, a connection test | served |
| Models: ACP agents, subscription accounts, decision model | Yes | Missing | tauri |
| Quick actions [快捷动作] | Yes | Missing | store |
| Workflows [工作流]: library, graph editor, approval policy | Yes | Missing | store |
| Experts [专家]: built-in and custom, default sub-agent model | Yes | Partial: an Experts page lists the built-in illustrator and your own, with add, change and remove; no model per expert, no skill or connector lists, no sub-agent model | tauri |
| Memory [记忆]: project notes, global habits, enable switch | Yes | Done, as a side-panel tab rather than a Settings page; no "clear all", automatic proposals or weekly tidy | store |
| Hooks [钩子]: automatic review, tool-failure analysis, command hooks | Yes | Partial: automatic review and follow-up questions, as switches on the Conversation page; no Hooks page, tool-failure analysis or command hooks | tauri |
| Skills [技能]: list, tags, enable, community, GitHub, reload | Yes | Partial: list with source and tags, search, on or off, reload; no tag editing, community catalog or GitHub install | shared |
| Plugins [插件] | Yes | Missing | store |
| Browser [浏览器]: auto-open, auto-close tabs, block and prefer lists | Yes | Missing | tauri |
| Connections [连接]: built-in bio tools and user MCP servers, enable, add | Yes | Partial: the bundled connectors, and your own MCP servers over HTTP (with a token) or as a local program, with test and on/off; no OAuth, extra headers, environment variables or MCP Apps | shared |
| Remote access [远程接入]: project sync relay, Feishu, WeChat, StickS3, web access, Wisp-Lab | Yes | Missing | shared |
| Credentials [凭据]: service keys in the keyring, custom env keys | Yes | Partial: the NCBI and OpenAlex keys the bundled connectors use; no other service keys, no custom environment keys | store |
| Permissions [权限]: approval mode, remembered approvals | Yes | Partial: how new conversations start, and tools always allowed per project with revoke; no per-tool allow/ask/deny rules or global grants | tauri |
| Environments [环境]: default, add SSH host, import `~/.ssh/config`, paths, probe | Yes | Partial: SSH hosts with key, agent or config sign-in (add, change, remove, add from `~/.ssh/config`), a connection check that records what the host is, and where new work runs; no password sign-in, WSL, interpreter paths or storage locations | store + shared |
| Storage [存储]: data directory, per-project usage breakdown | Yes | Partial: the data folder with a button to show it, database and favorites sizes, and what Wisp keeps in each project; nothing can be moved or cleaned | store |
| Usage [用量]: tokens by day, model, workspace, skill/MCP ranking | Yes | Partial: totals, thirty days, by model and by project, counted from now on; no skill or MCP ranking | store |
| Settings search | Yes | Partial: finds pages by their name and by what they are about; it does not find or jump to a single setting inside a page | ui |

## 9. Across the app

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| One Escape stack, topmost layer first | Yes | Done | ui |
| Read state: answer marked read when on screen | Yes | Done | served |
| Background status for conversations not on screen | Pushed | Done (2 s poll while a turn runs) | served |
| Shared icon set | Lucide-style | Done (same drawings, `apps/gpui/assets/icons`) | ui |
| Chinese and English | Yes | Partial: a Language choice in Settings (System, English, 中文); the fixed text of every page, panel and dialog, the names of tabs, commands and settings sections the placeholders of text fields, statuses, counts, ages, dates and most sentences with a value in them are in Chinese; a few figures (usage, storage, step notes) and what the host says are still English | ui |
| Desktop notifications and sound | Yes | Partial: for the open conversation, when a turn ends or a tool waits for approval while the window is in the background; one fixed sound; no notification on Windows, none for conversations that are not open | ui |
| Update check and install | Yes | Partial: Check for updates in Settings says whether a newer release exists and opens the release page; no automatic check and no in-app install | tauri |
| Several windows | Yes | Partial: any number on the same host, each with its own page and conversation; not restored at the next start | ui |
| Keyboard shortcuts (⌘N, ⌘K, ⌘B, ⌘F, ⌘⇧F, ⌘P, ⌘,) | Yes | Partial: ⌘K, ⌘N and ⌘, are wired and unit-tested, not yet pressed in the running app | ui |
| Import existing WebView data | n/a | Missing: GPUI keeps its own database | store |

## 10. Also in the WebView

Found in the sources rather than on screen; all `Missing` in GPUI.

| Area | Feature | Host |
| --- | --- | --- |
| Home | Search across projects, conversations and recent artifacts (projects, conversations and files are in the palette; artifacts are not) | store |
| Home | Import a folder in place, a ZIP, or recover conversations from a workspace; transfer progress | tauri |
| Home | Sync now, device code, conflict resolution (use remote / use local), cloud-folder badges | shared |
| Sidebar | Conversation menu: copy title, export conversation, export debug request, reload project rules | tauri |
| Sidebar | Nested children: branches are nested — **Done**. Sub-agents, explorations and "load earlier" paging — Missing | store |
| Sidebar | Update-available card; "done / failed in another project" notices | tauri |
| Composer | Slash commands: `/timer` `/archive` `/compact` `/fork` `/btw` `/rewind` `/review` `/remember` `/context` `/plan` `/permission` `/save-as-skill` `/skills` `/files` `/upload` `/share` `/trajectory` `/memo` | tauri |
| Composer | Conversation timer: repeat a prompt on an interval, pause, resume, cancel | store |
| Composer | Queue panel: edit, remove, reorder, "guide now" | tauri |
| Composer | Plan progress strip; read-only states (sample, archived notebook, sub-agent, frozen branch) | served |
| Composer | ACP agent conversations and their mode / config options | tauri |
| Transcript | Equations, table copy, internal file links, confirm before opening an external link | ui |
| Transcript | Generated-artifact strip (the files are in the Artifacts tab, not yet under the message); image and video generation cards | store |
| Transcript | Run cards: send to background, cancel, harvest outputs, details | shared |
| Transcript | Compaction markers, undo compaction, stream-recovery and reviewer banners | tauri |
| Transcript | Undo a turn (restore or remove the files it wrote) | store |
| Transcript | Quote a range of turns into another conversation; distill an experience | tauri |
| Transcript | Explorations: start, diff, promote to mainline, discard, abandon | store |
| Transcript | Branch cards and merge a branch back with a summary | store |
| Transcript | Plan card: approve and run, or leave plan mode — **Done**. Plan review with per-step feedback, and the execution-plan progress strip — Missing | served |
| Transcript | Reviewer findings panel and the automatic correction pass | tauri |
| Transcript | MCP App previews; Motif DNA selection | tauri |
| Context | Usage breakdown panel, compact dialog (regular / semantic), handoff note to a new conversation, too-long recovery | tauri |
| Top bar | Find in conversation (⌘F); search the project's chat history (⌘⇧F) | store |
| Top bar | Import Codex and Claude Code conversations; import a conversation archive | tauri |
| Centre | Open a file as a centre tab, split chat beside a document, snapshot, open in an external editor | ui |
| Files | Copy path, attach to chat, register as an artifact, reveal in the file manager, open with the default app | tauri |
| Previews | PDF, images with zoom and region pins, CSV/TSV, DOCX/XLSX/PPTX, LaTeX, HTML, notebooks, structures, molecules, alignments, FASTA, trees | ui |
| Previews | Code editor: save, run the selection or the script in a runtime | shared |
| Previews | DOCX review: accept or reject tracked changes | tauri |
| Runs | Review a remote run's workspace: download, delete, clean up; remote files pane | shared |
| Environments | Add or edit an SSH host (key or password, ProxyJump, test), import WSL, interpreter paths, storage locations | store + shared |
| Runtimes | Python / R runtime cards: start, stop, restart, object inspector, console, plots | shared |
| Terminal | Several terminals, choose the environment, collapse, close | tauri |
| Agents | Swarm progress, agent workflow runs (draft, approve, run, cancel, retry), method-search runs | store |
| Library | Versions of saved code, edit code, insert into chat | store |
| Publication | Revisions, manuscript items, evidence binding, readiness waivers, reproduction check, capsule | store |
| Assistant | Assistant chat, saved plans, scheduled tasks, daily recap, weekly memory tidy, research mandates, WeChat binding | store |
| App | Onboarding wizard, toolchain installer (uv, Python, R, Node, pixi), local tool detection | tauri |
| App | Action palette (⌘P), privacy mode (⌘⇧H), application and native menus | ui |
| App | Browser automation prompts (tab cleanup, needs a human), browser-extension update banner | tauri |
| App | SSH connectivity check, generic confirmation dialogs | shared |

`src-tauri` also carries a second, already-split command contract: the
`native_*` families defined in `crates/wisp-dto/src/native_*.rs`
(conversation panels, projects, journey, calendar, library, memos,
publication, runtimes, settings). `wisp-server` serves eleven of those
commands today. Prefer adopting that contract over inventing new shapes when a
row above is ported.

## Order of work

Each step is one bounded change with tests, in this order. The order follows
how often a researcher touches a surface in a working day, and puts the frame
before the things that hang in it.

1. **Frame** — Wisp palette and icons, home page, workspace sidebar and top
   bar, folded tool calls. *(done)*
2. **Command palette** — ⌘K over projects, conversations, files and commands;
   project files without a conversation. *(done)*
3. **Conversation controls** — effort per conversation, permission mode,
   question card, approvals for the conversation, copy, context and token
   usage. *(done)* Three things planned here moved: the model per
   conversation goes with step 4, and plan first and thinking blocks are
   step 4a, because both need a contract the embedded host does not have yet
   (the plan-mode prompt and plan card live in `src-tauri`; reasoning text is
   not in the snapshot).
4. **Model profiles** — several saved models, presets, default, connection
   test, and the model per conversation. *(done)*
4a. **Plan first and thinking** — plan mode with its plan card; reasoning
   shown while it streams. *(done; thinking is not kept after the turn)*
4b. **Follow-up suggestions** — what could be asked next, after an answer.
   *(done)*
5. **Attachments and mentions** — files, images, `@`, `#`, `/`. *(done for
   files, conversations and skills; paste, drop, images as model input and
   inline triggers remain)*
6. **Right panel as tabs** — artifacts, run control, environments, notebook.
   *(tabs, artifacts and run cancel done; 6a done in the composer: where a
   conversation's work runs; 6b done: the notebook; 6c done: questions
   asked aside)*
7. **Conversation organisation** — folders, batch actions, sort and group,
   move between projects. *(done; rewind and branch are step 7a)*
7a. **Rewind and branch** — edit and resend from an earlier message, branch a
   conversation, show branches under their parent. *(done; branch cards and
   merging a branch back remain)*
8. **Memory, memos, highlights, favorites.** *(done as 8a memos, 8b memory,
   8c highlights and favorites)*
9. **Settings** — conversation, permissions, hooks, skills, connections,
   credentials, network, environments, storage, usage, general. *(9a done:
   the pages frame, Conversation and Permissions; 9b done: Skills; 9c done: bundled connectors; 9d done: Usage and Storage; 9e done: Credentials; 9f done: General with appearance and launch; 9g done: your own MCP
   servers; 9h done: Environments, SSH hosts and the default; 9i done: Network, routes
   and package sources; 9j done: checking an SSH host)*
10. **Project pages** — research journey, calendar, paper evidence, research
    assistant, trajectory, archive. *(10a done: the research journey; 10b done: the
    research calendar; 10c done: the trajectory of a conversation)*
11. **Project transfer and sync** — import, export, relay sync, remote access. *(11a done:
    export and import as an archive)*
12. **Experts, workflows, quick actions, ACP, accounts, plugins, browser.**
13. **Finish** — localization, notifications, several windows, updates,
    data import from the WebView desktop, installers.
