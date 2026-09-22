# UI design principles

## Icons

- Interactive icons must be SVGs from the shared UI icon renderer or the existing SVG mask set.
- Do not use Unicode characters, emoji, or text glyphs as icons. Their shape, alignment, and availability vary across Windows, macOS, browsers, and fallback fonts.
- Text remains appropriate for labels, status values, scientific notation, and keyboard hints such as `↑↓` or `⌘K`.
- Icon-only controls must retain an accessible `title` or `aria-label`.

## Buttons

- Standalone CTAs use `.btn-primary` / `.btn-ghost` from `ui/src/styles/base.css`.
- Toolbar rows that already own chrome (modal/settings `.row`, plugin toolbar, plan/approval actions, file retry) use `button.primary` for the filled clay look; do not redefine clay fills per surface.
- Do not use bare `button.primary` for sidebar nav — `.side-btn.primary` is a soft affordance, not a filled CTA.
- Toggle switches use the shared softened clay track and light thumb; keep the full accent fill for primary actions instead of making settings switches visually compete with them.

## Spacing, type, and radius

- Prefer `--space-1`…`--space-7`, `--text-xs`…`--text-display`, and the three radius tiers (`--radius-xs` / `--radius-sm` / `--radius`) from `base.css`.
- Map near-miss radii onto those tiers (6–9→xs, 10–14→sm, 16–22→lg). Keep `999px` pills, `50%` circles, and asymmetric chat bubbles as literals.
- Adopt the scale first on brand surfaces (projects landing, chat empty, research graph); avoid one-off px when extending those surfaces.

## Brand surfaces

- Projects landing and chat empty use the full molecular wordmark (`.brand-wordmark`) with transparent backgrounds. Select its light or dark asset through the app theme; explicit appearance choices override the system preference.
- Keep the wordmark large enough for the “science” lettering to read. Compact chrome and the research-graph empty state retain a small symbol; chat greetings retain the serif typography.
- Research graph headings use Source Serif at `--text-lg`; list/canvas stay utilitarian.
- The projects home puts a documentation control immediately to the right of Settings. It opens the tutorials index, the same page as Help → Documentation.

## Queued follow-ups

- Follow-ups typed while a turn is running sit in a compact card above the composer, not as dashed transcript bubbles.
- The card shows a count, the parked text, and icon actions. Reorder controls appear only when two or more items are waiting. Cut-in stays a labeled clay pill because it is the distinctive action.
- Icon-only queue controls keep both `title` and `aria-label`.
- **Guide now / 立刻引导** in a native API conversation submits the selected
  follow-up to the current loop at its next safe boundary. The current model
  request or tool call may finish first; remaining tool calls from its old plan
  are skipped with paired results, and the next model request receives the
  guidance. Existing messages and completed tool results remain in history.
  Guidance received during automatic context compaction is included before the
  prepared model request is dispatched.
- Queue actions wait for the selected item's backend enqueue acknowledgement,
  even when its optimistic card is already visible. An enqueue failure prevents
  the waiting action from being sent; action failures are shown in the status
  area instead of silently succeeding.
- Queue rows reconcile by their backend id and receive lifecycle updates. A
  queued row with a duplicate body but different attachments remains a distinct
  intent. `Interrupt and replace` marks parked intents with identical text,
  attachments, and references superseded so they cannot be drained again.
  Replacement reserves priority before cancelling the current workflow, so
  waiting queue turns cannot slip in between cancellation and replacement.
- After a cut-in is requested, the row stays visible as “Sent · waiting for the
  current step” until the running loop consumes it. This reflects the safe
  boundary contract; it does not claim to cancel an already running tool.
- A final text response cannot finish a loop while guidance is already pending.
  If the turn has ended, errors, or reaches an explicit stop/iteration limit,
  unconsumed cut-ins take priority over ordinary queued turns. Each message is
  either injected once or handed off once. In-loop guidance uses the queued
  text; fallback turns retain the original attachments and references as well.
  ACP conversations continue to use ordinary queued follow-ups.

## Native question cards

- Selecting an option in a native `ask_user` card fills the composer and leaves
  the card pending. The user can edit the answer, change the selection, or add
  conditions before sending.
- The generated draft contains the option label and, when present, an explicit
  `说明：` line so the option description is not lost from the submitted turn.
- ACP `ask_user` cards continue to resolve through their protocol response path;
  they do not use the native composer-draft behavior.

## Composer attachments and references

- The composer keeps its top-edge resize affordance invisible at rest while preserving the full-width drag target and persisted custom height.
- Context usage sits immediately left of the model picker as a number-free gauge; its needle sweeps from upper-left to upper-right as the active conversation fills its context window.
- The context-usage panel opens docked in the composer column, pushing the transcript up instead of covering it. Dragging the header undocks it into a floating window that stays open while typing; a dock button or double-click returns it. There is no full-screen click-swallowing backdrop.
- Context-usage category rows use semantic elevated, sunken, hover, and accent tokens rather than native button fills, so every light and dark palette keeps the panel visually consistent.
- Files, images, skills, artifacts, conversations, execution environments, and runtime references must remain visually distinguishable before and after send.
- Image attachments use a real thumbnail when the project file is readable. Other files use a document card with a filename and type label.
- Persisted transcript markers such as `Uploaded files:` and `Selected skills:` are transport metadata. The chat UI renders them as cards instead of exposing the raw marker text.
- Opening or reopening a saved conversation shows its latest message. After the user scrolls up within that conversation, deferred content growth preserves the visible reading position; switching conversations starts at the latest message again.
- Long attachment names truncate inside the card; the full value remains available through the control's title.
- Remove controls live inside the related card and retain an accessible label.

## Transcript rendering

- `update_plan` renders an execution checklist with a single completed/total count, segmented progress, and labeled pending, running, completed, and cancelled steps. An accepted update is distinct from completion of the work. Pending or running plans expand by default; only a successful result with every step completed defaults to a compact header. Explicit disclosure choices survive transcript refreshes.
- A compact plan strip above the composer runtime row follows the latest accepted `update_plan` in the current user turn, even when tool history is folded or scrolled away. It shows the current running (or next pending) step and completed/total count. Clicking the strip expands the full checklist in place; a second click collapses it. After every step completes, a dismiss control removes the strip until a later plan appears. Progress advances only from reported step completion; animation indicates activity, never estimated progress. Stopping the turn stops the activity animation, completion remains visible until dismissed or the next user turn, and switching sessions uses that session's transcript. Failed/pending updates retain the last accepted snapshot. Motion respects the reduced-motion preference; full checklists remain in transcript cards.
- Before the tool result arrives, its count-only preview displays an updating notice; step titles come from the actual result. Older results without a checklist show an unavailable notice instead of invented steps. Failed or rejected updates show their error. Tool name, duration (zero milliseconds shown as `< 1 ms`), and raw input/output live in a separate, initially collapsed details section. Each call remains a historical snapshot; existing activity-group folding still applies.
- A live assistant message keeps a throttled Markdown prefix plus an immediate, whitespace-preserving plain-text tail. The Markdown budget adapts from 50 ms for short answers to 150 ms above 8,000 bytes and 300 ms above 32,000 bytes; once the turn settles, the remaining tail is rendered once as full Markdown.
- Turn-boundary affordances such as Undo update inside the existing message row; they must not remount or reparse an unchanged historical answer.
- Collapsed activity summaries, tool details, reasoning, and provenance rows do not keep hidden body DOM. Mount the body when its disclosure opens and remove it when the disclosure closes; headers and status remain available while collapsed.
- Transcript-derived Inspector data (artifacts, notebook cells, saved highlights, and the conversation outline) refreshes on structural events and settled revisions rather than on every text delta. Live tool status and provenance headers may update independently through compact keyed projections.

## Topbar and inspector chrome

- The conversation topbar keeps session tabs as the primary signal. Inbox, terminal, and inspector toggles live in `.topbar-actions`.
- The conversation outline opens from a list icon and question count in the topbar, keeping navigation off the message canvas. Compact panes hide the count while retaining the labeled icon. The outline is a bounded, scrollable card with quieter numbers and timestamps; the selected question has an accent edge, and Escape closes the card before its parent surface.
- Status text appears only when non-empty (or when an API-key action is required) and truncates with a `title` for the full value.
- Specialist labels stay quiet text, not status pills.
- Artifact type badges are neutral mono labels; only tabular data keeps a clay accent. Prefer `--ok` / `--err` / `--clay` over one-off HSL pill colors.

## Responsive workspace layout

- The default 1100 px desktop window keeps the sidebar, conversation, and Inspector as resizable columns. At 960 px and below, the Inspector becomes a modal drawer, preserving the conversation width. After shrinking the window, Escape closes the drawer's own menu first, then the drawer, then any composer menu or conversation outline it covered. Each press dismisses only the topmost layer; growing back restores the split-pane order.
- Conversation messages, runtime controls, and the composer grow together with the available center pane, leaving 16 px outer gutters and capping the column at 1280 px on wide screens. Resizing the window or opening the Inspector recalculates that width through CSS; document/chat split views continue to fill their narrower chat pane.
- Scrollable lists keep stable scrollbar gutters and contain overscroll so a nested list does not unexpectedly move the surrounding workspace.

## Dense settings lists

- Long capability lists expose status filters and a visible/enabled count before the rows.
- Secondary editors such as skill tags stay collapsed until requested; the row keeps a short summary so existing metadata remains discoverable.
- Settings that save on interaction say so explicitly, including when changes apply only to new sessions. Empty filter results show an explanatory state instead of a blank list.
