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
| New project [新建项目] | Name, folder | Yes: New project on the home page takes a name, a description and whether to lay the folder out in the standard research layout, then where to put it, and makes a folder named after the project there; Open folder opens an existing one. The rules for the model are set in Project settings afterwards | served |
| Import project [导入项目] | Archive import | Partial: a zip archive, placed in a folder you choose; no folder packages and no progress display | tauri |
| Export project [导出项目] | Archive export | Partial: as a zip archive in the WebView's format; no folder packages and no progress display | tauri |
| Project settings [项目设置] | Name, description, sync | Partial: name, description and the rules for the model (`.wisp/WISP.md`), from the project's card and the command palette; Sync now, device code, conflict resolution and cloud-drive folder saving; no run retention | store |
| Remove a project from Wisp, files kept [删除] | With confirmation | Done | store |
| Delete a project together with its local data | Second choice in the same dialog, with a countdown | Done: second choice in the same question; the project's name is typed instead of a countdown, and a folder that holds more than the project is refused | tauri |
| Open in a new window [在新窗口打开] | Yes | Yes: New window (⇧⌘N, the menu or the palette) opens on the home page, and a project's card or its row in the palette opens it in a new window | ui |
| Recent conversations across projects with status [最近会话] | Yes | Done | store |
| Sample project, read-only [示例项目] | Yes | Partial: the home page makes a sample project (made-up data, a script, a README of things to ask) in a folder you choose; it is an ordinary project to work in, not a read-only tour with prepared conversations | tauri |
| Research assistant [科研助理] | Scheduled tasks and plans | Partial: a Scheduled tasks page across projects (every few hours, daily or weekly; run now, on or off, delete; templates) that fires into a new conversation while the app is open, the daily recap run with its time, switch and Run now, the assistant's conversation with its plan for the day and dispatching to projects, and the weekly habit tidy on the Memory tab; and research mandates with their page, rounds, requests and reports | store |
| Research calendar [研究日历] | Month grid, per-project filter, day detail | Done: month grid with counts, projects can be left out, and the chosen day's records | store |
| Favorites library [收藏库] | Saved code, figures, highlights; search and kind filter | Done: a page of its own; saved snippets cannot be edited into versions | store |
| Command palette [搜索, ⌘K] | Projects, files, conversations, commands | Partial: finds projects, files and conversations in every project and runs GPUI's own commands; a project found there opens in a new window from its row, and a file of the open project or another conversation joins the message from its row; the commands for features not ported yet are absent | served + ui |
| Documentation [文档] | Opens the docs | Done: from the home page, Settings and the command palette, in the system's browser | ui |
| Settings entry | Yes | Done | ui |

## 2. Workspace sidebar

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Back to home, project name, collapse sidebar | Yes | Yes: the sidebar hides and comes back with a button by the conversation's title or ⌘B, and stays as it was left | ui |
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
| Paper evidence [论文证据] | Section 7 | Partial: see section 7 | store |
| Favorites [收藏库] | Section 1 | Done | store |
| Capability summary [能力]: skills, MCP, memory counts, local tools found | Dialog | Done: counts of skills, connectors, MCP servers, SSH hosts and memory, and the tools found on the search path | shared |
| Feedback [反馈] | Yes | Yes: Report a problem (Settings, palette) starts a conversation with the app's version, system and model in the composer, for the model to draft the issue and a prefilled GitHub link; Known problems opens the issue tracker. The collected details are visible in the composer rather than hidden, and startup timings are not included | ui |

## 3. Composer

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Multiline text, IME, drafts kept per conversation | Yes | Done | ui |
| Send and stop | Yes | Done | served |
| Send key choice (Enter or modifier+Enter) | Setting | Yes: Return or Cmd/Ctrl+Return in Settings, General, Return by default; the other key starts a new line | ui |
| Queue a message while a turn runs | Yes *(source)* | Partial: up to ten per conversation, sent in order as turns end, with take back, drop and a cut-in that gives one to the running turn now; kept in the window only | tauri |
| Attach files, images, PDF [附加文件] | Yes | Partial: any file through the system dialog, drop or paste, copied into `uploads/` when it is outside the project; a model that can see gets the attached pictures themselves, others get the paths; no describing pictures with a separate model for one that cannot see, and no question before a large picture is sent smaller | served |
| Browse project files to attach [你的文件] | Yes | Done: found by name in the add-context box | served |
| Paste or drop files | Yes | Done: files dropped onto the conversation are attached, and a picture pasted into the composer (a screenshot, say) is kept in `uploads/` and attached; text pastes as text | tauri |
| `@` artifact, `#` conversation, `/` skill mentions | Yes | Partial: conversations and skills, picked from the add-context box rather than typed inline; a conversation is passed as a recent excerpt, not through the reader agent; no artifacts, environments, runtimes, workflows or memos | served |
| Model picker per conversation | Yes | Done: saved API models, or follow the default | served |
| Reasoning effort / fast mode | Yes | Yes: effort, and fast mode (priority processing) where the model is OpenAI's own, per conversation and kept across restarts; fast mode has no per-model default in Settings | store |
| Permission mode [默认 / 完全权限] | Yes | Done: Ask first or Full access per conversation, with a question before it turns on; dangerous shell commands still ask | served |
| Plan first [先计划] | Yes | Done: the model investigates and submits a plan; tools that change things refuse until the plan is approved or plan mode is left | served |
| Expert (persona) for the conversation [专家] | Yes | Partial: chosen in the composer before the first message; its instructions join the system prompt and it runs on its own model when it has one; an expert's skill or connector lists are not applied | tauri |
| Automatic review toggle and review model [自动审查] | Yes | Yes: per conversation, with a default and a reviewer model in Settings; what a review finds is corrected once by resuming the conversation with the findings, and the correction is reviewed again; the result shows above the composer and stays in the transcript | tauri |
| Sub-agent model [子智能体模型] | Yes | Yes: a choice in Settings for every conversation, and each conversation's own in the composer's model menu (as in Settings, its own model, or a saved model), in the WebView's order | store |
| Compute environment for the conversation [计算环境] | Yes | Partial: the default, this computer or a saved SSH host, for where runs start; Python, R and the shell stay on this computer | store |
| Context usage meter and manual compaction | Yes | Yes: how full the context is, as a percentage that opens the list of what the model is sent, message by message; /compact shortens the history at once | served |
| Request a review of the conversation [请求审查] | Yes | Yes: a Review dialog lists what the reviewer model finds and puts the corrections in the composer; the report is kept with the conversation and shown in the transcript after what it read | tauri |
| Share as long image or HTML [分享为长图] | Yes | Partial: Share as HTML (and `/share`) saves the whole conversation's messages and answers as one page with the WebView's share stylesheet, and Export saves a Markdown file; no picking messages and no long image | ui |
| Save the conversation as a skill [保存为技能] | Yes | Done: puts the request for the `skill-creator` skill in the composer, after anything already typed | shared |
| Interactive runtime status (Python / R) [Local · Python · R] | Yes | Yes: the conversation's Python and R interpreters above the composer with their state and version, and in the tooltip their path, PID, generation, memory in use, when they started and last ran code, and why they stopped; Stop, and a console with their variables, code of your own and Restart | shared |

## 4. Transcript

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Markdown, code, tables, selectable text | Yes | Done | ui |
| Paginated history | Yes | Done | served |
| Tool calls: input, output, status, time | Collapsible cards | Done: one folded line each, click to unfold | served |
| Approvals: allow once / deny | Plus allow for session / always | Yes: once, for the rest of the conversation (forgotten on restart), or always in this project (listed in Settings, Permissions, where it can be revoked) | served |
| Agent question card with free-text answer [Agent 需要你的决定] | Yes | Done: options answer with one click, or type the answer | served |
| Follow-up suggestions | Yes | Done: up to three questions above the composer after an answer; picking one puts it in the composer | served |
| Thinking blocks | Yes | Yes: shown above the composer while the model thinks, and kept in the transcript as a folded Thought ahead of its answer | served |
| Images and figures inline | Yes | Partial: project pictures an answer shows with Markdown image syntax, and those its turn wrote with `write` or `edit`, drawn under the answer and opened in the Files tab with a click; the WebView's cards for the turn's other files, and pictures written by code the model ran, are not shown | served |
| Token usage per turn | Yes | Yes: the running turn's usage and context above the composer, and every saved turn's input, output, thinking and cached tokens under its last answer, kept across restarts | served |
| Skill and attachment chips on the user message | Yes | Yes: file chips, and the conversations and skills a message was sent with from this window while it is open; as in the WebView, references are context for that turn only and are not kept with the message | served |
| Copy message [复制] | Yes | Done: copies the message as Markdown | ui |
| Rewind to a message [回溯 / 撤销] | Yes | Yes: goes back and returns the text to the composer, and the latest turn can be undone with the files it wrote | store |
| Branch from a message [分支] | Yes | Partial: a copy through that turn, nested under its source, that can be brought back: Bring back has the model summarise what the branch did, you read and change the summary, and it is added to the source conversation; no branch card | store |
| Save to memory [记忆] | Yes | Done: the message is opened for trimming and saved for the project or as a habit, and Suggest has the reviewer model draft what is worth keeping from its turn and say where | store |
| Review a turn [审查] | Yes | Yes: a review button under each answer reviews the conversation as it stood when that turn ended, and the report is kept after that turn in the transcript | tauri |
| Select text: add to chat, ask aside, highlight | Yes | Partial: per message rather than per selection — Quote puts a message in the composer, Ask aside opens the aside with it quoted, the star saves it; a selected passage can only be copied (the text view's selection is private to gpui-component) | store |
| Full record / model view toggles [完整记录 / 模型视角] | Yes | Partial: the transcript is the full record; What the model is sent lists the model's working set in a dialog, with each message's share and the set by kind (tool definitions, made when a turn starts, not counted); no undo of a compaction | served |
| Conversation outline [展开对话目录] | Yes | Yes: your messages, to jump to; while the start is not loaded, Load earlier in the outline loads the whole conversation so all of it is listed | store |
| Find in conversation | Yes *(source)* | Done: Cmd/Ctrl+F over the loaded transcript, with a count and next and previous; marks rows, not words | ui |
| Sub-agent and delegation cards | Yes *(source)* | Partial: an explore call is an ordinary tool row in the transcript and a card in the Agents tab; no delegation cards | store |

## 5. Top bar

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Conversation tabs | Yes | Done: the conversations opened in a window show as tabs above the conversation, across projects; not kept across restarts | ui |
| Share conversation [分享对话] | Yes | Partial: as an HTML page or a Markdown file saved where you choose; no share link | tauri |
| Trajectory [轨迹] | Yes | Done: a dialog with the totals and every turn's steps; a step opens to its whole text | store |
| Research archive [研究归档] | Yes | Partial: the model drafts a report and scripts, you edit title and report, and freezing writes them into the project and makes the conversation read-only; no choice of files to snapshot or delete, no script editing, no continuation link | store |
| Pending decisions [N 个待处理] | Yes | Done (sidebar inbox) | served |
| Terminal [打开终端] | Yes | Partial: terminals drawn in the app, in a dock under the conversation, in the project folder or the folder being browsed, on a saved SSH host or a WSL distribution, several at once as tabs, hidden and shown again, closed one by one; the system's terminal is one button away; no input through the system's input method, and no selecting part of the screen | tauri |
| Toggle right panel [切换面板] | Yes | Partial: the panel opens and closes from the header and keeps Files, Runs, Artifacts, Notebook, Memos, Memory, Highlights, Aside and Agents tabs, one showing at a time; no Provenance tab (a file's preview shows which tool call wrote it) and no adding or closing tabs | ui |

## 6. Right panel

The WebView panel is tabbed; a launcher [开始] opens each tool as a tab.

| Tool | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Workspace files [工作区文件]: browse, preview, before any conversation exists | Yes | Done | served |
| Files: name search | In the panel | Yes: a field in the Files tab lists the project's files named like what is typed, with their folders, in place of the folder until it is emptied; the command palette also finds files across projects | served |
| Files: upload, new file, new folder, rename, delete, sort, grid view | Yes | Partial: new folder, new file, rename, delete as a move to the project's own trash, and sorting by name, time or size; no upload into a folder or grid | tauri |
| Files: other locations (SSH / WSL contexts) | Yes | Partial: the Files tab lists the folders of a checked SSH host, to look at; no preview, download, upload or change there, and no WSL | shared |
| Files: image, PDF, Office, notebook, table previews | Yes | Partial: pictures (PNG, JPEG, GIF, WebP, SVG, BMP) up to 32 MB, CSV and TSV as a table, and Jupyter notebooks as their cells and outputs (figures named, not drawn), PDFs as their first twelve pages drawn (by `hayro`, in pure Rust) with a switch to their text, and Word, Excel, PowerPoint, OpenDocument, RTF and EPUB documents as their text and tables (the converter the model's read tool uses); alignments (Clustal, Stockholm, aligned FASTA) and FASTA files as rows of residues coloured by kind, a hundred columns at a time; Office documents' layout and pictures are not shown, molecules (MOL, SDF, MOL2, crystals' CIF files, and SMILES laid out flat) as their atoms and bonds with their bond orders and structures (PDB, mmCIF) as the trace of their chains, drawn flat and turned by dragging rather than in 3D; HTML as a page of text, headings, lists, tables and links, without its scripts, styles or pictures | ui |
| Agents [Agents]: sub-agents, parallel exploration, workflows | Yes | Partial: an Agents tab lists the explore helpers of the loaded conversation with their question, status and findings; subagent conversations (`dispatch_subagent`, `subagent_status`, `stop_subagent`) run, nest under their parent and report back; delegated task batches (`delegate_tasks`, `get_delegated_result`) run in the turn, each task in a watch-only conversation under its parent, after a plan card when a task can change something, and the Agents tab lists each batch with its tasks and stops a running one; background batches deliver their result later and resume the conversation, and a failed or stopped batch runs again keeping finished tasks; tasks can be done as one of the researcher's experts, a task that did not finish can be given another token budget before the rerun, a task granted delegation runs one nested batch of its own within the root's limits, and an isolated task works in a Git worktree of its own whose changes are merged or kept as a patch | store |
| Artifacts [产物]: figures, tables, outputs, star, open | Yes | Partial: files a turn wrote are recorded with a snapshot, listed per conversation or project, and open in Files; no grouping, versions, download, star or provenance | served |
| Environments [环境]: conversation environment, attach servers, probe, terminal | Yes | Partial: the conversation's environment is chosen in the composer and hosts are checked in Settings; no panel tab, attaching or terminal | store + shared |
| Runs [运行]: list, output | Yes | Done: a Runs tab with status and output tails | served |
| Runs: cancel | Yes | Done | served |
| Runs: harvest, transfer, full log, workspace review | Yes | Partial: the whole log of a run on this computer; no harvest, transfer or workspace review, and no whole log for runs on SSH hosts | shared |
| Notebook [Notebook]: code cells and outputs | Yes | Done: a panel tab with the loaded conversation's cells and outputs, copy, save to favorites and show in the conversation | store |
| Highlights [划线] | Yes | Done: lists what the conversation saved, with use, copy and remove | store |
| Provenance [溯源]: tool evidence behind a claim | Yes | Partial: a file's preview shows which tool call wrote it, with the code, what it read and the conversation; no panel tab and no tracing of a claim in an answer | store |
| Side question [侧边问答]: ask without changing the main thread | Yes | Partial: an Aside tab where the conversation's model answers from its latest turns; no evidence list, no question scoped to a run or to selected text | tauri |
| Memos [备忘录]: private notes the model cannot read | Yes | Done: no search, no `/memo` command | store |

## 7. Project pages

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| Research journey [研究历程]: by day, relation graph, search, add record, recap, artifact sources | Yes | Partial: a month at a time by day, a filter, adding a record of your own, and a day's recap drafted by the model, confirmed or dismissed; no relation graph, recap edits or artifact sources | store |
| Paper evidence [论文证据]: papers, versions, figures and tables with their sources | Yes | Partial: a page of papers with their versions, and figures, tables and claims bound to project results and runs with a reason; readiness check, waivers, finalizing and new versions are there; no capsules or reproduction, and no conversation passages as evidence | store |

## 8. Settings

| Section | WebView | GPUI | Host |
| --- | --- | --- | --- |
| General [常规]: language, workspace directory, per-project storage, resume last conversation, send key, selection quick actions, notifications, sound, update check, local tool paths | Yes | Partial: language, appearance, send key, notifications, sound, update check and resuming the last conversation; no workspace directory, per-project storage, selection quick actions or tool paths | tauri |
| Network [网络]: proxy per channel (model API, account login, MCP, code requests), package mirrors | Yes | Partial: routes for model requests, connectors and MCP, and code; conda mirror, Python index and CA bundle; no sign-in route, and no route of its own for one model or one MCP server | tauri |
| Conversation [对话]: iteration limit, auto-continue, auto-compaction and threshold, follow-up questions | Yes | Done: steps per turn, continuing cut-off answers and how many times, automatic compaction and its threshold, follow-up questions, review of turns and the reviewer model | tauri |
| Appearance [外观]: system/light/dark, theme, accent, fonts and sizes, live preview | Yes | Partial: system, light or dark with the Wisp palettes, applied at once; no other themes, accent, fonts or sizes | ui |
| Pet [宠物] | Yes | Partial: a Pet page turns it on and chooses its folder, and it shows the conversation's state in the transcript's corner; no desktop pet window, no walking or looking at the pointer | ui |
| Models [模型]: API models with provider, URL, key, limits, effort | Several profiles, presets, default | Done: any number of models, the six presets, a default, a connection test | served |
| Models: ACP agents, subscription accounts, decision model | Yes | Partial: a Decision model page with the four services, key, thinking mode, timeout and Test connection, used by `map_items`; an ACP agents page (add, change, remove, test, shared list) and conversations run by an ACP agent; subscription sign-in for ChatGPT, ChatGPT (Codex) and SuperGrok, shared through the keychain, with token refresh before turns; the Codex account pool (import from Codex CLI and CLIProxyAPI, switch, remove, usage meter); no `desktop_autopilot` | tauri |
| Quick actions [快捷动作] | Yes | Partial: named requests kept in Settings and put into the composer from its quick actions menu; they start no workflow, and the WebView's own list is not read | store |
| Workflows [工作流]: library, graph editor, approval policy | Yes | Partial: a library of templates (goal, tasks, which earlier tasks each needs, what each may do as an agent, approval policy) run as one conversation, or as agents: drafted as a batch of one agent per task in a new conversation, approved or discarded in its Agents tab (auto-safe starts a read-only one at once), each task with its expert, model tier, own worktree, token budget, time limit and result shape; a map of the workflow by stage on which two clicks join or part two tasks; a context told to every task; From skills writes one from skills' method documents to review and save; no ACP executors or named model per task, no dragging on the map, skills with scripts of their own are not converted, and the WebView's own templates are not read | store |
| Experts [专家]: built-in and custom, default sub-agent model | Yes | Partial: an Experts page lists the built-in illustrator and your own, with add, change and remove, and the model each runs on (the conversation's or one of its own); no skill or connector lists; the sub-agent model is chosen on the Conversation page | tauri |
| Memory [记忆]: project notes, global habits, enable switch | Yes | Done, as a side-panel tab rather than a Settings page, with drafts proposed after a turn that asks to remember or whose tools failed often, and the weekly tidy; no "clear all" | store |
| Hooks [钩子]: automatic review, tool-failure analysis, command hooks | Yes | Partial: automatic review (with one correction), tool-failure analysis and follow-up questions, as settings on the Conversation page; no Hooks page and no command hooks | tauri |
| Skills [技能]: list, tags, enable, community, GitHub, reload | Yes | Partial: list with source and tags, search, on or off, reload, and the researcher's own tags; no community catalog or GitHub install | shared |
| Plugins [插件] | Yes | Partial: a Plugins page installs from a folder, lists skills and MCP servers, switches a plugin on per project (its skills join that project's conversations) and removes it; no install from a zip or an address, no checksum, and a plugin's MCP servers are not started | store |
| Browser [浏览器]: auto-open, auto-close tabs, block and prefer lists | Yes | Partial: a Browser page switches on the bridge to the extension, shows whether it is connected and keeps the block and prefer lists; conversations get four tools limited to tabs they opened; no auto-open, no auto-close, no human-verification prompt, screenshots or side panel | tauri |
| Connections [连接]: built-in bio tools and user MCP servers, enable, add | Yes | Partial: the bundled connectors, and your own MCP servers over HTTP (with a token) or as a local program, with test and on/off; no OAuth, extra headers, environment variables or MCP Apps | shared |
| Remote access [远程接入]: project sync relay, Feishu, WeChat, StickS3, web access, Wisp-Lab | Yes | Partial: remote web access through your own relay (on or off, address, token, state, link and code, reset) with the headless server's command list; WeChat and Feishu on a Chat apps page (WeChat QR binding; Feishu app made by scanning or entered, owner confirmed in Settings, answers on a progress card; on or off, unbind; the owner's messages run as turns with `/project`, `/session`, `/new`, `/status`, `/stop`, and WeChat approvals by code); Wisp Lab (found or join by invite, members approved and removed by the leader, invite codes, the project that answers lab mail, token, leave; agents mail members with `lab_members`/`lab_send` and answer mail in a conversation per sender); the StickS3 desk display (LAN listener with its token, agent state, pet frames, focus and acknowledge); project sync through a relay or a shared folder, and the lab's knowledge base; the research assistant's own WeChat bot (its commands, full access, model and mandate notices); the web access link as a QR code for a phone, shown when asked for | shared |
| Credentials [凭据]: service keys in the keyring, custom env keys | Yes | Partial: the NCBI and OpenAlex keys the bundled connectors use, and credentials of your own (a name, an environment variable and a value in the keychain, in the WebView's list), given to the interpreters; the WebView's InfiniSynapse and SCIMaster keys are not listed | store |
| Permissions [权限]: approval mode, remembered approvals | Yes | Partial: how new conversations start, and tools always allowed per project with revoke; no per-tool allow/ask/deny rules or global grants | tauri |
| Environments [环境]: default, add SSH host, import `~/.ssh/config`, paths, probe | Yes | Partial: SSH hosts with key, agent or config sign-in (add, change, remove, add from `~/.ssh/config`), a connection check that records what the host is, and where new work runs; password sign-in with the password in the keyring; no WSL, interpreter paths or storage locations | store + shared |
| Storage [存储]: data directory, per-project usage breakdown | Yes | Partial: the data folder with a button to show it, database and favorites sizes, and what Wisp keeps in each project; nothing can be moved or cleaned | store |
| Usage [用量]: tokens by day, model, workspace, skill/MCP ranking | Yes | Done: totals, thirty days, by model and by project (tokens counted from now on), and the SKILL / MCP calls ranking, counted from the model's messages | store |
| Settings search | Yes | Partial: finds pages by their name and by what they are about; it does not find or jump to a single setting inside a page | ui |

## 9. Across the app

| Feature | WebView | GPUI | Host |
| --- | --- | --- | --- |
| One Escape stack, topmost layer first | Yes | Done | ui |
| Read state: answer marked read when on screen | Yes | Done | served |
| Background status for conversations not on screen | Pushed | Done (2 s poll while a turn runs) | served |
| Shared icon set | Lucide-style | Done (same drawings, `apps/gpui/assets/icons`) | ui |
| Chinese and English | Yes | Partial: a Language choice in Settings (System, English, 中文); the fixed text of every page, panel and dialog, the names of tabs, commands and settings sections the placeholders of text fields, statuses, counts, ages, dates, token figures, run statuses and sentences with a value in them are in Chinese; host messages that carry a value (paths, addresses) and tool questions are still as the host wrote them | ui |
| Desktop notifications and sound | Yes | Partial: for the open conversation when a turn ends or a tool waits, and for a recent conversation that is not open when it stops working, while the window is in the background; one fixed sound; no notification on Windows | ui |
| Update check and install | Yes | Partial: a check when the app starts (the WebView's switch, with Don't check at start on its note) and Check for updates in Settings say whether a newer release exists and open the release page; no in-app install | tauri |
| Several windows | Yes | Yes: any number on the same host, each with its own page and conversation; a window opened for a project opens again in it at the next start until it is closed, as the WebView's project windows do | ui |
| Keyboard shortcuts (⌘N, ⌘K, ⌘B, ⌘F, ⌘⇧F, ⌘P, ⌘,) | Yes | Done: ⌘N, ⌘K, ⌘B, ⌘F, ⌘⇧F, ⌘, and ⌘P, the palette with only its commands (Ctrl off macOS) | ui |
| Import existing WebView data | n/a | Partial: project by project, by exporting an archive there and importing it here; settings, keys and favorites are not carried over | store |

## 10. Also in the WebView

Found in the sources rather than on screen. `Missing` in GPUI unless the
row says what is there.

| Area | Feature | Host |
| --- | --- | --- |
| Home | Search across projects, conversations and recent artifacts (projects, conversations and files are in the palette; artifacts are not) | store |
| Home | Import a folder in place, a ZIP, or recover conversations from a workspace; transfer progress | tauri |
| Home | Sync now, device code, conflict resolution (use remote / use local) — **Done** in project settings, with Join a project from another device on the home page; cloud-folder badges on the project cards — **Done** | shared |
| Sidebar | Conversation menu: copy title, export conversation, export debug request, reload project rules — exporting a conversation as Markdown, copying its title and exporting its debug request (the saved messages form, without tool definitions) are **Done**, and project rules are read again every turn | tauri |
| Sidebar | Nested children: branches are nested — **Done**; explorations are opened from the bar over their source conversation, not nested in the sidebar. Subagent conversations are nested under their parent — **Done**. "Load earlier" paging — Missing | store |
| Sidebar | Update-available card; "done / failed in another project" notices (an update check is in Settings, General, and a turn that ends raises a system notification; there is no card in the sidebar) | tauri |
| Composer | Slash commands: `/timer` `/archive` `/compact` `/fork` `/btw` `/rewind` `/review` `/remember` `/context` `/plan` `/permission` `/save-as-skill` `/skills` `/files` `/upload` `/share` `/trajectory` `/memo` — all eighteen run here and are listed under the composer's "/" button — **Done**; `/share` exports Markdown, and there is no list that narrows as you type | tauri |
| Composer | Conversation timer: repeat a prompt on an interval, pause, resume, cancel — `/timer 30m <prompt>` and `/timer off` are there, each firing a new turn; no pause or resume, and the turn before is not replaced | store |
| Composer | Queue panel: edit, remove, reorder, "guide now" — a waiting message can be taken back to change, removed, moved earlier, and given to the running turn now | tauri |
| Composer | Plan progress strip; read-only states (sample, archived notebook, sub-agent, frozen branch) | served |
| Composer | ACP agent conversations and their mode / config options — conversations run by an ACP agent are **Done** (text, thoughts, tool calls, plan, permission questions, stop); signing in is **Done** from Settings, ACP agents, and modes, config options and the agent's slash commands are **Done** in the composer once the agent runs, and the MCP bridge is **Done** for skills, memory, runs, connectors and MCP servers (over loopback HTTP or a stdio relay); the bridge's tool search is **Done**; its artifacts, research graph and delegation tools are Missing | tauri |
| Transcript | Equations, table copy, internal file links, confirm before opening an external link | ui |
| Transcript | Generated-artifact strip (the files are in the Artifacts tab, not yet under the message); image and video generation cards | store |
| Transcript | Run cards: send to background, cancel, harvest outputs, details — a call that started or watched a run shows the run's state under it and opens its output in the Runs tab, where it can be cancelled; no send to background or harvest | shared |
| Transcript | Compaction markers, undo compaction, stream-recovery and reviewer banners | tauri |
| Transcript | Undo a turn (restore or remove the files it wrote) — **Done** for the latest turn's text files from the go-back question; no preview beforehand | store |
| Transcript | Quote a range of turns into another conversation; distill an experience | tauri |
| Transcript | Explorations: start, diff, promote to mainline, discard, abandon | store |
| Transcript | Branch cards and merge a branch back with a summary | store |
| Transcript | Plan card: approve and run, or leave plan mode — **Done**. The execution plan — **Done**: each update the model makes is a checklist in the transcript, and a strip above the composer says how many steps are done and which is next. Plan review with feedback — **Done**: a plan the model asks to start on shows its steps with Approve, Turn down, and a box for what should change, which goes back to the model with the refusal | served |
| Transcript | Reviewer findings panel and the automatic correction pass — review on request, of one turn and after every turn are **Done**; the correction is put into the composer to send, not run by itself | tauri |
| Transcript | MCP App previews; Motif DNA selection | tauri |
| Context | Usage breakdown panel, compact dialog (regular / semantic), handoff note to a new conversation, too-long recovery — what the model is sent, message by message with token estimates, is **Done**, and `/compact` compacts on request without a choice of kind; the rest is Missing | tauri |
| Top bar | Find in conversation (⌘F) — **Done**; search the project's chat history (⌘⇧F) — **Done**: it opens the sidebar's search, which reads titles and messages | store |
| Top bar | Import Codex and Claude Code conversations — **Done** for this computer: the conversation list's import button lists what `~/.codex/sessions` and `~/.claude/projects` hold and brings one in, in a group named after the tool; importing again brings in only what was added. From SSH or WSL hosts, and importing a conversation archive — Missing | tauri |
| Centre | Open a file as a centre tab, split chat beside a document, snapshot, open in an external editor | ui |
| Files | Copy path, attach to chat, register as an artifact, reveal in the file manager, open with the default app — all **Done** from a file's preview, registering as an artifact of the open conversation included | tauri |
| Previews | PDF, images with zoom and region pins, CSV/TSV, DOCX/XLSX/PPTX, LaTeX, HTML, notebooks, structures, molecules, alignments, FASTA, trees — Partial: each has a preview (see the Files row in section 6 for how each is drawn), and LaTeX sources and trees (Newick) open as text, as they do in the WebView; images have no zoom or region pins | ui |
| Previews | Code editor: save, run the selection or the script in a runtime | shared |
| Previews | DOCX review: accept or reject tracked changes | tauri |
| Runs | Review a remote run's workspace: download, delete, clean up; remote files pane — an SSH host's folders can be looked at from Files; the rest is Missing | shared |
| Environments | Add or edit an SSH host (key or password, ProxyJump, test), import WSL, interpreter paths, storage locations | store + shared |
| Runtimes | Python / R runtime cards: start, stop, restart, object inspector, console, plots — **Done** for the interpreters on this computer a conversation keeps: start from the palette, stop, restart, variables, a console with plots; interpreters on SSH and WSL contexts are not started from the window | shared |
| Terminal | Several terminals, choose the environment, collapse, close — Partial: the host keeps PTY terminals (this computer, SSH hosts, WSL) that the window draws with `vt100` in a dock, as tabs, collapsed and shown again with their output, closed one by one; no input method and no partial selection | tauri |
| Agents | Swarm progress, agent workflow runs (draft, approve, run, cancel, retry), method-search runs — swarm progress of `map_items` is shown in the Agents tab while the app runs; an Agents tab lists a conversation's helpers, workflows run as one conversation (section 8), and the agent's delegated batches run with a plan card for approval and are listed there with Stop and Run again, in the turn or in the background; a failed task's token budget can be changed before Run again, and tasks can delegate one level down or work in a worktree of their own; a method search's contract is prepared (`prepare_method_search`), a batch's run-activity step drafts its Run, and its Search card in the Runs tab shows the contract, progress, outputs, candidates and strategies with Start, Pause, Resume and Cancel; a workflow run as agents is drafted, approved or discarded, run, stopped and run again there too | store |
| Library | Versions of saved code, edit code, insert into chat | store |
| Publication | Revisions, manuscript items, evidence binding, readiness waivers, reproduction check, capsule — papers, items, evidence from results and runs, new revisions, the readiness check with waivers and finalizing are there (section 7); reproduction and capsules are Missing | store |
| Assistant | Assistant chat, saved plans, scheduled tasks, daily recap, weekly memory tidy, research mandates, WeChat binding — the assistant's conversation with its tools and plan, scheduled tasks, the daily recap with the progress memory, the weekly memory tidy, research mandates with their reports, the WeChat binding (Chat apps), and the assistant's own WeChat bot are there | store |
| App | Onboarding wizard, toolchain installer (uv, Python, R, Node, pixi), local tool detection — the toolchain installer is **Done** on Settings → Environments (Tools on this computer), and the onboarding wizard is **Done** as a first-run guide (welcome, features, DeepSeek key, install Python); detection of tools installed elsewhere is Missing | tauri |
| App | Action palette (⌘P), privacy mode (⌘⇧H), application and native menus | ui |
| App | Browser automation prompts (tab cleanup, needs a human), browser-extension update banner — Missing; the Browser page says when the extension has to be reloaded | tauri |
| App | SSH connectivity check — **Done** in Settings, Environments; generic confirmation dialogs | shared |

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
    research calendar; 10c done: the trajectory of a conversation; 10d done:
    archiving a conversation's research; 10e done: paper evidence, first
    phase; 10f done: scheduled tasks)*
11. **Project transfer and sync** — import, export, relay sync, remote access. *(11a done:
    export and import as an archive; 11b done: remote web access through a
    relay; relay sync and the messaging channels remain)*
12. **Experts, workflows, quick actions, ACP, accounts, plugins, browser.**
    *(done in a first phase: experts, quick actions, the decision model,
    plugins, workflows and the browser bridge; ACP agents and conversations
    run by them in a second; subscription accounts remain)*
13. **Finish** — localization, notifications, several windows, updates,
    data import from the WebView desktop, installers. *(done: the Chinese
    interface, notifications, several windows, the update check and the pet;
    installers remain)*
