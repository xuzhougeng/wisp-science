# GPUI design language

The GPUI desktop reads like a lab notebook on warm paper: ivory surfaces, ink
text, one teal accent and serif titles. It continues the look the WebView
desktop already has, so a researcher who moves over recognises the product.
The direction was chosen on 2026-11-15 from four candidates and is called
*paper notebook* (纸本笔记).

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

## Text

Four roles. The rem is set so that the component library's helpers come out
at these sizes when the body is 14 px (`theme::rem_for`); the General setting
for text size moves all four together.

| Role | Code | Size | Face | Use for |
| --- | --- | --- | --- | --- |
| Caption | `text_xs()` | 12 | UI | Metadata, hints, timestamps, counts |
| Body | none, or `text_sm()` | 14 | UI | Everything else; the window's default |
| Title | `surfaces::title` | 18 | Serif | A page, a section, a dialog, a conversation, a project |
| Display | `surfaces::display` | 30 | Serif | At most one per page: the home greeting, the empty conversation |

- The UI face is the system's (`.SystemUIFont`). The serif face is Georgia,
  falling back to each system's Chinese serif (Songti SC, Source Han Serif,
  Noto Serif CJK, SimSun), so a title is serif in either language. Code is
  the component library's monospace face.
- Weights: regular for text, `MEDIUM` for the label that leads a row,
  `SEMIBOLD` for serif titles. No bold.
- Rank is said with colour (`muted_foreground`), not with a smaller size.
  Caption is for metadata, not for making a secondary sentence quieter.
- Views do not use `text_base()`, `text_lg()`, `text_xl()`, `text_2xl()`,
  `text_3xl()` or a size in pixels; a test in `surfaces.rs` rejects them. The
  terminal's grid is the one exception. A heading is `surfaces::title`; a
  figure or a code that should stand out is body text in `MEDIUM`.

## Colour

The values live in `apps/gpui/assets/wisp-theme.json`; views use the theme's
names and never a colour of their own.

| Name | Light | Dark | Use for |
| --- | --- | --- | --- |
| Paper (`background`) | `#faf9f6` | `#171614` | The page |
| Margin (`sidebar`) | `#f0eee6` | `#1c1b18` | The sidebar |
| Card (`popover`) | `#ffffff` | `#1f1e1b` | Cards, the composer, dialogs, menus |
| Ink (`foreground`) | `#141413` | `#ebe8e2` | Text |
| Faded ink (`muted_foreground`) | `#5a574e` | `#a39e94` | Secondary text |
| Line (`border`) | ink at 12% | paper-white at 10% | Hairlines |
| Teal (`primary`) | `#0f766e` | `#2da898` | The main action, links, focus |

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

- **Radius**: 8 px for controls and rows (`radius`), 12 px for cards, the
  composer, dialogs and menus (`radius_lg`). Pills only for counts and tags.
- **Cards**: `surfaces::card`: a white surface with a hairline and no
  shadow. Hover darkens the line; it does not lift the card.
- **Shadows**: dialogs and menus only (with the palette and the cards
  for other projects' conversations, which also float over the page). A test in `surfaces.rs` names
  every function that may draw one and rejects a shadow anywhere else.
- **Spacing**: the rem steps (`gap_1` … `gap_8`, `p_*`), not pixels. Pixels
  are for layout widths.
- **Rows**: a row in a list or the sidebar is 32 px high or more, with a
  hairline between rows of a list on the page.

## Layout

### Home page

- No sidebar. A slim bar across the top holds the wordmark at the left and
  the places to go, as words, at the right.
- Below it one centred column, at most 880 px wide: the greeting (Display),
  the search field, then the ways to start (New project filled; Open folder
  and Import outlined; the rarer ways as plain text).
- Projects are cards in two columns, starred ones first. A card shows the
  name (Title), the folder, the counts and the age; its actions appear in the
  last line.
- Recent conversations are a list under the projects, one line each with a
  hairline between, the status at the right end.

### Project page

- The sidebar is the margin colour. The project's name at its top is a
  Title; the selected conversation is a white row with a hairline.
- The conversation keeps a reading column (`surfaces::READING_WIDTH`),
  centred in the space the sidebar and the side panel leave. The composer
  sits in the same column. So do the error and notice lines above the
  transcript, inset as the bars above the composer are.
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
- The conversation's name in the bar above it is a Title. What can be done
  with the conversation (rename, pin, group, move, export, review, archive,
  delete) is a menu under the bar's More button, in groups with a hairline
  between; Group and Move open beside it. Nothing is listed across the page.
- A conversation that cannot be written to has no message box: one bar
  stands in its place with a status icon, the sentence that says why and
  at most the one action that leads on (`read_only_card`). The reason is
  said there only, not again in a bar above the transcript.
- The small actions under a message show while the pointer is over the
  message, and stay under the latest one.
- In the composer a setting at its usual value is an icon with a tooltip
  (plan first, ask first, review turns); one that changes what Wisp does
  says so in words (Planning, Full access, Reviewing). The model, the
  effort and where runs start always show their value. The settings wrap
  before they push Send out of the box.
- An empty conversation shows one Display line and one filled button.
- The side panel is the margin for tools: what the answer rests on, the
  files it wrote, runs. It keeps hairlines and cards like the page.

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
- **Icons**: the shared stroke set in `apps/gpui/assets/icons`; no emoji or
  text glyphs as icons.
- **Escape**: every dismissible layer joins the window's stack in
  `apps/gpui/src/layers.rs`.

## Where the rules live in code

| Rule | File |
| --- | --- |
| Colours, radius, base sizes | `apps/gpui/assets/wisp-theme.json` |
| The rem and the palettes | `apps/gpui/src/theme.rs` |
| Titles, display text, cards, dialogs (`dialog`, `DialogWidth`), the reading column, the bars above the composer (`bar`, `above_composer`), the on/off switch | `apps/gpui/src/workspace/surfaces.rs` |
| A setting's row: name and note on the left, control on the right | `setting` in `apps/gpui/src/workspace/settings_sections.rs` |

## Not done yet

The first change that follows this page restyles the theme, the home page,
the conversation's column, its actions and the composer's settings; a second
puts a switch in place of every On/Off button; a third puts the bars above
the composer in its column; a fourth draws every dialog from one shape, makes
every heading a Title and keeps shadows to what floats. Still to bring under
it: the side panel as margin notes.
