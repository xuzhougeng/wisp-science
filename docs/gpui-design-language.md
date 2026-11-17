# GPUI design language

The GPUI desktop reads like a lab notebook on warm paper: ivory surfaces, ink
text, one teal accent and serif titles. It continues the look the WebView
desktop already has, so a researcher who moves over recognises the product.
The direction was chosen on 2026-11-15 from four candidates and is called
*paper notebook* (纸本笔记).

A proposed successor, *lab record* (实验记录本), with an icon language shared
with the WebView, is written up in
[the design draft](superpowers/specs/2026-11-16-gpui-lab-record-design-language.md).
It lands in steps. The first is in: the bundled faces and Medium titles,
the 4 / 6 radius, the window's own title bar on macOS, the icon helpers,
the record row and the stamp, and the home page drawn from them. The rules
below are the ones in force, these included.

This page is the rule book. [GPUI desktop](gpui-desktop.md) describes how the
build works and [GPUI parity ledger](gpui-parity.md) what is ported.

## Principles

1. **Reading first.** A conversation is a record to read, so its text keeps a
   reading width and its titles are set in a serif face.
2. **Paper, ink and one colour.** Surfaces are paper and white, text is ink,
   and teal marks only the main action, links and focus.
3. **Lines, not shadows.** A hairline separates things that sit on the page.
   Only layers that float above it (dialogs, menus) cast a shadow.
4. **Tools at the margin.** Files, runs and evidence sit beside the text like
   notes in a margin; they never push the conversation out of its column.
   A document opened in the centre is not a margin note: it is read on the
   page, in the conversation's place or beside it, and the conversation
   beside it keeps a column of its own.

## Text

Four roles. The rem is set so that the component library's helpers come out
at these sizes when the body is 14 px (`theme::rem_for`); the General setting
for text size moves all four together.

| Role | Code | Size | Face | Use for |
| --- | --- | --- | --- | --- |
| Caption | `text_xs()` | 12 | UI | Hints and notes not yet moved to Fact |
| Fact | `surfaces::fact` | 12 | Mono | What a machine measured: times, durations, sizes, counts, paths, ids, model names, shortcuts, the stamp's word |
| Body | none, or `text_sm()` | 14 | UI | Everything else; the window's default |
| Title | `surfaces::title` | 18 | Serif | A page, a section, a dialog, a conversation, a project |
| Display | `surfaces::display` | 30 | Serif | At most one per page: the home greeting, the empty conversation |

- Said is serif, measured is mono: a title, a question or a project's name
  is a Title; a time, a size or a path is a Fact.
- The UI face is the system's (`.SystemUIFont`). Two faces are bundled in
  `apps/gpui/assets/fonts` with their SIL Open Font Licences and registered
  at start (`assets::register_fonts`), so the Latin is the same on every
  system: Source Serif 4 (Regular and Medium, static instances at optical
  size 20 of the google/fonts variable font) for titles, and IBM Plex Mono
  (Regular and Medium) for facts, with tabular figures (`tnum`).
- A title's Chinese falls back to the system's Chinese serif (Songti SC,
  Source Han Serif SC, Noto Serif CJK SC); where none is installed, as on
  most Windows systems, to the UI's Chinese face (PingFang SC, Microsoft
  YaHei UI, Noto Sans CJK SC), never SimSun. macOS and Windows honour that
  order; on Linux the text system chooses the fallback itself.
- Code and the terminal keep the component library's monospace face and
  the Code size setting; the fact face is not that setting.
- **Weight** (spec §4.1): Regular is the weight of all text in the
  interface: navigation, tabs (the open one too), list and record-row
  content, setting names, button labels, condition values, stamps,
  section, table and panel heads, card titles, names in lists, and serif
  titles. Medium is only for the one Display line of a page and the
  wordmark; for the label of a page's or dialog's one filled button; and
  for at most one line per region that must be seen first, used sparingly.
  What is chosen, open or current is said with the ground, ink against
  faded ink, or teal, never with weight. No Semibold or Bold anywhere; a
  terminal's bold is drawn in Medium. In Markdown (answers, documents,
  reports) strong text and every heading are Medium, and headings rank by
  size only: an h1 1.3× body text (no larger than a title), an h2 1.15×,
  the rest body size (`surfaces::heading_scale`); a table's header row is
  Regular like its cells. A name with a note: the name in
  ink, the note faded, the same weight. A test in `surfaces.rs` rejects a
  heavier weight anywhere else.
- Rank is said with colour (`muted_foreground`), not with a smaller size.
  Caption is for metadata, not for making a secondary sentence quieter.
- Views do not use `text_base()`, `text_lg()`, `text_xl()`, `text_2xl()`,
  `text_3xl()` or a size in pixels; a test in `surfaces.rs` rejects them. The
  terminal's grid is the one exception. A heading is `surfaces::title`; a
  figure or a code that should stand out is body text in ink.

## Colour

The values live in `apps/gpui/assets/wisp-theme.json`; views use the theme's
names and never a colour of their own.

| Name | Light | Dark | Use for |
| --- | --- | --- | --- |
| Paper (`background`) | `#faf9f6` | `#171614` | The page |
| Margin (`sidebar`) | `#f0eee6` | `#1c1b18` | The sidebar and the side panel |
| Card (`popover`) | `#ffffff` | `#1f1e1b` | Cards, the composer, dialogs, menus |
| Ink (`foreground`) | `#141413` | `#ebe8e2` | Text |
| Faded ink (`muted_foreground`, and `tab.foreground`) | `#6b675d` | `#958f86` | Secondary text, times, icons at rest |
| Line (`border`, `sidebar.border`, `title_bar.border`) | ink at 9% | paper-white at 8% | Hairlines |
| Teal (`primary`) | `#0f766e` | `#2da898` | The main action, links, focus |

- Faded ink must reach 4.5:1 on both the page and the margin; a test
  checks both themes. The fields' line (`input.border`) stays stronger
  than the hairline.
- The main action's label on teal must reach 4.5:1; a test checks both
  themes. That is why the light teal is `#0f766e` and not the lighter
  `#0d9488`, which stays for focus rings and progress bars.
- One filled teal button per page or dialog. Other actions are outlined or
  plain text.
- Status is green, amber, red or blue from the theme, always with a word or
  an icon, never colour alone.
- The palettes in Settings › General lay their own accent, page and text
  colours over these names; the rules above still hold for them.

## Shape and space

- **Radius**: 4 px for controls and rows (`radius`), 6 px for cards, fields,
  the composer, dialogs and menus (`radius_lg`), 2 px for a stamp. Pills
  only for counts, tags and switches.
- **Cards**: `surfaces::card`: a white surface with a hairline and no
  shadow. Hover darkens the line; it does not lift the card.
- **Shadows**: dialogs and menus only (with the palette and the cards
  for other projects' conversations, which also float over the page). A test in `surfaces.rs` names
  every function that may draw one and rejects a shadow anywhere else.
- **Spacing**: the rem steps (`gap_1` … `gap_8`, `p_*`), not pixels. Pixels
  are for layout widths.
- **Rows**: a row in a list or the sidebar is 32 px high or more, with a
  hairline between rows of a list on the page.
- **Record rows** (`record_row`, `record_group`): a record is one row of
  its time (a Fact), its object's icon (inline size, faded), what it says
  (body, cut short with the whole under the pointer, what it belongs to in
  faded ink before it), at most one stamp, and a value (a Fact) at the
  right end. 32 px or more, a hairline under each row and one above the
  first of a group. The time and the icon keep their columns when empty,
  and so does the value in a group that has values, so rows line up. The
  time column is 52 px, room for `17:04`, `昨天` or `11-15`; a group with a
  longer time (a full date, `Yesterday`) widens it to its widest time for
  all its rows (`record_time_width`), and a group with no times has no time
  column at all: its rows start with the icon. An identifier (a tool's
  name, a command, a path) is set in the fact face in ink (`mono`); a
  faded suffix may follow the text (`· local`), and icon buttons may end
  the row after its value. 10 px from the time to the icon and
  from the icon to the text.
- **Stamps** (`stamp`, `stamp_with`): a status is a stamp, not a coloured
  word or a pill: square corners (2 px), a hairline in the status colour,
  no fill, and a word as a Fact at the regular weight in that colour; no
  icon. Success,
  waiting, failed and running are the theme's `success`, `warning`,
  `danger` and `info`; on the light page the word wears the colour's
  `active` shade so it reaches 4.5:1, on the dark page the colour itself
  (a test checks both). Only the home page's bell over a count of what
  waits for the user carries an icon (`stamp_with`). Where there is no room
  for a stamp, the status icon alone in the status colour, its word as the
  tooltip, stands in its place.

## Layout

### Window

- On macOS the window draws its own title bar (`surfaces::window_bar`): 40 px
  in the margin colour with the margin's hairline under it, in either theme
  and any palette, the traffic lights inset inside it and no "Wisp Science"
  row. It drags the window and zooms it on a double click (as the system
  setting says), through the component library's `TitleBar`. Every page is
  framed under it (`window_frame`); the home page puts its top bar in it, a
  project's page its tabs and actions, the other pages have it plain for
  now.
- On Windows the window draws its own bar too, the same 40 px in the
  margin colour, with no traffic-light inset (12 px) and the system's
  minimize, maximize and close at its right end (the component library's
  `TitleBar`, which marks them as the system's own hit areas, so maximize
  offers Windows 11's snap layouts). Its empty parts drag the window and a
  double click on them maximizes or restores it; a 5 px strip along its top
  is left to the system so the top edge still resizes a window that is not
  maximized. In the bar, what takes a click and is not a button (a tab,
  the bell) occludes, so the system does not take it for the caption. The
  window keeps its title for the taskbar; it is not drawn in the bar.
- Linux keeps the system's title bar (drawing our own needs client-side
  decorations); the home page's and a project's bar is then a strip at the
  top of the page, and the other pages have none. The choice per platform
  is `surfaces::OWN_TITLE_BAR`.
- **What waits for the user** (`render_waiting`): on a project's page left
  of the four icon buttons, apart from them, and on the home page left of
  the documentation and settings, the bell and the count in the waiting
  colour, its tooltip Needs you, shown only while something waits (on the
  home page across projects). A click opens a floating list under it at
  the right (`Layer::Inbox`; card ground, `radius_lg`, a hairline, a
  shadow; 360 px, its height capped): one record row for each waiting
  conversation, its time, `chat`, its title, its project faded before it
  only when that is another one. A click opens the conversation, in its
  project, and closes the list; so do Escape, a press beside it, and
  leaving the project's page.

### Home page

- No sidebar. The window's bar holds the wordmark at the left and, at the
  right, the places to go as icon and word (Research assistant, Mandates,
  Calendar, Scheduled, Favorites, each with its full name as tooltip), then
  the documentation and the settings as icon buttons.
- Below it one centred column, at most 880 px wide: the greeting (Display),
  the search field (white, a line, `radius`), then the ways to start (New
  project filled; Open folder and Import outlined, with icons), on one line
  that does not wrap. At its right end "More ways to start" (a chevron:
  the rest of these, not more actions) opens a menu
  (`Layer::StartMenu`, closed by Escape) of the rarer ways, each an icon
  and a word: Example conversations, Bring over from WebView, Join from
  another device, Sample project.
- Without a model, a bar to act on (`surfaces::bar`: white, a hairline,
  the waiting colour's `circle-alert`, the sentence, Open Settings as an
  outlined `gear` and word) stands under the ways to start.
- Projects are cells side by side, two to a line, starred ones first (a
  last one alone takes the whole line, `home::last_cell_spans`), with
  hairlines between them and round the grid (`radius_lg`). A cell shows the
  name (Title), a waiting stamp with the bell and the count when something
  needs the user, the folder (a Fact), and a line of icons and numbers
  (conversations, artifacts, last touched, and what is running), whose
  tooltip says them in words. Its actions (keep first, settings, new
  window, the two exports, remove) are icon buttons with tooltips that
  show over the cell's top right corner under the pointer, taking no height
  of their own; a click anywhere else on the cell opens the project.
- Recent conversations are record rows under the projects: when (the clock
  today, Yesterday, then a date), the conversation icon, the project and the
  title, its status as a stamp.

### Project page

- The sidebar is the margin colour. The project's name at its top is a
  Title. Its navigation rows are an icon (inline size, faded; ink when
  chosen or under the pointer) and a word: New conversation `plus`, Files
  `folder`, Memos `note-plus`, Capabilities `sparkles`, Journey
  `research-trail`, Papers `book`, Favorites `star`, Settings `gear`. The
  search field is white with the fields' line.
- When the window is too short for the conversation list to keep six tenths of
  the sidebar as the navigation column stands (`present::nav_folds`,
  reckoned from fixed row heights so it does not flicker: a sidebar under
  about 870 px), the seven places fold into one row of icon buttons in the
  same order, each named by its tooltip (New conversation with its key),
  the open one in ink on white; it wraps only when the sidebar is too
  narrow for seven. The search stays under it, the project's row and
  Settings stay, and the rest goes to the conversations. It unfolds when
  the window grows.
- What waits for the user is not in the sidebar (it is the bell in the
  window's bar), so the conversations have the room. The conversation
  list is headed by
  `chat` (or the archive's `inbox`) and the count, then three icon buttons
  (new group, sort and group, select) and More (import, the archived
  conversations; `Layer::ListMenu`).
- A conversation's row is its title; its state is an icon alone at the
  right end in the state's colour, the word as tooltip
  (`present::session_mark`: running `loader`, waiting `circle-ellipsis`,
  archived `inbox`, pinned `pin`). The open conversation is a white row
  with a hairline, `radius`; branches keep their indent and `branch`. A
  group is its chevron, its name and its count as a fact; its own More
  stays.
- The conversation keeps a reading column (`surfaces::READING_WIDTH`),
  centred in the space the sidebar and the side panel leave. The composer
  sits in the same column. So do the bars above the transcript (no model
  to send with, with Open Settings as an outlined icon and word; an error;
  a notice with its dismiss button): `surfaces::status_bar` cards at the
  top of the column with the composer box's edges
  (`surfaces::above_transcript`), not strips across the centre.
- Codex's file citations in an answer (`:codex-file-citation{path=…}`)
  are drawn as links named after the file (`present::answer_markdown`,
  `wisp_server::citations`), in the transcript, the side chat,
  favourites, the exported document and the shared page; an absolute path
  links as a `file:` address, which the system opens.
- What waits between the transcript and the composer (approvals and plans
  to approve, the line about a review, hooks that failed, a drafted memory,
  the questions to ask next, the transfer tray) and the composer's own
  strips (an interpreter's console, the model's thinking, queued messages,
  the conversation's timer, the full-access question) are bars in that
  column, their edges on the composer box's at any width. A bar is a card (`surfaces::bar`): white,
  a hairline, `radius_lg`, no shadow and no tint; its status is the
  theme's colour on an icon or a word, and it has at most one filled
  button. Bars stack with a small gap; approvals and transfers scroll
  inside a capped height, so a stack of them never takes the transcript.
- **The window's bar** (`render_project_bar`) is the page's only header:
  the sidebar's toggle at its left end; then the tabs (`surfaces::window_tab`),
  one for each conversation opened in the window and always one for the
  conversation in view, each its object's icon (`chat`, or `doc` for a
  document in the centre), its name and a close button, the tab in view on
  the paper with a hairline round it, the others faded on the margin; and
  at the right end four icon buttons with tooltips: search, the terminals,
  the side panel and More. There is no header row under the bar, and the
  model's name is not repeated there (the composer shows it).
- What can be done with the conversation is a menu under More, in groups
  with a hairline between, each item an icon and a word
  (`Workspace::menu_item`): the outline and a refresh; rename, copy title,
  pin, group, move; export, share as HTML, debug request; trajectory,
  review, bring back (on a branch), research archive, save as skill;
  archive and delete. Group and Move open beside it, and the outline opens
  in its place. Nothing is listed across the page.
- **The transcript is a record of entries** (spec §7.2), with no gutter
  and no vertical line: everything shares the reading column's text edges
  (`surfaces::ENTRY_TEXT`, 24 px in from each side). Each of the user's
  messages opens an entry with one head line (`surfaces::entry_head`):
  `№ n` and the time in faded fact text on the text edge; the entry whose
  turn runs has a solid 7 px teal dot before the number. A hairline across
  the text column (`entry_rule`, theme `border`) with 14 px above and below
  (`ENTRY_SPACE`) separates an entry from the one before; none before the
  first. The question row draws both, so the list measures them. `n`
  counts the user's messages from 1 (`present::entry_number`, the one place
  it is counted).
- **The question** is the user's message, left-aligned on the paper with no
  bubble or fill (`surfaces::question`): a Title (regular) when it is short, one
  paragraph of at most 120 characters (a Chinese character counts as one,
  `present::title_question`); anything longer is body text in ink. Past
  three lines it is folded, with Show all / Show less tight under it. What
  it carried (files, conversations and skills) stays under it; its actions
  lie over its top right corner and take no line.
- **Rhythm in an entry**: 8 px from the question to what follows, 8 px
  between a paragraph and a group of record rows either way, and between
  entries 14 px, the hairline, 14 px, then the next entry's head.
- **Tool calls are record rows** (`record_row`), consecutive ones one group:
  the icon of what they work on (`present::tool_icon`: run `flask`, file
  `doc`, command `terminal`, approval `hand`, search `search`, another agent
  `swarm`, else `wrench`), the tool's name in the fact face, or what its
  input names (a shell's command line, a file's path, a search's pattern or
  query; `present::tool_summary`) with the name as a faded lead (an ACP
  agent's call shows its title), a stamp for the outcome
  (`present::tool_outcome`; none when the row does not say) and the
  duration as the value. The rows sit on the answers' text edge. A click
  opens a row: its input and output, as code in a well, under it, indented
  to its text, on the paper, with the hairline after them.
- **A call that runs something is the run's row**: `flask`, the run's
  title with where it ran after it, the run's own state as a stamp (live
  while it goes on), how long it has run as the value, then icon buttons to
  ask aside about it (`bubble`) and open it in the Runs tab
  (`external-link`). The rest of the run's card (cancel, background, its
  host, its output) is what the row opens to. Thoughts (`brain`), a review kept
  with the conversation (`review`, its outcome as a stamp; a click shows the
  report) and a branch brought back (`branch`) are record rows too.
- Markdown (answers, the side chat, a `.md` preview, favourites, reports)
  goes through `surfaces::markdown`: blocks stand about 0.6 em apart
  (`PARAGRAPH_GAP`), before and after lists, code and tables too. The
  component library drew no gap at all; the vendored copy fixes it
  (`vendor/gpui-component/WISP-PATCH.md`). Closing CJK punctuation can
  still start a line: GPUI 0.2.2's wrapper has no line-breaking rules.
- Whether a model is there is one answer (`settings::has_model`: a usable
  saved or `WISP_*` model, or Magpie), for the start-up jump to Settings,
  the home page's bar, the composer's line and the guide's model step.
- Wisp's answers are body text on the paper, in the UI face at Regular
  with its Chinese face named outright (`surfaces::body_face`; left to the
  system the Chinese came out a step heavier). The answer a turn delivers
  through `attempt_completion` is Wisp's answer like any other
  (`present::is_answer`, `present::as_answer`): drawn as body text with the
  entry's actions and what the turn cost, and taken as the answer by copy,
  quote, favourites, find, the outline, share and export. A completion
  that failed or still runs stays a record row; an answer with no text
  draws nothing. Questions the model asked,
  plans to approve, the working plan and the cards of work handed to other
  agents stay cards in the column; the compaction line stays a line.
- A conversation that cannot be written to has no message box: one bar
  stands in its place with a status icon, the sentence that says why and
  at most the one action that leads on (`read_only_card`). The reason is
  said there only, not again in a bar above the transcript.
- The small actions under a message are icon buttons with tooltips: copy,
  favourite, ask aside (`bubble`), and More, whose menu (`Layer::MessageMenu`, closed
  by Escape) holds the rest: save to memory, quote, quote / reuse, and by
  the message's kind edit and send again, branch, review the turn, start an
  exploration. Quote and Quote / reuse are `square-pen`, End quote here
  `check`. What the turn cost is a Fact at the right end. They show
  while the pointer is over the message, and stay under the latest one and
  one whose menu is open. A question, and an answer the entry goes on after
  (a tool call, a thought), lay them over their top right corner, so no
  line of them stands between the text and what follows.
- **The composer** is one white box (`radius_lg`, the `input.border` line,
  the teal ring on focus): the text on top, one line of controls under it.
  Its text, chips and the `+` icon are on the answers' text edge, and
  the box is wider than the text by its own padding (`BOX_PAD`, 12 px)
  and its border on each side (spec §7.3); the `+` button reaches
  `ICON_INSET` left of the edge, and the send button keeps the 12 px inset
  on the right. No number beside or inside it. The bars above it and the
  composer's own strips take exactly the box's edges; all of them go
  through `surfaces::column_box`.
  - At the left end a `+` icon button opens a menu (`Layer::PlusMenu`) of
    what can be put into the message, each an icon and a word with its key
    in fact text: attach files, add a project file, conversation or skill
    (`@`), quick actions, commands, workflows and skills (`/`); and under
    a hairline the modes, checked while on: plan first, full access (which
    still asks first), review turns, fast (where the model offers it).
    Typing `/` or `@` opens the same lists as before, over the `+` button.
  - In the middle the conditions line (`surfaces::condition`): a segment
    for each condition, its icon and its current value as a fact in ink,
    no label (the tooltip says what it is), 14 px apart, no dots, pills or
    borders; under the pointer a faint ground with 4 px corners. Who runs
    it (`swarm`, an ACP agent or Wisp, before the first message), an ACP
    agent's own modes and options, the model (`bot`), the effort (`gauge`),
    where runs start (`computer` or `server` and the host's name), the
    expert (`user`). A click opens the picker it had before. A mode at its
    usual value is not in the line; one that is on is a teal segment, icon
    and word, and a click turns it off.
  - At the right end how full the context is as a fact (`8%`, the sentence
    in the tooltip, the waiting colour from 90%; a click shows what the
    model is sent), then the send button (`surfaces::send_button`): a
    28 px teal square with `arrow-up`, its tooltip naming the send key,
    dimmed when there is nothing to send. While a turn runs it is the stop
    button (`stop`), with a `clock` icon button before it to queue what is
    typed.
  - `+` stays first and send last on the first line; the conditions wrap
    onto a second line before they push either out.
  - What the running turn has used so far is a fact line above the box
    while it runs; once the turn is saved, the entry's actions show it.
- An empty conversation shows one Display line and one filled button.
- The side panel is the margin for tools: what the answer rests on, the
  files it wrote, runs. It is the page's right margin as the sidebar is its
  left one, and its parts read as notes in that margin:
  - **Frame** (`side_panel`): the margin colour with the sidebar's
    hairline on the side that faces the conversation. It is 340 px wide
    (480 px while a file is previewed) and gives way down to 240 px before
    the conversation goes below its own 300 px; the conversation takes all
    the rest, so its reading column narrows only once its margins are gone.
  - **Tab strip** (`panel_tab`): one line that never wraps. Each tab is
    its object's icon at toolbar size (`PanelTab::icon`, each its own: Files
    `folder`, Runs `flask`, Environments `server`, Artifacts `layers`,
    Notebook `code`, Provenance `timeline`, Agents `swarm`, Highlights
    `highlighter`, Memos `note-plus`, Memory `memory`, Aside `bubble`). The
    open tab is a white chip with a hairline, its icon and name in ink, Runs
    with the count still going; the others are faded icon buttons with
    their name as tooltip. They come in the panel's order as far as the
    width allows (`panel::strip`, measured); the rest go under a chevron
    (`chevron-down`, "More tabs") at the strip's right, which also holds
    Remove this tab and the tabs that can be added back, each an icon and a
    word (`Layer::PanelTabMenu`). The strip has no close cross: the bar's
    side-panel toggle and Escape close the panel. Its last button stands
    `surfaces::RIGHT_INSET` (12 px) from the window's edge, as the bar's
    last does, so the two sit on one vertical line (on macOS and Linux; on
    Windows the bar ends at the system's buttons).
  - **Header** (`panel_header`): a tab with actions over its whole list,
    or a group in it with its own (Files and a host's folders, a file's
    preview, Artifacts, Highlights, Memos, Memory's habits and notes, the
    memo and memory editors), opens on one row: its title as body text in
    ink, a faded count or size after it, its actions as icon buttons at
    the right end, a hairline under it. A tab with none (Runs, Environments,
    Notebook, Provenance, Agents, Aside) opens on its list, the strip naming
    it. What a tab is for is one faded caption line under the header
    (`panel_note`).
  - **Record rows** for the lists of things: Files (`folder` or `doc`, the
    name, a file's size as the value; open in the centre, rename and move
    to the trash, last in the danger colour, as icon buttons under the
    pointer; a found file has its folder after it), Runs (the same row as
    the conversation's, `run_record_row`: title, where it ran, its state as
    a stamp, its time, ask aside and stop at its end; it opens to its
    output, its search, its host's leftovers and its clean-up), Artifacts
    (the file, where and when after it, its actions as icon buttons, its
    versions and origin under it) and Provenance (the tool's icon, name,
    outcome stamp and time, opening to its input and output). Statuses in
    Agents and Environments are stamps (`surfaces::status`).
  - **Files' header**: the folder, then up, a new file, a new folder and
    More (upload, reload, a terminal here; `Layer::FilesMenu`). Under it
    the white search field, the list/tiles toggle and the order, as icon
    buttons. The place (this project or a host) and Artifacts' scope are
    segmented controls (`surfaces::segments`): one white box, the chosen
    segment faint teal, each an icon and a name.
  - **Rows** (`panel_row`, `panel_entry`, `panel_label`): 32 px or more,
    a hairline under each, the leading label in ink, metadata in
    faded caption, status in the theme's colour on a word or an icon, the
    list hover on a row that opens something. A row that opens what is
    under it keeps the hairline under that. An entry of several lines (a
    run, a host, a batch, a cell, an aside) has the same inset and line.
  - **Cards** (`panel_card`): `card`, white on the margin with a hairline
    and no shadow, for things to act on one by one (memos, habits and notes,
    saved highlights, a run's clean-up question, a host's storage form).
  - **Sections** (`panel_section`): a heading inside a tab is caption in
    faded ink; no bold, no capitals.
  - **Empty** (`panel_empty`): nothing to list, or still reading it, is
    one faded sentence, with at most one outlined button that leads on.
    No icon, no Display.
  - Everything is inset `px_3` from the panel's edges, so a tab's content
    starts where the tab's name does and switching tabs does not move it;
    a file's preview keeps that inset round its text, tables, pictures,
    pages, molecules and alignments.
- A project file opened in the centre (Open in centre) is a document on the
  page, not a tool at the margin:
  - **Tab**: it follows its conversation's own tab in the window's bar, in
    that tab's shape (`window_tab`: the document icon, its name, a close
    button); the tab in view is the one on the paper, so
    while a document is shown the conversation's tab is not, and a click on
    it brings the conversation back.
  - **Document**: it takes the conversation's place under the
    conversation's header, on the paper, as the preview the Files tab draws
    (`preview_parts`): the same header row (its name in ink, a faded
    note, its actions as icon buttons, a hairline under it), the same bars
    and the same inset. The centre draws no preview of its own.
  - **A file's header** has at most four icon buttons: Back first, at the
    left of the name (in the side panel; the centre has none); after the
    name where to show it (Open in centre from the side panel, the split
    in the centre), Attach to the message, and More (`Layer::FilePreviewMenu`):
    copy the path, keep as an artifact, ask aside about the selection,
    where it came from, show in the file manager, open with the system's
    application, each an icon and a word.
  - **Split**: the document on the left; a divider, a hairline in a 5 px
    grip that goes teal under the pointer and drags; the conversation on the
    right in a column of its own, 38% of the centre until dragged and never
    under 320 px (`centre::CHAT_MIN`). Inside that column the transcript, the
    bars and the composer keep the reading column, as everywhere. The
    document takes the rest, and a drag leaves it at least 240 px. Splitting
    puts the side panel away, as the WebView does with its right pane; if it
    is opened again it gives way first (down to its 240 px), then the
    document, while the conversation keeps its 320 px: the centre does not
    go under 565 px while it is split (`centre::SPLIT_MIN`).

### Settings

- **Frame**: the navigation at the left on the margin colour, each page an
  icon and a word (General `adjustments`, Network `share`, Remote access
  `sync`, Chat apps `bubble`, Pet `sparkles`, Models `bot`, ACP agents
  `swarm`, Decision model `check-square`, Conversation `chat`, Experts
  `user`, Quick actions `file-play`, Workflows `split`, Skills `skill`,
  Plugins `grid`, Browser `compass`, Connections `link`, Credentials `lock`,
  Environments `server`, Permissions `shield`, Hooks `fork`, Usage `gauge`,
  Storage `database`), the groups' captions faded, the search field white
  with the fields' line. Every page is one column centred on the paper,
  at most `SETTINGS_WIDTH` (680 px) wide (`surfaces::settings_column`), its
  name a Title at its top; there is no header band, nothing is pinned to
  the left edge and no page splits into two panes. A page's body starts
  from `settings_page`, which adds only the page's error line.
- **One row shape** (`setting`, through `item_row`): the name in ink at the
  regular weight and, only where the name is not enough, one faded note
  under it, on the left; the control on the right, centred on the row; a
  hairline under it. A note is one short line, at most `NOTE_LIMIT` (64)
  characters in either language (a test checks the English and its
  Chinese); what it cannot say goes into the tooltip of a small faded
  `circle-help` icon beside the name (`explained_setting`), the heading
  (`settings_heading_help`) or the page's short note (`settings_note_help`). A control that needs the column's width (a long text, a
  group of chips, the workflow map) goes under its name
  (`stacked_setting`). Headings inside a page are faded captions
  (`surfaces::settings_heading`), and a sentence about the page or a group
  is a faded note (`settings_note`).
- **Controls**: one of a few (language, appearance, send key, when Wisp
  starts, provider, where new runs start, an approval scope, a tool's
  rule) is a segmented control (`surfaces::segmented` of `segment`, an icon
  and a word, or `plain_segment`, a word): one white box with the fields'
  line and `radius`, hairlines between the segments, the chosen one faint
  teal with teal ink at the regular weight. Appearance is System `monitor`,
  Light `sun`, Dark `moon`; this computer is `computer` and a host
  `server`. A longer or open list of choices (palettes, saved models, tools
  to install, capabilities, a workflow's tasks) is a wrapping group of
  chips (`choice_chip`): white with the line, the chosen one with a teal
  line and teal ink; a palette's chip is its colour's dot and its name.
  On/off is a switch. Text size and code size are steppers
  (`surfaces::stepper`): `type-decrease` | the size as a fact | `type-increase`
  in one box, and an `undo` icon button back to Wisp's own size. Segments,
  chips and a stepper's ends are `pressable`: Tab reaches them, a ring shows
  the focus, Enter or Space press them.
- **Fields** are white with the `input.border` line and `radius`
  (`surfaces::input`), never the grey fill. A value that cannot be changed
  here (a model from `WISP_*` variables, a detected path, a code to copy)
  is a fact in ink on the paper (`surfaces::read_only`), not a disabled
  field.
- **Lists with a form** (Models, ACP agents, Experts, Quick actions,
  Workflows, SSH hosts, Hooks, MCP servers, Plugins, the skill store, the
  browser's hosts, the lab's members, credentials of your own, tool rules
  and grants): the list is record rows at the top of the column, the chosen
  one on white; what opens under an item (a test result, a question before
  removing it) is the row's `below`; the chosen item's form follows under
  the list as rows of the one shape. Cards stay only for what is acted on
  in place: a sign-in or a binding under way.
- **Actions on an item**: the one used most is icon and word on an
  outlined button (`surfaces::outlined_action`: Check a host, Test a model
  or an agent, Run a workflow, Sign in); the others are icon buttons with
  tooltips (`icon_action`); a destructive one is last, in the danger colour
  (`danger_action`). No row of words standing as links. At most one filled
  teal button on a page (pages shown together count as one: Chat apps with
  the lab, Models with the subscriptions); its label stays at the regular
  weight.
- **Models**: when the model comes from `WISP_*` variables, one bar with an
  info icon says so at the top, and the form is names and values only:
  facts in ink, an empty value as what it means in faded fact text (the
  provider's own endpoint, From the model catalog, Provider default,
  Follows the Network page, the key Saved or Not set), no notes and no
  actions (Save, Test connection, Use for new conversations, Remove). Otherwise the saved models are record rows (Default and a
  problem as stamps), Add a model and the providers to start from are
  outlined, and the form's actions are Save (filled), Test connection
  (outlined), Use for new conversations (`star`) and Remove last in the
  danger colour. The subscriptions follow as record rows, the Codex pool's
  accounts under their row.
- **General**: the interface font and the code font are two rows, each a
  white field at the column's right edge, with Apply outlined under them at
  the same edge. A stepper's value is a fact at the regular weight in ink;
  the fact face falls back to the UI's Chinese face for a Chinese word.
- **Environments**: the tools to install as checkable chips and one
  Install button; where new runs start as a segmented control; SSH hosts as
  record rows (the name, the address after it, the check as a stamp, Not
  checked as a waiting stamp whose tooltip says why work does not run there
  yet) with Check outlined and the terminal, change and remove as icon
  buttons; where the tools are as rows of the tool's name and a white field
  whose hint is the path the search path finds, said once.
- No weight above regular anywhere in Settings.

### Other pages

- **Frame**: a page with a rail (Research journey, Paper evidence,
  Research calendar, Scheduled tasks, Research mandates, Favorites) keeps
  it on the margin colour with Back, the page's name as a Title and, beside
  the name, a `circle-help` icon whose tooltip says what the page is for
  (`surfaces::help`); no paragraph in the rail. The content scrolls beside
  the rail in a centred column of the reading width (`page_scroll`,
  `page_column`), not pinned to the left.
- **Research journey**: By day or Relationships is a segmented control; the
  month is a fact between its arrow icon buttons, with This month an icon
  button after them; the search is a white field; Add a record is an
  outlined icon and word. Each day is headed by its date as a faded fact,
  with Draft a recap as an icon and a word at its right; its records are
  record rows: the time, the kind's icon (`journey::kind_icon`: asked
  `chat`, run `flask`, file `doc`, decision `check-square`, next step
  `arrow-right`, a note of one's own `note-plus`), what happened, the first
  line of what it says faded after it, a run's state as a stamp. The source
  details at the right are a side panel (`side_panel`, `panel_header`,
  `panel_section`, `panel_empty`).
- **Research calendar**: a day's records are the same record rows, the
  project faded after each.
- **Paper evidence**: the papers in the rail are `book` rows; New paper and
  Add a figure, table or claim are outlined icon and word buttons. A
  paper's header has Finalize… as the outlined icon and word, the capsule,
  a new version and rename as icon buttons, Remove last in the danger
  colour; its versions are a segmented control; a source's kind is a faded
  fact and its selection and a rerun's Matches / Differs are stamps.
- **Scheduled tasks** and **Research mandates** have nothing to list in a
  rail, so they have none: back, the name with its help and the New button
  (outlined icon and word) head the centred column (`surfaces::page_head`).
  A task's Run now is a `play` icon button and Delete a danger icon button
  last; its cadence, next and last run are facts, the next run only while
  it is on. Templates are chips with their prompt as tooltip. The daily
  recap's explanation is a help icon and its Save is outlined. A mandate's
  Run now (`play`) and Open its conversation (`chat`) are icon and word,
  pause, report, edit and close icon buttons, Delete a danger icon last;
  what it asks is a white card (`bar`) with Answer outlined at its right.
- **Research calendar**: the weekdays go through the translation table; the
  month is a fact between arrow icon buttons; the projects shown are choice
  chips with a check.
- **Favorites**: the kind filter is a segmented control (`doc`, `code`,
  `image`); saved items are record rows (when, the kind's icon, the title,
  where from faded after it).
- **Titles from a message's first line** drop Markdown's inline marks
  (`present::title_line`): recent conversations, the bell's list, Favorites,
  the journey.
- **Segmented controls** hug their segments; they never stretch across a
  column.
- **The command palette** lists record rows: the object's icon from the
  vocabulary, the name, where it is faded after it, its actions as icon
  buttons; the chosen row is on white with a hairline.
- **Entry numbers** count from the host's `user_offset` for the earliest
  page loaded, so № stays the same whether or not earlier pages are loaded.
- **Statuses** on these pages (a recap's, a run's, a mandate's, a task the
  assistant runs, a verdict) are stamps (`surfaces::status`); fields are
  white (`surfaces::input`), in dialogs too.

## Controls

- **On/off**: a switch (`surfaces::switch`), never a button that says On or
  Off. It sits at the right end of its row, teal when on and a shade of the
  page when off (in every palette), with the setting's name to its left and
  a tooltip that says what it switches; Tab reaches it and Enter or Space
  flips it. A test in `surfaces.rs` rejects a button labelled On or Off.
- **Dialogs**: `surfaces::dialog(id, width, title)`: a card with a Title, a
  backdrop and a shadow, in one of three widths for dialogs
  (`DialogWidth::Narrow` 520 px for a short form or a question, `Regular`
  640 px for a list or a longer form, `Wide` 760 px for a record to read),
  plus one for a dialog of two panes side by side (`TwoPanes`, 1000 px),
  used by Example conversations and not by a dialog of one column. Its body scrolls inside 90% of
  the window's height. The footer has what concerns the whole dialog (a
  count, a guide's steps, Select all) at the left, then the actions:
  secondary ones, Cancel or Close, and the main action at the right end,
  the dialog's one filled button. A dialog closes on a click beside it
  (`close_on_click_outside`) unless it holds work a stray click would lose,
  a draft or a guide part way through; one with no footer has a close button
  in its title row. A test in `surfaces.rs` checks every dialog is drawn
  this way and that nothing else dims the window but the palette.
- **Icons**: the shared stroke set in `apps/gpui/assets/icons`, drawn at a
  stroke of 1.5 on the 24 grid in `currentColor` in both frontends (a test
  checks every file and the WebView's `compose_icon`); no emoji or text
  glyphs as icons. Three sizes (`IconSize`): 16 inline and in record
  rows, 18 on toolbars and icon buttons, 20 in navigation; they follow the
  text size. Three colours (`IconTone`): faded ink as they usually are, ink
  under the pointer or when chosen, teal while a mode is on. Status colours
  are the stamp's, and the danger colour a delete's. `surfaces::sized_icon`
  draws one; an icon in a faded state turns to ink in an `ICON_HOVER` group.
- **More and the rest** (spec §9.3): `more`, three horizontal dots, is
  "more actions on this thing"; "the rest of these" (a strip's or a
  toolbar's overflow, the start menu) is `chevron-down`. Two adjacent
  controls never share an icon with different meanings, and a panel a
  toggle closes has no cross of its own. `close` is inset (7 to 17) so it
  is optically the size of its neighbours.
- **Icon buttons** (`surfaces::icon_action`): a toolbar-size icon in a
  28 px square (more than the 26 a pointer needs), with a tooltip its
  signature requires. An icon and a word (`surfaces::labelled_action`) is
  for navigation and actions with consequences. The rest of the app moves
  onto these in later steps; the older `icon_button` stays until then.
- **Scrolling**: a column, rail, list, menu or dialog body whose content
  can be taller than its space scrolls under a stable id rather than being
  cut. A menu or list under a button is never taller than the window has
  room for (`surfaces::menu_room`; a message's menu, opened anywhere in
  the transcript, half the window). The conversation menu is the one
  exception for now: its Group and Move choices open beside it, and a
  scrolling menu would clip them.
- **Escape**: every dismissible layer joins the window's stack in
  `apps/gpui/src/layers.rs`.

## Where the rules live in code

| Rule | File |
| --- | --- |
| Colours, radius, base sizes | `apps/gpui/assets/wisp-theme.json` |
| The rem and the palettes | `apps/gpui/src/theme.rs` |
| Titles, display text, facts (`fact`), icons (`sized_icon`, `icon_action`, `labelled_action`), stamps (`stamp`, `stamp_with`), record rows (`record_row`, `record_group`), the window's bar (`window_titlebar`, `window_bar`, `window_frame`), cards, dialogs (`dialog`, `DialogWidth`), the reading column, the bars above the composer (`bar`, `above_composer`), the on/off switch | `apps/gpui/src/workspace/surfaces.rs` |
| The bundled faces and their registration | `apps/gpui/assets/fonts`, `apps/gpui/src/assets.rs` |
| The transcript's entries (`entry_row`, `EntryDot`, `question`) and the bar's tabs (`window_tab`, `tab_close`) | `apps/gpui/src/workspace/surfaces.rs` |
| A project's bar, its tabs and its four buttons | `apps/gpui/src/workspace/tabs_view.rs` |
| Entry numbers, tool icons and outcomes | `apps/gpui/src/present.rs` |
| The side panel: its frame (`side_panel`), tabs (`panel_tab`), headers (`panel_header`), rows (`panel_row`, `panel_entry`, `panel_label`, `PANEL_ROW_HEIGHT`), cards (`panel_card`), sections (`panel_section`), notes (`panel_note`) and empty lines (`panel_empty`) | `apps/gpui/src/workspace/surfaces.rs` |
| A document in the centre: the preview it shares with the Files tab (`preview_parts`), its tab, the split and its widths | `apps/gpui/src/workspace/files_panel.rs`, `apps/gpui/src/workspace/centre_view.rs`, `apps/gpui/src/centre.rs` |
| A setting's row: name and note on the left, control on the right | `setting` in `apps/gpui/src/workspace/settings_sections.rs` |
| The settings column (`settings_column`, `SETTINGS_WIDTH`), headings and notes (`settings_heading`, `settings_note`), white fields (`input`), facts that cannot be changed (`read_only`), segmented controls (`segmented`, `segment`, `plain_segment`), chips (`choice_chip`), steppers (`stepper`), outlined and destructive actions (`outlined_action`, `danger_action`), what the keyboard presses (`pressable`) | `apps/gpui/src/workspace/surfaces.rs` |
| A setting whose control needs the width; the shape both share (`stacked_setting`, `item_row`) | `apps/gpui/src/workspace/settings_sections.rs` |

## Where it stands

The first change that follows this page restyles the theme, the home page,
the conversation's column, its actions and the composer's settings; a second
puts a switch in place of every On/Off button; a third puts the bars above
the composer in its column; a fourth draws every dialog from one shape, makes
every heading a Title and keeps shadows to what floats; a fifth makes the
side panel the page's margin. The page is applied in full. A sixth is the
lab record's first step: the bundled faces, Medium titles, the 4 / 6 radius,
the window's own bar on macOS, the icon, stamp and record-row helpers, and
the home page drawn from them. A seventh puts a project's tabs and actions
in the window's bar and makes the transcript a record of numbered entries
with tool calls as record rows. For the project page they check the bar's four icon
buttons, tabs from `window_tab`, no header row of the page's own and every
header action kept in More; the transcript's rows framed by `entry_row`
and numbered by `present::entry_number`; the question with no bubble; tool
calls, thoughts, reviews and branches only through `record_row`; and
three icon buttons and More under a message. `present.rs` checks the
numbering, the tool icons and the outcomes. A tenth gives the full pages a
rail note as a help icon, a centred column, record rows for the days'
records, segmented switches and icon actions; tests check those rails hold
no paragraph, the journey and calendar draw records through `record_row`
with the kind's icon, the column is centred, and a paper's and a task's
actions are icons. An eighth makes the composer
one box of `+`, conditions and send, and lightens the pages: icon stroke
1.5, regular titles, lighter faded ink and hairlines, and stamps without
icons. Tests check that the `+` menu holds every way to put something into
the message and every mode, a mode shows in the line only while on and
then in teal, the segments carry tooltips, send is an icon button, faded
ink reaches 4.5:1 on page and margin, and every icon is stroke 1.5. A
ninth makes the side panel's strip one line of icons and its lists record
rows, and the sidebar's states icons: tests check every tab's icon is its
own and drawn, the strip neither wraps nor uses a word for an unchosen tab,
files, runs, artifacts and provenance come through `record_row`, a
conversation's state is an icon with a tooltip, and the list's heading has
at most three icon buttons and More.
A tenth restyles Settings: every page one centred column of setting rows
under its Title, segmented controls, chips, steppers, white fields and
facts for what cannot change, lists as record rows with the form under
them, and no weight above regular. Tests check that every page is drawn from
`settings_page` inside `settings_column` with `setting` rows, that fields
are white, that no settings file uses a selected button, a field of its own
or a heavier weight, that a page has at most one filled button, that
appearance, language and the send key are segmented, that segments and
chips are reached with the keyboard, and that every note is at most
`NOTE_LIMIT` characters in English and in Chinese.

Tests in `surfaces.rs` hold it there: text sizes only by role, shadows only
on what floats, every dialog from `dialog`, the bars above the composer in
its column, switches for on/off, and for the side panel its frame from
`side_panel` with no colour or line of its own, tabs drawn by `panel_tab`
with no pill or icon for a name, every tab's empty line from `panel_empty`,
a header from `panel_header` exactly where the rules above give one, and its
cards, rows, entries and sections from their helpers with no bold, hand-made
card or shadow; and a document in the centre drawn from the Files tab's
`preview_parts` and no preview of its own, its conversation at least
`CHAT_MIN` wide in a centre of at least `SPLIT_MIN`. The one exception they allow is Provenance's input and
output, which are code in a well as the conversation shows code.

They also check the lab record's first step: titles in the bundled serif at
`MEDIUM` with Chinese serifs and then the UI's Chinese face as fallbacks
(no SimSun), facts in the bundled monospace with tabular figures, the
bundled files' family names and weights (in `assets.rs`), the 4 / 6 radius
of both themes (in `theme.rs`), three icon sizes and an icon button of at
least 26 px with a required tooltip, the stamp's word at 4.5:1 on the page
in both themes, a record row's fixed columns, the window's bar in the
margin colour framing every page, and the home page's recent conversations
as record rows.
