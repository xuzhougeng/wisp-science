//! The shapes every page shares, from `docs/gpui-design-language.md`: the
//! two text roles above body size with their serif face, the card, the
//! dialog, the conversation's reading column with the bars above its
//! composer, the switch of an on/off setting, and the side panel as the
//! page's right margin with its tabs, headers, rows and empty lines.
//! Views take these instead of choosing a size, a face, a border or a
//! control of their own.
use std::{rc::Rc, sync::Arc};

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, StyledExt, TitleBar,
    button::{Button, ButtonVariants},
    switch::Switch,
    tooltip::Tooltip,
};

use super::shell::icon_button;
use crate::present::Tone;

/// The serif face of titles, bundled with the app (`assets::register_fonts`).
const SERIF: &str = crate::assets::SERIF_FACE;

/// What a title falls back to for the characters the bundled serif does
/// not have. First each system's Chinese serif, so a title is serif in
/// either language; then, where none of them is installed (Windows has
/// only SimSun, which the design leaves out), the Chinese face of the
/// system's UI, so a Chinese title is the UI face, in the title's weight.
/// macOS and Windows honour this order; on Linux the text system picks the
/// fallback itself.
const SERIF_FALLBACKS: [&str; 6] = [
    "Songti SC",
    "Source Han Serif SC",
    "Noto Serif CJK SC",
    "PingFang SC",
    "Microsoft YaHei UI",
    "Noto Sans CJK SC",
];

/// How many of `SERIF_FALLBACKS` are serifs; the rest are UI faces.
#[cfg(test)]
const SERIF_FALLBACK_SERIFS: usize = 3;

/// Titles (a page's, a dialog's, a project's, a question) are the serif at
/// its regular weight, so a page of them stays light.
const TITLE_WEIGHT: FontWeight = FontWeight::NORMAL;

/// The one display line of a page is the serif in `MEDIUM`, never heavier,
/// so its Latin and its Chinese weigh the same.
const DISPLAY_WEIGHT: FontWeight = FontWeight::MEDIUM;

/// The serif face at `weight`, with its fallbacks.
fn serif_at(weight: FontWeight) -> Font {
    let mut face = font(SERIF);
    face.weight = weight;
    face.fallbacks = Some(FontFallbacks::from_fonts(
        SERIF_FALLBACKS
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    ));
    face
}

/// The serif of titles.
fn serif() -> Font {
    serif_at(TITLE_WEIGHT)
}

/// The face of a fact: the bundled IBM Plex Mono, its figures in even
/// columns (`tnum`).
fn fact_face() -> Font {
    let mut face = font(crate::assets::MONO_FACE);
    face.weight = FontWeight::NORMAL;
    face.features = FontFeatures(Arc::new(vec![("tnum".into(), 1)]));
    // A Chinese word in a fact (`Wisp 默认`) is the UI's Chinese face at
    // the regular weight, not whatever heavier face the system picks.
    face.fallbacks = Some(FontFallbacks::from_fonts(
        FACT_FALLBACKS
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    ));
    face
}

/// What a fact falls back to for the characters the monospace does not
/// have: each system's Chinese UI face, at the regular weight.
const FACT_FALLBACKS: [&str; 3] = ["PingFang SC", "Microsoft YaHei UI", "Noto Sans CJK SC"];

/// The face of body text in the transcript (Wisp's answers, a question
/// too long to be a title): the UI's face at the regular weight, its
/// Chinese the UI's Chinese face named outright. Left to the system, the
/// Chinese of the UI face comes out of its cascade a step heavier than
/// the Latin beside it, so an answer read as Medium.
pub(super) fn body_face(cx: &App) -> Font {
    body_font(cx.theme().font_family.clone())
}

fn body_font(family: SharedString) -> Font {
    let mut face = font(family);
    face.weight = FontWeight::NORMAL;
    face.fallbacks = Some(FontFallbacks::from_fonts(
        FACT_FALLBACKS
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    ));
    face
}

/// A fact a machine measured, as the design's second rule says: a time, a
/// duration, a size, a count, a path, an id, a model's name, a shortcut.
/// The monospace face at the caption size (12 px), in faded ink. Not for a
/// sentence that should only look quieter.
pub(super) fn fact(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .font(fact_face())
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// The three sizes an icon comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IconSize {
    /// In a line of text, a record row, beside body text.
    Inline,
    /// On a toolbar, a tab, an icon button.
    Toolbar,
    /// In navigation, the icon vocabulary, an empty state.
    Navigation,
}

impl IconSize {
    #[cfg(test)]
    const ALL: [Self; 3] = [Self::Inline, Self::Toolbar, Self::Navigation];

    /// Its size in pixels at the usual body size.
    pub(super) const fn px(self) -> f32 {
        match self {
            Self::Inline => 16.,
            Self::Toolbar => 18.,
            Self::Navigation => 20.,
        }
    }

    /// The same in rems, so icons grow with the text-size setting as the
    /// text beside them does (the rem is 16 px at the usual body size).
    fn rems(self) -> Rems {
        rems(self.px() / 16.)
    }
}

/// The three colours an icon takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum IconTone {
    /// As it usually is: faded ink, going to ink under the pointer when it
    /// sits in an `ICON_HOVER` group.
    Faded,
    /// Under the pointer or chosen: ink.
    Ink,
    /// A mode that is on: teal.
    // First worn by the composer's modes, a later step of the design.
    #[allow(dead_code)]
    On,
}

/// The hover group an icon in `IconTone::Faded` turns to ink in. An icon
/// button and a record row are such a group; a caller may name any other
/// element one.
pub(super) const ICON_HOVER: &str = "icon-hover";

/// The theme's colour for `tone`.
pub(super) fn icon_color(tone: IconTone, cx: &App) -> Hsla {
    match tone {
        IconTone::Faded => cx.theme().muted_foreground,
        IconTone::Ink => cx.theme().foreground,
        IconTone::On => cx.theme().primary,
    }
}

/// One icon of the shared set at one of the three sizes, in one of the
/// three colours. Status colours are the stamp's, not an icon's.
pub(super) fn sized_icon(name: &str, size: IconSize, tone: IconTone, cx: &App) -> Svg {
    let ink = cx.theme().foreground;
    crate::assets::icon(name)
        .size(size.rems())
        .text_color(icon_color(tone, cx))
        .when(tone == IconTone::Faded, |icon| {
            icon.group_hover(ICON_HOVER, move |style| style.text_color(ink))
        })
}

/// The side of an icon button's square: more than the 26 px a pointer
/// needs.
pub(super) const ICON_BUTTON: f32 = 28.;

/// A button that is only an icon: a toolbar-size icon in a square of
/// `ICON_BUTTON`, faded until the pointer is over it, teal while the mode
/// it turns on is on (`IconTone::On`), ink while it is chosen. The tooltip
/// is its name and cannot be left out; pass it through `tr`.
pub(super) fn icon_action(
    id: impl Into<ElementId>,
    name: &str,
    tooltip: impl Into<SharedString>,
    tone: IconTone,
    cx: &App,
) -> Button {
    Button::new(id)
        .ghost()
        .tooltip(tooltip)
        .group(ICON_HOVER)
        .size(px(ICON_BUTTON))
        .p_0()
        .child(sized_icon(name, IconSize::Toolbar, tone, cx))
}

/// A button that is an icon and a word: a way to go somewhere or an action
/// with consequences. Faded until the pointer is over it, ink while chosen.
/// The word is a child with its own colour, so the component library's
/// red hover never reaches it (see `panel_tab`).
pub(super) fn labelled_action(
    id: impl Into<ElementId>,
    name: &str,
    word: impl Into<SharedString>,
    size: IconSize,
    tone: IconTone,
    cx: &App,
) -> Button {
    let ink = cx.theme().foreground;
    Button::new(id)
        .ghost()
        .small()
        .compact()
        .group(ICON_HOVER)
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_1p5()
                .child(sized_icon(name, size, tone, cx))
                .child(
                    div()
                        .flex_none()
                        .line_height(relative(1.))
                        .text_color(icon_color(tone, cx))
                        .when(tone == IconTone::Faded, |word| {
                            word.group_hover(ICON_HOVER, move |style| style.text_color(ink))
                        })
                        .child(word.into()),
                ),
        )
}

/// One condition in the composer's line: an icon and the current value as
/// a fact in ink, no label (the tooltip carries it), no border or pill.
/// Under the pointer its icon goes to ink on a faint ground. A mode that is
/// on (`IconTone::On`) is teal, icon and word. The caller adds what a
/// click does; one that cannot be changed now is left without.
pub(super) fn condition(
    id: impl Into<ElementId>,
    icon: &str,
    value: impl Into<SharedString>,
    tooltip: impl Into<SharedString>,
    tone: IconTone,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let hover = theme.list_hover;
    let tooltip: SharedString = tooltip.into();
    let ink = if tone == IconTone::On {
        theme.primary
    } else {
        theme.foreground
    };
    div()
        .id(id)
        .group(ICON_HOVER)
        .h_flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_1()
        .h(px(24.))
        .rounded(theme.radius)
        .hover(move |style| style.bg(hover))
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(sized_icon(icon, IconSize::Inline, tone, cx))
        .child(
            div()
                .font(fact_face())
                .text_xs()
                .text_color(ink)
                .max_w(px(220.))
                .truncate()
                .child(value.into()),
        )
}

/// The gap between two conditions in the composer's line.
pub(super) const CONDITION_GAP: f32 = 14.;

/// The composer's send button: a square of `ICON_BUTTON`, teal, with an
/// icon (`arrow-up` to send, `stop` while a turn runs) and its tooltip.
pub(super) fn send_button(
    id: impl Into<ElementId>,
    icon: &str,
    tooltip: impl Into<SharedString>,
    cx: &App,
) -> Button {
    Button::new(id)
        .primary()
        .tooltip(tooltip)
        .size(px(ICON_BUTTON))
        .p_0()
        .rounded(cx.theme().radius)
        .child(
            crate::assets::icon(icon)
                .size(IconSize::Toolbar.rems())
                .text_color(cx.theme().primary_foreground),
        )
}

/// The status a stamp records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StampKind {
    /// Done, saved: the theme's `success`.
    Success,
    /// Waiting for the user, queued: `warning`.
    Waiting,
    /// Failed, stopped, refused: `danger`.
    Failed,
    /// Running, newly written, syncing: `info`.
    Running,
}

impl StampKind {
    #[cfg(test)]
    const ALL: [Self; 4] = [Self::Success, Self::Waiting, Self::Failed, Self::Running];

    /// Its status icon from the shared set.
    pub(super) const fn icon(self) -> &'static str {
        match self {
            Self::Success => "circle-check",
            Self::Waiting => "circle-ellipsis",
            Self::Failed => "circle-alert",
            Self::Running => "loader",
        }
    }

    /// The stamp for a status said with a tone; a neutral one has none.
    pub(super) fn from_tone(tone: Tone) -> Option<Self> {
        match tone {
            Tone::Success => Some(Self::Success),
            Tone::Attention => Some(Self::Waiting),
            Tone::Danger => Some(Self::Failed),
            Tone::Active => Some(Self::Running),
            Tone::Neutral => None,
        }
    }
}

/// Which shade of a status colour a stamp's word wears: on a light page
/// the colour's `active` shade, which reaches 4.5:1 on the paper, on a dark
/// page the colour itself.
pub(super) fn stamp_shade<T>(dark: bool, colour: T, active: T) -> T {
    if dark { colour } else { active }
}

/// The colour of a stamp of `kind`: its line, its icon and its word.
pub(super) fn stamp_color(kind: StampKind, cx: &App) -> Hsla {
    let theme = cx.theme();
    let dark = theme.is_dark();
    match kind {
        StampKind::Success => stamp_shade(dark, theme.success, theme.success_active),
        StampKind::Waiting => stamp_shade(dark, theme.warning, theme.warning_active),
        StampKind::Failed => stamp_shade(dark, theme.danger, theme.danger_active),
        StampKind::Running => stamp_shade(dark, theme.info, theme.info_active),
    }
}

/// A stamp's corner, square but for a touch.
const STAMP_RADIUS: f32 = 2.;

/// The bell's size in the home page's stamp, smaller than its count.
const STAMP_ICON: f32 = 11.;

/// The mark of a status: square corners, a hairline in the status colour,
/// no fill, and a word (a fact, at the regular weight) in that colour; no
/// icon. At most one on a row; counts and tags keep their round pills.
/// Where there is no room for one, the status icon alone in the status
/// colour with the word as its tooltip stands in its place.
pub(super) fn stamp(kind: StampKind, word: impl Into<SharedString>, cx: &App) -> Div {
    stamp_box(kind, cx).child(word.into())
}

/// A stamp with an icon before its word, for the one status whose object
/// says more than a word: the home page's bell over a count of what waits
/// for the user.
pub(super) fn stamp_with(
    kind: StampKind,
    icon: &str,
    word: impl Into<SharedString>,
    cx: &App,
) -> Div {
    let ink = stamp_color(kind, cx);
    stamp_box(kind, cx)
        .child(
            crate::assets::icon(icon)
                .size(rems(STAMP_ICON / 16.))
                .text_color(ink),
        )
        .child(word.into())
}

/// The box of a stamp of `kind`, in its colour.
fn stamp_box(kind: StampKind, cx: &App) -> Div {
    let ink = stamp_color(kind, cx);
    div()
        .h_flex()
        .flex_none()
        .items_center()
        .gap_1()
        .px_1p5()
        .rounded(px(STAMP_RADIUS))
        .border_1()
        .border_color(ink)
        .font(fact_face())
        .text_xs()
        .line_height(relative(1.5))
        .text_color(ink)
}

/// A status said with a tone, as the design says a status: a stamp for
/// success, waiting, failure or work going on; a neutral one (a tag, a
/// state with nothing to flag) as a faded fact.
pub(super) fn status(word: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    match StampKind::from_tone(tone) {
        Some(kind) => stamp(kind, word, cx),
        None => fact(word, cx).flex_none(),
    }
}

/// How high a record row is at least.
pub(super) const RECORD_ROW_HEIGHT: f32 = 32.;

/// The narrowest a record row's time column is: room for `17:04`, `昨天`
/// or `11-15` in the fact face. A group whose times are longer (`2025-11-15`,
/// `Yesterday`) widens it for all its rows (`record_time_width`).
pub(super) const RECORD_TIME_WIDTH: f32 = 52.;

/// How wide one column of the fact face is at the caption size: IBM Plex
/// Mono's advance is 0.6 em, and the caption is 12 px.
const FACT_COLUMN: f32 = 7.2;

/// How many columns `text` takes in the fact face: one for each Latin
/// character, two for each wide (Chinese, Japanese, Korean) one.
fn fact_columns(text: &str) -> usize {
    text.chars()
        .map(|ch| if (ch as u32) >= 0x1100 { 2 } else { 1 })
        .sum()
}

/// The width in pixels at the usual body size that the time column of a
/// group of rows takes: the widest of its times, and never less than
/// `RECORD_TIME_WIDTH`, so a group's icons line up and the time sits close
/// to them.
/// A group with no times at all has no time column: 0, and its rows start
/// with the icon.
pub(super) fn record_time_width<'a>(times: impl IntoIterator<Item = &'a str>) -> f32 {
    times
        .into_iter()
        .map(|time| (fact_columns(time) as f32 * FACT_COLUMN + 2.).ceil())
        .map(|width| width.max(RECORD_TIME_WIDTH))
        .fold(0., f32::max)
}

/// The width of a record row's value at its right end, when its group has
/// values.
pub(super) const RECORD_VALUE_WIDTH: f32 = 72.;

/// One record: from left to right its time, the icon of its object, what
/// it says, a stamp and a value. The time and the icon keep their columns
/// when they are empty, and so does the value once `value_column` (or
/// `value`) is set, so the rows of a group line up. Stack rows with
/// `record_group`, which draws the hairline above the first; each row
/// draws the one under it.
pub(super) fn record_row(id: impl Into<ElementId>, content: impl Into<SharedString>) -> RecordRow {
    RecordRow {
        base: div().id(id),
        time: None,
        icon: None,
        lead: None,
        content: content.into(),
        stamp: None,
        stamp_tooltip: None,
        value: None,
        value_column: false,
        time_width: 0.,
        below: None,
        suffix: None,
        mono: false,
        actions: Vec::new(),
    }
}

/// What `record_row` makes. It takes a click, a hover and a tooltip like a
/// `div` with an id.
#[derive(IntoElement)]
pub(super) struct RecordRow {
    base: Stateful<Div>,
    time: Option<SharedString>,
    icon: Option<&'static str>,
    lead: Option<SharedString>,
    content: SharedString,
    stamp: Option<(StampKind, SharedString)>,
    /// What the stamp's word stands for, said in full under the pointer.
    stamp_tooltip: Option<SharedString>,
    value: Option<SharedString>,
    value_column: bool,
    /// The time column's width in pixels at the usual body size; its group
    /// sets it for all its rows.
    time_width: f32,
    /// What the row opens to, indented under it; the hairline goes after it.
    below: Option<AnyElement>,
    /// What follows the content in faded ink (`· local`).
    suffix: Option<SharedString>,
    /// The content is an identifier, set in the fact face in ink.
    mono: bool,
    /// Icon buttons at the right end, after the value.
    actions: Vec<AnyElement>,
}

impl RecordRow {
    /// When it happened, as a fact: `17:03`, `Yesterday`, `11-03`.
    pub(super) fn time(mut self, time: impl Into<SharedString>) -> Self {
        let time = time.into();
        self.time_width = self.time_width.max(record_time_width([time.as_ref()]));
        self.time = Some(time);
        self
    }

    /// Where or what else, in faded ink after the content (`· local`).
    pub(super) fn suffix(mut self, suffix: impl Into<SharedString>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }

    /// Sets the content in the fact face, in ink: it is an identifier (a
    /// tool's name, a command, a path) rather than a sentence.
    pub(super) fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// An icon button at the right end of the row, after its value.
    pub(super) fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    /// The icon of its object, from the vocabulary: `flask` for a run,
    /// `doc` for a file, `chat` for a conversation.
    pub(super) fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    /// What the content belongs to, in faded ink before it (`Server ·`).
    pub(super) fn lead(mut self, lead: impl Into<SharedString>) -> Self {
        self.lead = Some(lead.into());
        self
    }

    /// Its status, as a stamp of `kind` with `word`.
    pub(super) fn stamp(mut self, kind: StampKind, word: impl Into<SharedString>) -> Self {
        self.stamp = Some((kind, word.into()));
        self
    }

    /// The sentence the stamp's word stands for, as its tooltip: `Not
    /// checked` says in full why work does not run there yet.
    pub(super) fn stamp_tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.stamp_tooltip = Some(tooltip.into());
        self
    }

    /// A value at the right end, as a fact: `14.0 s`, `1.5 KB`.
    pub(super) fn value(mut self, value: impl Into<SharedString>) -> Self {
        self.value = Some(value.into());
        self.value_column = true;
        self
    }

    /// What the row opens to (a call's input and output, a run's card),
    /// shown under it, indented to its text, with the row's hairline after
    /// it. A click on it is not a click on the row.
    pub(super) fn below(mut self, below: Option<AnyElement>) -> Self {
        self.below = below;
        self
    }

    /// Keeps the value's column with nothing in it, for a row in a group
    /// whose other rows have values.
    pub(super) fn value_column(mut self) -> Self {
        self.value_column = true;
        self
    }
}

impl Styled for RecordRow {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for RecordRow {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for RecordRow {}

impl RenderOnce for RecordRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        // Cut short on one line, the whole of it under the pointer.
        let mut whole = self.content.to_string();
        if let Some(lead) = &self.lead {
            whole = format!("{lead} · {whole}");
        }
        if let Some(suffix) = &self.suffix {
            whole = format!("{whole} · {suffix}");
        }
        let whole = SharedString::from(whole);
        let content = div()
            .id("record-content")
            .h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .when(self.mono, |content| content.font(fact_face()).text_xs())
            .tooltip(move |window, cx| Tooltip::new(whole.clone()).build(window, cx))
            .children(self.lead.map(|lead| {
                div()
                    .flex_shrink_0()
                    .max_w(relative(0.4))
                    .truncate()
                    .text_color(muted)
                    .child(format!("{lead} ·"))
            }))
            .child(div().flex_shrink().min_w_0().truncate().child(self.content))
            .children(self.suffix.map(|suffix| {
                div()
                    .flex_shrink_0()
                    .max_w(relative(0.4))
                    .truncate()
                    .text_color(muted)
                    .child(format!("· {suffix}"))
            }));
        let line = cx.theme().border;
        // With no time column there is no time and no gap after it.
        let timed = self.time_width > 0.;
        let inset = if timed {
            self.time_width + RECORD_TO_TEXT
        } else {
            RECORD_TO_TEXT - 10.
        };
        let below = self.below.map(|below| {
            div()
                .w_full()
                .pl(rems(inset / 16.))
                .pr_1()
                .pb_2()
                .child(below)
        });
        let open = below.is_some();
        let row = self
            .base
            .group(ICON_HOVER)
            .h_flex()
            .items_center()
            .flex_shrink_0()
            // 10 px from the time to the icon and from the icon to the text.
            .gap_2p5()
            .px_1()
            .py_1()
            .min_h(px(RECORD_ROW_HEIGHT))
            .when(!open, |row| row.border_b_1().border_color(line))
            .when(timed, |row| {
                row.child(
                    div()
                        .flex_none()
                        // In rems, so it grows with the text as the fact does.
                        .w(rems(self.time_width / 16.))
                        .children(self.time.map(|time| fact(time, cx).truncate())),
                )
            })
            .child(
                div().flex_none().w(IconSize::Inline.rems()).children(
                    self.icon
                        .map(|name| sized_icon(name, IconSize::Inline, IconTone::Faded, cx)),
                ),
            )
            .child(content)
            .children(self.stamp.map(|(kind, word)| {
                let mark = stamp(kind, word, cx);
                match self.stamp_tooltip {
                    Some(said) => div()
                        .id("record-stamp")
                        .flex_none()
                        .tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
                        .child(mark)
                        .into_any_element(),
                    None => mark.into_any_element(),
                }
            }))
            .when(self.value_column, |row| {
                row.child(
                    div()
                        .flex_none()
                        .w(px(RECORD_VALUE_WIDTH))
                        .h_flex()
                        .justify_end()
                        .children(self.value.map(|value| fact(value, cx).truncate())),
                )
            })
            .when(!self.actions.is_empty(), |row| {
                row.child(div().h_flex().flex_none().gap_0p5().children(self.actions))
            });
        match below {
            None => row.into_any_element(),
            Some(below) => div()
                .w_full()
                .v_flex()
                .border_b_1()
                .border_color(line)
                .child(row)
                .child(below)
                .into_any_element(),
        }
    }
}

/// How far a record's text starts past its time column: the row's inset,
/// the icon and the two gaps beside it, in pixels at the usual body size.
const RECORD_TO_TEXT: f32 = 4. + 10. + 16. + 10.;

/// Rows of records stacked, with a hairline above the first. Their time
/// column is one width, the widest of their times (`record_time_width`).
pub(super) fn record_group(rows: impl IntoIterator<Item = RecordRow>, cx: &App) -> Div {
    let mut rows: Vec<RecordRow> = rows.into_iter().collect();
    let times: Vec<String> = rows
        .iter()
        .filter_map(|row| row.time.as_ref().map(ToString::to_string))
        .collect();
    let width = record_time_width(times.iter().map(String::as_str));
    for row in &mut rows {
        row.time_width = width;
    }
    div()
        .v_flex()
        .border_t_1()
        .border_color(cx.theme().border)
        .children(rows)
}

/// The width of the transcript's gutter, where an entry's number and time
/// stand left of the record line.
pub(super) const ENTRY_GUTTER: f32 = 64.;

/// Where the first line of a question sits in its row, from the row's top:
/// the row's padding and half a line of the title face. The gutter's
/// number and the dot on the line are set against it.
const ENTRY_FIRST_LINE: f32 = 8. + 13.;

/// The dot an entry has on the record line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EntryDot {
    /// Not the start of an entry: the line runs past.
    None,
    /// An entry: a hollow ink dot.
    Open,
    /// The entry whose turn is running: a solid teal dot.
    Live,
}

/// One row of the transcript as part of the record: the gutter at the left,
/// with the entry's number and time when the row opens one; then the
/// content column, whose left edge is the record line. Every row draws its
/// own stretch of the line through its full height, so the line runs
/// through the transcript without a gap; space between rows is padding
/// inside them. An entry's dot sits on the line at the question's first
/// line.
pub(super) fn entry_row(
    number: Option<usize>,
    time: Option<SharedString>,
    dot: EntryDot,
    content: AnyElement,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let gutter = div()
        .flex_none()
        .w(px(ENTRY_GUTTER))
        .v_flex()
        .items_end()
        .pr_3()
        .pt(px(ENTRY_FIRST_LINE - 9.))
        .children(number.map(|number| fact(entry_label(number), cx)))
        .children(time.map(|time| fact(time, cx)));
    const DOT: f32 = 9.;
    let dot = (dot != EntryDot::None).then(|| {
        let (fill, line) = match dot {
            EntryDot::Live => (theme.primary, theme.primary),
            _ => (theme.background, theme.foreground),
        };
        div()
            .absolute()
            .top(px(ENTRY_FIRST_LINE - DOT / 2.))
            .left(px(0.5 - DOT / 2.))
            .size(px(DOT))
            .rounded_full()
            .border_1()
            .border_color(line)
            .bg(fill)
    });
    // A plain flex row: its children stretch to its height, so the line
    // runs from top to bottom.
    div().w_full().flex().child(gutter).child(
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .border_l_1()
            .border_color(theme.border)
            .child(content)
            .children(dot),
    )
}

/// How an entry's number is written in the gutter.
pub(super) fn entry_label(number: usize) -> String {
    format!("№ {number}")
}

/// How high a folded question stands: three lines of the title face.
pub(super) const QUESTION_FOLD_LINES: f32 = 3.;

/// The question that opens an entry, left-aligned on the paper with no
/// bubble: a short one (`present::title_question`) in the Title role, the
/// serif at its regular weight; a longer one as body text in ink. A long one is cut
/// to three lines while `folded`.
pub(super) fn question(title: bool, folded: bool, cx: &App) -> Div {
    // A line of the title (18 px at 1.45) or of body text (14 px at 1.5),
    // in rems.
    let line = (if title { 18. * 1.45 } else { 14. * 1.5 }) / 16.;
    div()
        .when(title, |question| {
            question.font(serif()).text_lg().line_height(relative(1.45))
        })
        .when(!title, |question| {
            question.font(body_face(cx)).line_height(relative(1.5))
        })
        .when(folded, |question| {
            question
                .max_h(rems(line * QUESTION_FOLD_LINES))
                .overflow_hidden()
        })
}

/// A tab in the window's bar: its object's icon and its name, the one in
/// view on the paper with a hairline round it, the others in faded ink on
/// the margin. The caller adds the close button and what a click does.
pub(super) fn window_tab(
    id: impl Into<ElementId>,
    icon: &str,
    name: impl Into<SharedString>,
    selected: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let paper = theme.background;
    div()
        .id(id)
        .group(ICON_HOVER)
        // In the window's bar: on Windows, not the caption to drag.
        .occlude()
        .h_flex()
        .flex_shrink_0()
        .items_center()
        .gap_1p5()
        .pl_2()
        .pr_0p5()
        .h(px(30.))
        .max_w(px(220.))
        .rounded(theme.radius)
        .border_1()
        .text_sm()
        .when(selected, |tab| {
            tab.bg(paper)
                .border_color(theme.border)
                .text_color(theme.foreground)
        })
        .when(!selected, |tab| {
            tab.border_color(theme.transparent)
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(move |tab| tab.bg(paper.opacity(0.6)))
        })
        .child(sized_icon(
            icon,
            IconSize::Inline,
            if selected {
                IconTone::Ink
            } else {
                IconTone::Faded
            },
            cx,
        ))
        .child(div().min_w_0().truncate().child(name.into()))
}

/// The close button of a tab: an icon button with its tooltip.
pub(super) fn tab_close(
    id: impl Into<ElementId>,
    tooltip: impl Into<SharedString>,
    cx: &App,
) -> Button {
    icon_action(id, "close", tooltip, IconTone::Faded, cx)
}

/// Whether the window draws its own title bar, one choice per platform:
/// - macOS: yes, with the traffic lights inside it (the system draws them
///   over a transparent title bar).
/// - Windows: yes. GPUI hides the system's caption for a transparent title
///   bar and asks the page which part of it is what (`WM_NCHITTEST`): the
///   component library's `TitleBar` marks its empty parts as the caption
///   to drag and draws minimize, maximize and close as the system's own
///   hit areas, so Windows still moves, snaps and resizes the window.
/// - Linux: no. Drawing the bar there needs client-side decorations and
///   the compositor's say, a different job; the system's bar stays and
///   ours is a strip on the page under it.
pub(super) const OWN_TITLE_BAR: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// Whether the system's traffic lights sit in our bar (macOS only).
const TRAFFIC_LIGHTS_IN_BAR: bool = cfg!(target_os = "macos");

/// How high the window's bar is.
pub(super) const TITLE_BAR_HEIGHT: f32 = 40.;

/// Where macOS puts the close button's corner in the bar, so the traffic
/// lights sit centred in it, inset from the window's edge.
const TRAFFIC_LIGHTS: (f32, f32) = (14., 13.);

/// Where the bar's own content starts on macOS: past the traffic lights.
const TRAFFIC_LIGHTS_INSET: f32 = 84.;

/// Where it starts where there are no traffic lights (Windows, and macOS in
/// full screen).
const BAR_INSET: f32 = 12.;

/// How far the last icon button of the window's bar, and of the side
/// panel's tab strip under it, stands from the window's right edge, so the
/// two sit on one vertical line (the side panel's content is inset as
/// much, `px_3`). On Windows the bar ends at the window's own buttons
/// instead.
pub(super) const RIGHT_INSET: f32 = 12.;

/// How a window is opened: on macOS and Windows with a transparent title
/// bar so the page draws its own (`window_bar`), on Linux with the
/// system's. The title is not drawn in the bar but still names the window
/// to the system (the Dock, the taskbar).
pub fn window_titlebar() -> TitlebarOptions {
    TitlebarOptions {
        title: Some("Wisp Science".into()),
        appears_transparent: OWN_TITLE_BAR,
        traffic_light_position: TRAFFIC_LIGHTS_IN_BAR
            .then(|| point(px(TRAFFIC_LIGHTS.0), px(TRAFFIC_LIGHTS.1))),
    }
}

/// The bar across the top of the window, in the margin colour with a
/// hairline under it, `content` laid out in it. On macOS and Windows it is
/// the window's title bar, through the component library's `TitleBar`: on
/// macOS it leaves room for the traffic lights and zooms on a double click;
/// on Windows it ends with the system's three window buttons, its empty
/// parts drag the window and a double click on them maximizes it, and a
/// thin strip along its top is left to the system so the top edge still
/// resizes the window. On Linux, under the system's own title bar, it is a
/// bar on the page, drawn only when there is something to put in it.
///
/// On Windows an element in the bar that takes a click and is not a button
/// (a tab, the bell) must `occlude()`, so the system is told it is not the
/// caption; buttons take their press themselves (`prevent_default`).
pub(super) fn window_bar(
    content: Option<AnyElement>,
    window: &Window,
    cx: &App,
) -> Option<AnyElement> {
    let theme = cx.theme();
    if OWN_TITLE_BAR {
        let inset = if TRAFFIC_LIGHTS_IN_BAR && !window.is_fullscreen() {
            px(TRAFFIC_LIGHTS_INSET)
        } else {
            px(BAR_INSET)
        };
        // Our content ends with a gap before the window buttons on
        // Windows; on macOS before the window's edge.
        let content = content.map(|content| {
            div()
                .h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .items_center()
                .pr(px(RIGHT_INSET))
                .child(content)
        });
        // Windows: the top edge of a window that is not maximized resizes
        // it. GPUI asks the page first and would take the bar's caption
        // there; a strip that is not the caption leaves it to the system.
        let resize_edge = (cfg!(target_os = "windows") && !window.is_maximized()).then(|| {
            div()
                .id("window-top-edge")
                .absolute()
                .top_0()
                .left_0()
                .right_0()
                .h(px(5.))
                .occlude()
        });
        return Some(
            TitleBar::new()
                .relative()
                .h(px(TITLE_BAR_HEIGHT))
                .pl(inset)
                .bg(cx.theme().sidebar)
                .border_color(theme.sidebar_border)
                .children(content)
                .children(resize_edge)
                .into_any_element(),
        );
    }
    let content = content?;
    Some(
        div()
            .h_flex()
            .flex_shrink_0()
            .items_center()
            .h(px(TITLE_BAR_HEIGHT))
            .pl_4()
            .pr(px(RIGHT_INSET))
            .bg(cx.theme().sidebar)
            .border_b_1()
            .border_color(theme.sidebar_border)
            .child(content)
            .into_any_element(),
    )
}

thread_local! {
    /// The height of the window being drawn, noted as its draw starts
    /// (`note_window_height`); every window is drawn on this thread, one at
    /// a time, so a menu reads the height of its own.
    static WINDOW_HEIGHT: std::cell::Cell<f32> = const { std::cell::Cell::new(800.) };
}

/// What the window's bar, a menu's anchor and the composer under an
/// upward menu leave a menu of the window's height, with a margin.
const MENU_RESERVE: f32 = 150.;

/// However short the window, a menu shows this much and scrolls the rest.
const MENU_LEAST: f32 = 120.;

/// Notes the height of the window about to be drawn, for `menu_room`.
pub(super) fn note_window_height(height: f32) {
    WINDOW_HEIGHT.with(|cell| cell.set(height));
}

/// How tall a menu, a popover or a list under a button may be: `cap` (or
/// no cap), and never more than the window has room for, so in a short
/// window it scrolls instead of running off the window. A menu opened
/// from a row in the transcript, upwards or downwards, has half.
pub(super) fn menu_room(cap: Option<f32>, half: bool) -> Pixels {
    px(room_in(WINDOW_HEIGHT.with(|cell| cell.get()), cap, half))
}

fn room_in(height: f32, cap: Option<f32>, half: bool) -> f32 {
    let room = if half {
        height / 2. - TITLE_BAR_HEIGHT
    } else {
        height - TITLE_BAR_HEIGHT - MENU_RESERVE
    };
    let room = room.max(MENU_LEAST);
    cap.map_or(room, |cap| room.min(cap))
}

/// How far apart the blocks of a Markdown text are: paragraphs, lists,
/// code and tables, about 0.6 em of body text (14 px), so an answer's
/// paragraphs read as paragraphs.
pub(super) const PARAGRAPH_GAP: f32 = 0.55;

/// The style of Markdown text: the component library's, with the paragraph
/// gap (`PARAGRAPH_GAP`, in rems).
pub(super) fn text_style() -> gpui_component::text::TextViewStyle {
    gpui_component::text::TextViewStyle::default().paragraph_gap(rems(PARAGRAPH_GAP))
}

/// Markdown text, as answers, documents, reports and previews show it,
/// with its paragraphs apart (`text_style`).
pub(super) fn markdown(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut App,
) -> gpui_component::text::TextView {
    gpui_component::text::TextView::markdown(id, text, window, cx).style(text_style())
}

/// A whole window: its bar (`window_bar`) over the page, which takes the
/// rest.
pub(super) fn window_frame(bar: Option<AnyElement>, page: impl IntoElement) -> Div {
    div().v_flex().size_full().children(bar).child(
        div()
            .flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .relative()
            .child(page),
    )
}

/// The one large line a page may have: the home greeting, the question an
/// empty conversation asks.
pub(super) fn display(text: impl Into<SharedString>) -> Div {
    div()
        .font(serif_at(DISPLAY_WEIGHT))
        .text_3xl()
        .line_height(relative(1.3))
        .child(text.into())
}

/// The name of a page, a section, a dialog, a conversation or a project.
pub(super) fn title(text: impl Into<SharedString>) -> Div {
    div().font(serif()).text_lg().child(text.into())
}

/// A surface that sits on the page: white on paper, a hairline around it
/// and no shadow.
pub(super) fn card(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
}

/// The three widths a dialog comes in, and one for a dialog that lays out
/// two panes side by side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DialogWidth {
    /// A short form or a question: a name, a code, a choice among a few.
    Narrow,
    /// A list to choose from, or a form of several fields.
    Regular,
    /// A record to read: a report, a trajectory, what the model is sent.
    Wide,
    /// Two panes side by side, a list and what is chosen in it to read:
    /// Example conversations. Not for a dialog of one column.
    TwoPanes,
}

impl DialogWidth {
    #[cfg(test)]
    const ALL: [Self; 3] = [Self::Narrow, Self::Regular, Self::Wide];

    /// Its width in pixels; in a window narrower than that it takes 94% of
    /// the window.
    pub(super) const fn px(self) -> f32 {
        match self {
            Self::Narrow => 520.,
            Self::Regular => 640.,
            Self::Wide => 760.,
            Self::TwoPanes => 1000.,
        }
    }
}

/// A dialog: a card of one of three widths over a backdrop that takes the
/// whole window, with a Title, a body that scrolls inside a capped height,
/// and a footer whose main action is at the right end, Cancel or Close to
/// its left. Children go into the body, and its style is the body's (a
/// dialog may set the gap between its parts). `id` names the card; the
/// backdrop and the body are named after it.
pub(super) fn dialog(
    id: impl Into<SharedString>,
    width: DialogWidth,
    title: impl Into<SharedString>,
) -> Dialog {
    let id = id.into();
    Dialog {
        body: div()
            .id(SharedString::from(format!("{id}-body")))
            .v_flex()
            .gap_2(),
        id,
        width,
        title: title.into(),
        height: None,
        close: None,
        on_outside: None,
        aside: Vec::new(),
        actions: Vec::new(),
    }
}

/// What `dialog` makes.
#[derive(IntoElement)]
pub(super) struct Dialog {
    id: SharedString,
    width: DialogWidth,
    title: SharedString,
    height: Option<f32>,
    close: Option<Button>,
    on_outside: Option<Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App)>>,
    body: Stateful<Div>,
    aside: Vec<AnyElement>,
    actions: Vec<AnyElement>,
}

impl Dialog {
    /// Closes it on a press beside the card. A dialog that holds work the
    /// user would lose (a draft, a guide part way through) leaves this out,
    /// so only its own buttons and Escape close it.
    pub(super) fn close_on_click_outside(
        mut self,
        handler: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_outside = Some(Box::new(handler));
        self
    }

    /// A quiet close button at the right end of the title's row, for a
    /// dialog with no footer to hold one.
    pub(super) fn close_button(
        mut self,
        id: impl Into<ElementId>,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.close = Some(icon_button(id, "close", "Close").on_click(handler));
        self
    }

    /// A fixed height in pixels. The body then fills it without scrolling,
    /// for a dialog that lays out panes that scroll on their own.
    pub(super) fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// Something at the left end of the footer: a count, the steps of a
    /// guide, an action on the whole list.
    pub(super) fn aside(mut self, child: impl IntoElement) -> Self {
        self.aside.push(child.into_any_element());
        self
    }

    /// The next of the footer's actions, from left to right: secondary
    /// actions, then Cancel or Close, then the main action last.
    pub(super) fn action(mut self, child: impl IntoElement) -> Self {
        self.actions.push(child.into_any_element());
        self
    }

    /// Several of the footer's actions, in order.
    pub(super) fn actions(mut self, children: impl IntoIterator<Item = impl IntoElement>) -> Self {
        self.actions
            .extend(children.into_iter().map(IntoElement::into_any_element));
        self
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl Styled for Dialog {
    fn style(&mut self) -> &mut StyleRefinement {
        self.body.style()
    }
}

/// What a dialog stands on: the whole window, dimmed, and nothing under it
/// clickable while it is open.
fn backdrop(id: SharedString, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().overlay)
}

/// The card of a dialog: white, a hairline, `radius_lg` and the one shadow
/// a floating layer has.
fn dialog_card(width: DialogWidth, cx: &App) -> Div {
    div()
        .v_flex()
        .gap_3()
        .w(px(width.px()))
        .max_w(relative(0.94))
        .max_h(relative(0.9))
        .p_5()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .shadow_lg()
        // A click inside is not a click on the backdrop.
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

impl RenderOnce for Dialog {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let fixed = self.height.is_some();
        let body = self
            .body
            .min_h_0()
            .when(fixed, |body| body.flex_1())
            .when(!fixed, |body| body.overflow_y_scroll());
        let header = div()
            .h_flex()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .child(title(self.title).flex_1().min_w_0())
            .children(self.close);
        let footer = (!self.aside.is_empty() || !self.actions.is_empty()).then(|| {
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .flex_shrink_0()
                .children(self.aside)
                .child(div().flex_1())
                .children(self.actions)
        });
        let card = dialog_card(self.width, cx)
            .id(self.id.clone())
            .when_some(self.height, |card, height| card.h(px(height)))
            .child(header)
            .child(body)
            .children(footer);
        backdrop(SharedString::from(format!("{}-backdrop", self.id)), cx)
            .when_some(self.on_outside, |backdrop, handler| {
                backdrop.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    handler(event, window, cx)
                })
            })
            .child(card)
    }
}

/// The control of an on/off setting: the theme's switch, teal when on.
/// `what` names what it switches and shows as its tooltip; the row beside
/// it carries the setting's name and note, as `setting` lays them out.
pub(super) fn switch(id: impl Into<ElementId>, on: bool, what: impl Into<SharedString>) -> OnOff {
    OnOff {
        id: id.into(),
        on,
        what: what.into(),
        disabled: false,
        on_change: None,
    }
}

/// What `switch` makes. Unlike the component library's own switch it can be
/// reached with Tab and flipped with Enter or Space, as the buttons it
/// replaces could.
#[derive(IntoElement)]
pub(super) struct OnOff {
    id: ElementId,
    on: bool,
    what: SharedString,
    disabled: bool,
    on_change: Option<Rc<dyn Fn(&bool, &mut Window, &mut App)>>,
}

impl OnOff {
    /// Shown but not flipped, as when the value is not known yet.
    pub(super) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Runs with the value the user asks for, the opposite of the one shown.
    pub(super) fn on_change(
        mut self,
        handler: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl RenderOnce for OnOff {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let focused = focus.is_focused(window);
        let next = !self.on;
        let what = self.what;
        let change = self.on_change.filter(|_| !self.disabled);
        div()
            .id(self.id)
            .flex_shrink_0()
            .rounded_full()
            // A ring for the keyboard's focus; the pointer does not focus it.
            .border_1()
            .border_color(if focused {
                cx.theme().ring
            } else {
                cx.theme().transparent
            })
            .when(!self.disabled, |this| {
                this.track_focus(&focus.tab_stop(true))
            })
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .tooltip(move |window, cx| Tooltip::new(what.clone()).build(window, cx))
            .when_some(change, |this, change| {
                this.on_click(move |_, window, cx| change(&next, window, cx))
            })
            .child(
                Switch::new("switch")
                    .checked(self.on)
                    .disabled(self.disabled),
            )
    }
}

/// The widest a conversation runs, its rows' own padding included.
pub(super) const READING_WIDTH: f32 = 800.;

/// The widest the home page's column runs.
pub(super) const HOME_WIDTH: f32 = 880.;

/// A full page's content area beside its rail: it scrolls, and `column`
/// (made with `page_column`) is centred in it.
pub(super) fn page_scroll(id: impl Into<ElementId>, column: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .child(column)
}

/// The column a full page's records sit in: the reading width, centred,
/// never pinned to the left.
pub(super) fn page_column() -> Div {
    div()
        .v_flex()
        .w_full()
        .max_w(px(READING_WIDTH))
        .mx_auto()
        .p_6()
        .gap_3()
}

/// The head of a page with nothing to list in a rail (Research mandates,
/// Scheduled tasks): back, the page's name as a Title with its `help`, and
/// at its right the page's New button (an outlined icon and word), at the
/// top of the centred column.
pub(super) fn page_head(
    back: Button,
    name: impl Into<SharedString>,
    help_id: &'static str,
    explained: impl Into<SharedString>,
    new: Option<Button>,
    cx: &App,
) -> Div {
    div()
        .h_flex()
        .items_center()
        .gap_2()
        .pb_2()
        .child(back)
        .child(title(name))
        .child(help(help_id, explained, cx))
        .child(div().flex_1())
        .children(new)
}

/// Centres `child` in the conversation's reading column.
pub(super) fn reading_column(child: impl IntoElement) -> Div {
    div()
        .w_full()
        .flex()
        .justify_center()
        .child(div().w_full().max_w(px(READING_WIDTH)).child(child))
}

/// The shape of a bar above the composer (an approval, the line about a
/// review, a hook that failed, a drafted memory, the questions to ask
/// next, the transfers, the composer's own strips): a card kept compact.
/// The caller lays out its row or column.
pub(super) fn bar(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    card(id, cx).flex_shrink_0().px_3().py_2()
}

/// Stacks the bars that sit between the transcript and the composer in the
/// conversation's reading column. The inset is the composer's own
/// (`px_4` in `render_composer`), so their edges are the composer box's at
/// any width.
pub(super) fn above_composer(bars: Vec<AnyElement>) -> Option<Div> {
    (!bars.is_empty()).then(|| {
        reading_column(div().v_flex().gap_2().px_4().pt_2().pb_1().children(bars)).flex_shrink_0()
    })
}

/// The side panel's frame: the page's right margin, as the sidebar is its
/// left one. The margin colour, with a hairline against the conversation.
/// The caller gives it its width; what it holds is laid out by the shapes
/// below, each inset `px_3` from its edges.
pub(super) fn side_panel(cx: &App) -> Div {
    div()
        .v_flex()
        .h_full()
        .bg(cx.theme().sidebar)
        .border_l_1()
        .border_color(cx.theme().sidebar_border)
}

/// One tab in the side panel's strip, which stays on one line. The open
/// one is a white chip with a hairline: its icon and its name in ink. The
/// others are icon buttons in faded ink, their name the tooltip, so Tab
/// still reaches them and Enter or Space opens them; `open` is what a
/// click does.
pub(super) fn panel_tab(
    id: impl Into<ElementId>,
    icon: &str,
    name: impl Into<SharedString>,
    selected: bool,
    open: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    if !selected {
        return icon_action(id, icon, name, IconTone::Faded, cx)
            .on_click(open)
            .into_any_element();
    }
    let theme = cx.theme();
    div()
        .id(id)
        .h_flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .px_2()
        .h(px(ICON_BUTTON))
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .text_color(theme.foreground)
        .child(sized_icon(icon, IconSize::Toolbar, IconTone::Ink, cx))
        .child(div().flex_none().whitespace_nowrap().child(name.into()))
        .into_any_element()
}

/// An icon button for a destructive action (delete, move to trash): the
/// icon in the danger colour, last among its row's actions.
pub(super) fn danger_action(
    id: impl Into<ElementId>,
    icon: &str,
    tooltip: impl Into<SharedString>,
    cx: &App,
) -> Button {
    Button::new(id)
        .ghost()
        .tooltip(tooltip)
        .size(px(ICON_BUTTON))
        .p_0()
        .child(
            crate::assets::icon(icon)
                .size(IconSize::Toolbar.rems())
                .text_color(cx.theme().danger),
        )
}

/// A segmented control: one choice among a few (at most five), side by
/// side in a white box with the fields' hairline and `radius`, a hairline
/// between two segments; the chosen one is a faint teal with teal ink, at
/// the regular weight. Each segment is `segment` (an icon and a word) or
/// `plain_segment` (a word); the caller adds what a click does.
pub(super) fn segmented(children: impl IntoIterator<Item = AnyElement>, cx: &App) -> Div {
    let theme = cx.theme();
    let line = theme.input;
    let mut parts = Vec::new();
    for (index, child) in children.into_iter().enumerate() {
        if index > 0 {
            parts.push(
                div()
                    .flex_none()
                    .w(px(1.))
                    .h(px(26.))
                    .bg(line)
                    .into_any_element(),
            );
        }
        parts.push(child);
    }
    let mut control = div()
        .h_flex()
        .flex_none()
        .items_center()
        .rounded(theme.radius)
        .border_1()
        .border_color(line)
        .bg(theme.popover)
        .overflow_hidden()
        .children(parts);
    // It hugs its segments: in a column it does not stretch across it, so
    // one segment never looks like a field.
    control.style().align_self = Some(AlignSelf::FlexStart);
    control
}

/// Something pressed like a button that is not the component library's
/// button (a segment, a chip, a stepper's end): Tab reaches it, a ring
/// shows the keyboard's focus, and Enter or Space press it, as the
/// buttons it replaces could. It takes a click, a hover and a tooltip like
/// a `div` with an id.
pub(super) fn pressable(id: impl Into<ElementId>) -> Pressable {
    let id = id.into();
    Pressable {
        base: div().id(id.clone()),
        id,
    }
}

/// What `pressable` makes.
#[derive(IntoElement)]
pub(super) struct Pressable {
    id: ElementId,
    base: Stateful<Div>,
}

impl Styled for Pressable {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl InteractiveElement for Pressable {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.base.interactivity()
    }
}

impl StatefulInteractiveElement for Pressable {}

impl ParentElement for Pressable {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for Pressable {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let ring = cx.theme().ring;
        let focused = focus.is_focused(window);
        self.base
            .track_focus(&focus.tab_stop(true))
            // The pointer does not leave the focus ring behind.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .when(focused, |this| this.border_1().border_color(ring))
    }
}

/// The height of a segment, a chip and a stepper.
const CHOICE_HEIGHT: f32 = 26.;

/// The ground and ink of a choice: faint teal and teal while chosen, ink
/// on nothing (and the list hover under the pointer) otherwise.
fn choice_look(base: Pressable, chosen: bool, cx: &App) -> Pressable {
    let theme = cx.theme();
    let hover = theme.list_hover;
    base.h_flex()
        .items_center()
        .flex_none()
        .h(px(CHOICE_HEIGHT))
        .text_sm()
        .when(chosen, |choice| {
            choice
                .bg(theme.primary.opacity(0.12))
                .text_color(theme.primary)
        })
        .when(!chosen, |choice| {
            choice
                .text_color(theme.foreground)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
        })
}

/// One segment of `segmented`: an icon and a word.
pub(super) fn segment(
    id: impl Into<ElementId>,
    icon: &str,
    label: impl Into<SharedString>,
    chosen: bool,
    cx: &App,
) -> Pressable {
    let tone = if chosen {
        IconTone::On
    } else {
        IconTone::Faded
    };
    choice_look(pressable(id), chosen, cx)
        .group(ICON_HOVER)
        .gap_1p5()
        .px_2()
        .child(sized_icon(icon, IconSize::Inline, tone, cx))
        .child(div().max_w(px(140.)).truncate().child(label.into()))
}

/// A chip of a group of choices that wrap (a palette, a tool to install,
/// a capability): white with the fields' hairline and `radius`; a chosen
/// one has a teal line and teal ink. The caller puts in what it shows (a
/// colour's dot and its name, a word) and what a click does.
pub(super) fn choice_chip(id: impl Into<ElementId>, chosen: bool, cx: &App) -> Pressable {
    let theme = cx.theme();
    let (line, teal, paper, hover) = (theme.input, theme.primary, theme.popover, theme.list_hover);
    pressable(id)
        .h_flex()
        .items_center()
        .flex_none()
        .gap_1p5()
        .px_2()
        .h(px(CHOICE_HEIGHT))
        .rounded(theme.radius)
        .border_1()
        .bg(paper)
        .text_sm()
        .when(chosen, |chip| chip.border_color(teal).text_color(teal))
        .when(!chosen, |chip| {
            chip.border_color(line)
                .text_color(theme.foreground)
                .cursor_pointer()
                .hover(move |style| style.bg(hover))
        })
}

/// An icon and a word on an outlined button (white, a hairline): the one
/// action on an item that is used most (Check a host, Test a model's
/// connection). The item's other actions are `icon_action`s after it, a
/// destructive one last (`danger_action`).
pub(super) fn outlined_action(
    id: impl Into<ElementId>,
    icon: &str,
    word: impl Into<SharedString>,
    cx: &App,
) -> Button {
    Button::new(id)
        .outline()
        .small()
        .bg(cx.theme().popover)
        .child(
            div()
                .h_flex()
                .items_center()
                .gap_1p5()
                .child(sized_icon(icon, IconSize::Inline, IconTone::Ink, cx))
                .child(div().flex_none().child(word.into())),
        )
}

/// A heading inside a settings page, over a group of its rows: caption in
/// faded ink, not bold.
pub(super) fn settings_heading(text: impl Into<SharedString>, cx: &App) -> Div {
    heading_box(cx).child(text.into())
}

/// The box of a heading in Settings: faded caption with room above.
fn heading_box(cx: &App) -> Div {
    div()
        .flex_shrink_0()
        .pt_5()
        .pb_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
}

/// The longest a note in Settings runs, in characters (a Chinese one
/// counts as one): one short line. What a user needs beyond it goes into a
/// `help` icon's tooltip beside the name or the heading.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) const NOTE_LIMIT: usize = 64;

/// A small `circle-help` icon in faded ink beside a setting's name or a
/// heading, its tooltip the whole explanation that does not fit in a note.
pub(super) fn help(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    let text: SharedString = text.into();
    div()
        .id(id)
        .group(ICON_HOVER)
        .flex_none()
        .h_flex()
        .items_center()
        .tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
        .child(sized_icon(
            "circle-help",
            IconSize::Inline,
            IconTone::Faded,
            cx,
        ))
}

/// `settings_heading` with a `help` icon after its words.
pub(super) fn settings_heading_help(
    text: impl Into<SharedString>,
    id: impl Into<ElementId>,
    explained: impl Into<SharedString>,
    cx: &App,
) -> Div {
    heading_box(cx)
        .h_flex()
        .items_center()
        .gap_1()
        .child(text.into())
        .child(help(id, explained, cx))
}

/// A sentence in a settings page that is not a setting's note (what a page
/// or a group is for, what a list holds): body text cut to caption in
/// faded ink.
pub(super) fn settings_note(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// `settings_note` whose one short line is followed by a `help` icon
/// carrying the whole explanation.
pub(super) fn settings_note_help(
    text: &'static str,
    explained: impl Into<SharedString>,
    cx: &App,
) -> Div {
    settings_note(text, cx)
        .h_flex()
        .items_center()
        .gap_1()
        .child(help(
            SharedString::from(format!("help-{text}")),
            explained,
            cx,
        ))
}

/// A description a list's row carries from its data (a connector's, a
/// skill's, a plugin's): one faded line, cut short, the whole of it in the
/// tooltip, so a list does not become a wall of faded text.
pub(super) fn one_line_note(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    cx: &App,
) -> Stateful<Div> {
    let text: SharedString = text.into();
    let whole = text.clone();
    div()
        .id(id)
        .min_w_0()
        .truncate()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .tooltip(move |window, cx| Tooltip::new(whole.clone()).build(window, cx))
        .child(text)
}

/// A value that cannot be changed here (a model from `WISP_*` variables, a
/// tool's detected path): a fact in ink on the paper, not a grey field.
pub(super) fn read_only(value: impl Into<SharedString>, cx: &App) -> Div {
    fact(value, cx)
        .text_color(cx.theme().foreground)
        .min_w_0()
        .truncate()
}

/// A text field: the component library's input on white with the fields'
/// line (`input.border`) and `radius`, never the grey of a disabled one. A
/// value that cannot be changed is not a field: it is a fact on the paper
/// (`read_only`).
pub(super) fn input(
    state: &Entity<gpui_component::input::InputState>,
    cx: &App,
) -> gpui_component::input::Input {
    gpui_component::input::Input::new(state)
        .bg(cx.theme().popover)
        .border_color(cx.theme().input)
        .rounded(cx.theme().radius)
}

/// The widest the settings' one column runs; every page is centred in it.
pub(super) const SETTINGS_WIDTH: f32 = 680.;

/// A settings page: its name as a Title at the top of one centred column
/// on the paper, then the page.
pub(super) fn settings_column(name: impl Into<SharedString>, page: impl IntoElement) -> Div {
    div().w_full().flex().justify_center().child(
        div()
            .v_flex()
            .w_full()
            .max_w(px(SETTINGS_WIDTH))
            .px_6()
            .pt_6()
            .pb_10()
            .gap_3()
            .child(title(name))
            .child(page),
    )
}

/// A stepper: smaller | the value as a fact | larger, one segmented box,
/// and a reset icon button after it. The caller gives the three handlers.
pub(super) fn stepper(
    id: &'static str,
    value: impl Into<SharedString>,
    smaller: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    larger: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    reset: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let tr = crate::i18n::tr;
    let end = |part: &'static str,
               icon: &'static str,
               tooltip: &'static str,
               run: Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>| {
        choice_look(
            pressable(SharedString::from(format!("{id}-{part}"))),
            false,
            cx,
        )
        .group(ICON_HOVER)
        .justify_center()
        .w(px(ICON_BUTTON))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .on_click(move |event, window, cx| run(event, window, cx))
        .child(sized_icon(icon, IconSize::Inline, IconTone::Faded, cx))
    };
    div()
        .h_flex()
        .flex_none()
        .items_center()
        .gap_1()
        .child(segmented(
            [
                end("smaller", "type-decrease", tr("Smaller"), Box::new(smaller))
                    .into_any_element(),
                div()
                    .h_flex()
                    .items_center()
                    .justify_center()
                    .min_w(px(64.))
                    .px_2()
                    .h(px(CHOICE_HEIGHT))
                    .child(fact(value, cx).text_color(theme.foreground))
                    .into_any_element(),
                end("larger", "type-increase", tr("Larger"), Box::new(larger)).into_any_element(),
            ],
            cx,
        ))
        .child(
            icon_action(
                SharedString::from(format!("{id}-reset")),
                "undo",
                crate::i18n::tr("Back to Wisp's own size"),
                IconTone::Faded,
                cx,
            )
            .on_click(reset),
        )
}

/// A segment of `segmented` that is a word only, for choices with no
/// icon of their own (languages, keys).
pub(super) fn plain_segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    chosen: bool,
    cx: &App,
) -> Pressable {
    choice_look(pressable(id), chosen, cx)
        .px_2p5()
        .child(label.into())
}

/// A field on the page: white, with the fields' line and `radius`; the
/// caller puts the input (without its own look) inside.
pub(super) fn field(cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .h_flex()
        .items_center()
        .gap_1()
        .px_2()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.input)
        .bg(theme.popover)
}

/// The row at the top of a side-panel tab, or over a group in it: `title`
/// as body text in ink, a faded `detail` after it (a count, a size),
/// then what the caller adds (its actions, as icon buttons) at the right
/// end, and a hairline under it.
pub(super) fn panel_header(
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    cx: &App,
) -> Div {
    panel_header_led(None, title, detail, cx)
}

/// A panel header with a button before its title: Back, in a file's
/// preview.
pub(super) fn panel_header_led(
    lead: Option<AnyElement>,
    title: impl Into<SharedString>,
    detail: Option<SharedString>,
    cx: &App,
) -> Div {
    div()
        .h_flex()
        .items_center()
        .gap_1()
        .px_3()
        .py_1()
        .min_h(px(PANEL_HEADER_HEIGHT))
        .flex_shrink_0()
        .border_b_1()
        .border_color(cx.theme().border)
        .children(lead)
        .child(
            div()
                .h_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .child(div().min_w_0().truncate().child(title.into()))
                .children(detail.map(|detail| {
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail)
                })),
        )
}

/// How high a side-panel tab's header row is at least.
pub(super) const PANEL_HEADER_HEIGHT: f32 = 40.;

/// How high a row of a list in the side panel is at least.
pub(super) const PANEL_ROW_HEIGHT: f32 = 32.;

/// One line of a list in the side panel: the panel's inset, at least
/// `PANEL_ROW_HEIGHT` high, a hairline under it. The caller adds the
/// leading label (`panel_label`), the metadata in caption, and, when the
/// row opens something, the theme's list hover.
pub(super) fn panel_row(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    div()
        .id(id)
        .h_flex()
        .items_center()
        .flex_shrink_0()
        .gap_2()
        .px_3()
        .min_h(px(PANEL_ROW_HEIGHT))
        .border_b_1()
        .border_color(cx.theme().border)
}

/// An entry of several lines in a side-panel list (a run, a host, a
/// delegated batch, a cell, a tool call): the inset and the hairline of a
/// row, with its lines stacked.
pub(super) fn panel_entry(cx: &App) -> Div {
    div()
        .v_flex()
        .flex_shrink_0()
        .gap_1()
        .px_3()
        .py_2()
        .min_h(px(PANEL_ROW_HEIGHT))
        .border_b_1()
        .border_color(cx.theme().border)
}

/// The label that leads a row: body text in ink, cut short on one line.
pub(super) fn panel_label(text: impl Into<SharedString>) -> Div {
    div().flex_1().min_w_0().truncate().child(text.into())
}

/// A card in the side panel: `card`, padded and stacked. One that sits
/// straight in a tab's list takes the panel's inset with `mx_3`.
pub(super) fn panel_card(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    card(id, cx).v_flex().gap_1().p_2()
}

/// A heading over a group inside a tab: caption in faded ink. A heading
/// straight in a tab's list takes the panel's inset with `px_3`.
pub(super) fn panel_section(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_shrink_0()
        .pt_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// The sentence under a tab's header that says what the tab is for:
/// caption in faded ink, at the panel's inset.
pub(super) fn panel_note(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .flex_shrink_0()
        .px_3()
        .pt_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// What a tab shows while it has nothing to list, or is still reading it:
/// one sentence in faded ink at the panel's inset. A tab that offers one
/// way on adds it as the one child, an outlined button.
pub(super) fn panel_empty(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .v_flex()
        .items_start()
        .flex_shrink_0()
        .gap_2()
        .px_3()
        .py_3()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// The items of a grid `columns` wide, row by row; the last row may be
/// short.
pub(super) fn rows_of<T>(items: Vec<T>, columns: usize) -> Vec<Vec<T>> {
    let columns = columns.max(1);
    let mut rows: Vec<Vec<T>> = Vec::new();
    for item in items {
        match rows.last_mut() {
            Some(row) if row.len() < columns => row.push(item),
            _ => rows.push(vec![item]),
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{
        DISPLAY_WEIGHT, DialogWidth, ICON_BUTTON, IconSize, OWN_TITLE_BAR, READING_WIDTH,
        RECORD_ROW_HEIGHT, SERIF_FALLBACK_SERIFS, SERIF_FALLBACKS, StampKind, TITLE_BAR_HEIGHT,
        TITLE_WEIGHT, TRAFFIC_LIGHTS, TRAFFIC_LIGHTS_INSET, dialog, record_row, rows_of,
        stamp_shade, switch, window_titlebar,
    };

    /// Every `.rs` file under `src`, with its path.
    fn sources() -> Vec<(std::path::PathBuf, String)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut folders = vec![root];
        let mut found = Vec::new();
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(folder).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                } else if path.extension().is_some_and(|kind| kind == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap();
                    found.push((path, text));
                }
            }
        }
        found
    }

    /// What each `.label(` call in `source` is given, up to its closing
    /// bracket, with string and character literals kept whole.
    fn labels(source: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find(".label(") {
            rest = &rest[at + ".label(".len()..];
            let chars: Vec<(usize, char)> = rest.char_indices().collect();
            let mut depth = 1;
            let mut quoted = false;
            let mut end = rest.len();
            let mut index = 0;
            while index < chars.len() {
                let ch = chars[index].1;
                let next = |by: usize| chars.get(index + by).map(|(_, ch)| *ch);
                match ch {
                    '\\' if quoted => index += 1,
                    '"' => quoted = !quoted,
                    // A character literal such as '"' or ')', not a lifetime.
                    '\'' if !quoted && next(2) == Some('\'') => index += 2,
                    '\'' if !quoted && next(1) == Some('\\') && next(3) == Some('\'') => index += 3,
                    '(' if !quoted => depth += 1,
                    ')' if !quoted => {
                        depth -= 1;
                        if depth == 0 {
                            end = chars[index].0;
                            break;
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            found.push(rest[..end].to_owned());
        }
        found
    }

    #[test]
    fn the_scanner_finds_what_each_label_is_given() {
        let source = "Button::new(\"a\").label(if on { tr(\"On\") } else { tr(\"Off\") })\
                      .small().label(tr(\"Say (it)\")).label(name.clone())\
                      .label(text.trim_end_matches(')').replace('\"', \"\"))";
        assert_eq!(
            labels(source),
            [
                "if on { tr(\"On\") } else { tr(\"Off\") }",
                "tr(\"Say (it)\")",
                "name.clone()",
                "text.trim_end_matches(')').replace('\"', \"\")",
            ]
        );
    }

    #[test]
    fn on_off_settings_are_switches_not_buttons_that_say_on_or_off() {
        // A boolean a user flips is `switch`; a button labelled On or Off,
        // or with a state after a colon ("At start: on"), is the control the
        // design language rules out.
        let says_on_or_off = |label: &str| {
            ["\"On\"", "\"Off\"", ": on\"", ": off\""]
                .iter()
                .any(|word| label.contains(word))
        };
        assert!(says_on_or_off("if on { tr(\"On\") } else { tr(\"Off\") }"));
        assert!(says_on_or_off("tr(\"At start: off\")"));
        assert!(!says_on_or_off("tr(\"Turn on\")"));
        let mut found = Vec::new();
        for (path, text) in sources() {
            // This file's own tests write such a label to check the scanner.
            if path.ends_with("surfaces.rs") {
                continue;
            }
            for label in labels(&text) {
                if says_on_or_off(&label) {
                    found.push(format!("{}: .label({label})", path.display()));
                }
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn a_switch_keeps_its_value_and_is_live_until_disabled() {
        let live = switch("s", true, "Notify me");
        assert!(live.on);
        assert!(!live.disabled);
        assert_eq!(live.what.as_ref(), "Notify me");
        assert!(live.on_change.is_none());
        let shown = switch("s", false, "Notify me")
            .disabled(true)
            .on_change(|_, _, _| {});
        assert!(!shown.on);
        assert!(shown.disabled);
        assert!(shown.on_change.is_some());
    }

    #[test]
    fn a_grid_fills_row_by_row_and_leaves_the_last_row_short() {
        assert_eq!(
            rows_of(vec![1, 2, 3, 4, 5], 2),
            vec![vec![1, 2], vec![3, 4], vec![5]]
        );
        assert_eq!(rows_of(Vec::<u8>::new(), 2), Vec::<Vec<u8>>::new());
        // No columns is one column, not a loop that never ends.
        assert_eq!(rows_of(vec![1, 2], 0), vec![vec![1], vec![2]]);
    }

    /// The text of method `name` in `source`, from its signature to the
    /// brace that closes it at the `impl`'s indent.
    fn method<'a>(source: &'a str, name: &str) -> Option<&'a str> {
        let start = source.find(&format!("fn {name}("))?;
        let rest = &source[start..];
        Some(&rest[..rest.find("\n    }\n")?])
    }

    #[test]
    fn the_bars_above_the_composer_are_cards_in_its_column() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        // Each is drawn as `bar`, with no margin, fill, tint or shadow of
        // its own that would take it out of the column or the family.
        for (name, function) in [
            ("workspace/shell.rs", "render_approvals"),
            ("workspace/shell.rs", "plan_approval_card"),
            ("workspace/review_view.rs", "render_review_banner"),
            (
                "workspace/stream_recovery_view.rs",
                "render_stream_recovery",
            ),
            ("workspace/settings_hooks.rs", "render_hook_failures"),
            ("workspace/memory_view.rs", "render_memory_proposal"),
            ("workspace/follow_ups_view.rs", "render_follow_ups"),
            ("workspace/transfers_view.rs", "render_transfers"),
            ("workspace/queue_view.rs", "queue_strip"),
            ("workspace/runtime_console_view.rs", "render_console"),
            ("workspace/slash_view.rs", "timer_strip"),
            ("workspace/shell.rs", "read_only_card"),
            ("workspace/reuse_view.rs", "render_reuse_range"),
            (
                "workspace/browser_prompts_view.rs",
                "render_browser_extension_bar",
            ),
        ] {
            let body = method(file(name), function)
                .unwrap_or_else(|| panic!("{name} has no fn {function}"));
            assert!(body.contains("surfaces::bar("), "{function} is not a bar");
            for other in [
                ".mx_4()",
                ".mb_2()",
                ".shadow",
                ".bg(attention",
                ".border_color(attention",
                ".bg(cx.theme().secondary)",
            ] {
                assert!(!body.contains(other), "{function} uses {other}");
            }
        }
        // Those between the transcript and the composer are stacked in the
        // composer's column, not added to the page one by one.
        let conversation = method(file("workspace/shell.rs"), "render_conversation").unwrap();
        let from = conversation
            .find("let bars")
            .expect("the bars are gathered");
        let to = conversation
            .find("surfaces::above_composer(")
            .expect("and stacked above the composer");
        let gathered = &conversation[from..to];
        for function in [
            "render_approvals",
            "render_review_banner",
            "render_stream_recovery",
            "render_hook_failures",
            "render_memory_proposal",
            "render_follow_ups",
            "render_transfers",
            "render_reuse_range",
            "render_browser_extension_bar",
        ] {
            assert!(
                gathered.contains(&format!("self.{function}(")),
                "{function}"
            );
            assert_eq!(
                conversation.matches(&format!("self.{function}(")).count(),
                1
            );
        }
    }

    #[test]
    fn the_banners_above_the_transcript_keep_its_column() {
        // The error and the notice line up with the transcript and the bars
        // above the composer: the reading column, with the composer's inset.
        let files = sources();
        let (_, shell) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/shell.rs"))
            .expect("shell.rs");
        let conversation: String = method(shell, "render_conversation")
            .unwrap()
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect();
        for banner in ["error_banner(error,cx)", "self.notice_banner(notice,cx)"] {
            assert!(
                conversation.contains(&format!(
                    "surfaces::reading_column(div().px_4().pt_2().child({banner})"
                )),
                "{banner}"
            );
        }
    }

    #[test]
    fn a_dialog_comes_in_three_widths_and_one_for_two_panes() {
        let widths = DialogWidth::ALL.map(DialogWidth::px);
        assert_eq!(widths, [520., 640., 760.]);
        // Narrowest first, and the widest still inside a conversation's
        // reading column.
        assert!(widths.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(widths[2] <= READING_WIDTH);
        // Two panes take more than a column of reading: each keeps room of
        // its own beside the other.
        assert_eq!(DialogWidth::TwoPanes.px(), 1000.);
        assert!(DialogWidth::TwoPanes.px() > READING_WIDTH);
    }

    #[test]
    fn a_dialog_closes_on_a_click_outside_only_when_told_to() {
        let plain = dialog("d", DialogWidth::Narrow, "Title");
        assert_eq!(plain.id.as_ref(), "d");
        assert_eq!(plain.title.as_ref(), "Title");
        assert!(plain.on_outside.is_none());
        assert!(plain.close.is_none());
        assert!(plain.height.is_none());
        assert!(plain.aside.is_empty() && plain.actions.is_empty());
        let full = dialog("d", DialogWidth::Wide, "Title")
            .close_on_click_outside(|_, _, _| {})
            .close_button("d-close", |_, _, _| {})
            .height(600.)
            .aside(gpui::div())
            .action(gpui::div())
            .actions([gpui::div(), gpui::div()]);
        assert!(full.on_outside.is_some());
        assert!(full.close.is_some());
        assert_eq!(full.height, Some(600.));
        assert_eq!(full.aside.len(), 1);
        // In the order given: the main action is the one added last.
        assert_eq!(full.actions.len(), 3);
    }

    /// Code outside a file's test module, where a scan's own words are not
    /// taken for a use.
    fn code(source: &str) -> &str {
        source
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or_default()
    }

    /// The name of the function at a file's or an `impl`'s indent that
    /// `at` falls in; a function nested in another is not counted.
    fn function_at(source: &str, at: usize) -> Option<&str> {
        source[..at].lines().rev().find_map(|line| {
            let indent = line.len() - line.trim_start().len();
            let rest = line.trim_start();
            let rest = ["pub(super) ", "pub(crate) ", "pub "]
                .iter()
                .find_map(|vis| rest.strip_prefix(vis))
                .unwrap_or(rest);
            let name = rest.strip_prefix("fn ")?;
            (indent <= 4).then(|| name.split(['(', '<']).next().unwrap_or(name))
        })
    }

    #[test]
    fn the_scanner_names_the_function_a_call_is_in() {
        let source = "fn popup() {\n    a.shadow_md()\n}\nimpl X {\n    pub(super) fn menu(&self) {\n        fn action() {}\n        b.shadow_md()\n    }\n}\n";
        let first = source.find(".shadow").unwrap();
        let second = source.rfind(".shadow").unwrap();
        assert_eq!(function_at(source, first), Some("popup"));
        assert_eq!(function_at(source, second), Some("menu"));
    }

    #[test]
    fn only_dialogs_and_menus_cast_a_shadow() {
        // What sits on the page is set off by a hairline. What floats above
        // it casts a shadow: the dialog, the menus and pickers anchored to a
        // control, the palette and the cards for other projects'
        // conversations in the window's corner.
        let floating = [
            ("workspace/surfaces.rs", "dialog_card"),
            ("workspace/shell.rs", "composer_menu"),
            ("workspace/shell.rs", "outline_menu"),
            ("workspace/conversation_actions.rs", "conversation_menu"),
            ("workspace/panel_tabs.rs", "panel_tab_menu"),
            ("workspace/centre_view.rs", "centre_tab_menu"),
            ("workspace/organise_view.rs", "popup"),
            ("workspace/demos_view.rs", "example_copy_menu"),
            ("workspace/terminal_view.rs", "terminal_hosts_menu"),
            ("workspace/palette_view.rs", "render_palette"),
            ("workspace/notices_view.rs", "render_session_notices"),
            ("workspace/home_view.rs", "start_menu"),
            ("workspace/shell.rs", "message_menu"),
            ("workspace/files_panel.rs", "files_menu"),
            ("workspace/files_panel.rs", "file_preview_menu"),
            ("workspace/tabs_view.rs", "waiting_list"),
        ];
        let mut found = Vec::new();
        for (path, text) in sources() {
            let code = code(&text);
            for (at, _) in code.match_indices(".shadow") {
                let function = function_at(code, at).unwrap_or_default();
                if !floating
                    .iter()
                    .any(|(file, name)| path.ends_with(file) && *name == function)
                {
                    found.push(format!("{} in fn {function}", path.display()));
                }
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn dialogs_are_drawn_from_one_shape() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        // Each is `dialog`, with no backdrop, card, shadow or title size of
        // its own.
        for (name, function) in [
            ("workspace/home_view.rs", "render_new_project"),
            ("workspace/privacy_view.rs", "render_privacy"),
            ("workspace/run_review_view.rs", "render_run_review"),
            ("workspace/webview_import_view.rs", "render_webview_import"),
            ("workspace/demos_view.rs", "render_examples"),
            ("workspace/onboarding_view.rs", "render_onboarding"),
            ("workspace/import_view.rs", "render_imports"),
            ("workspace/merge_view.rs", "render_branch_merge"),
            ("workspace/model_view_dialog.rs", "render_model_view"),
            ("workspace/capabilities_view.rs", "render_capabilities"),
            ("workspace/review_view.rs", "render_review"),
            ("workspace/compact_view.rs", "render_compact_dialog"),
            ("workspace/archive_view.rs", "render_research_archive"),
            ("workspace/archive_view.rs", "render_discard_archive"),
            ("workspace/explore_view.rs", "render_exploration_preview"),
            ("workspace/explore_view.rs", "render_start_exploration"),
            ("workspace/project_settings.rs", "render_project_settings"),
            ("workspace/project_sync_view.rs", "render_join_synced"),
            ("workspace/trajectory_view.rs", "render_trajectory"),
            ("workspace/file_editor_view.rs", "render_file_changed"),
            ("workspace/file_editor_view.rs", "render_close_unsaved"),
            ("workspace/memory_view.rs", "render_clear_memory"),
            ("workspace/hosts_view.rs", "render_remove_staged"),
            ("workspace/share_view.rs", "render_share"),
            ("workspace/reuse_view.rs", "render_session_reuse"),
            (
                "workspace/browser_prompts_view.rs",
                "render_browser_cleanup",
            ),
            (
                "workspace/browser_prompts_view.rs",
                "render_browser_needs_human",
            ),
        ] {
            let body = method(file(name), function)
                .unwrap_or_else(|| panic!("{name} has no fn {function}"));
            assert!(
                body.contains("surfaces::dialog("),
                "{function} is not a dialog"
            );
            for other in [".occlude()", ".shadow", "theme().overlay", "radius_lg"] {
                assert!(!body.contains(other), "{function} uses {other}");
            }
            // The width for two panes is the examples' alone; a dialog of
            // one column takes one of the three.
            assert_eq!(
                body.contains("DialogWidth::TwoPanes"),
                function == "render_examples",
                "{function}"
            );
        }
        // Nothing else dims the window: the backdrop is the dialog's, and
        // the palette's, which hangs from the top of the window.
        let mut found = Vec::new();
        for (path, text) in &files {
            let code = code(text);
            for (at, _) in code.match_indices("theme().overlay") {
                let function = function_at(code, at).unwrap_or_default();
                let allowed = (path.ends_with("workspace/surfaces.rs") && function == "backdrop")
                    || (path.ends_with("workspace/palette_view.rs")
                        && function == "render_palette");
                if !allowed {
                    found.push(format!("{} in fn {function}", path.display()));
                }
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }

    /// The text of free function `name` in `source`, from its signature to
    /// the brace that closes it at the file's indent.
    fn free_function<'a>(source: &'a str, name: &str) -> Option<&'a str> {
        let start = source.find(&format!("fn {name}("))?;
        let rest = &source[start..];
        Some(&rest[..rest.find("\n}\n")?])
    }

    #[test]
    fn a_side_panel_row_is_as_high_as_a_row_on_the_page() {
        // A row in a list is 32 px or more, and the header over a tab
        // stands taller than the rows under it.
        assert!(super::PANEL_ROW_HEIGHT >= 32.);
        assert!(super::PANEL_HEADER_HEIGHT > super::PANEL_ROW_HEIGHT);
    }

    #[test]
    fn the_side_panel_is_the_pages_right_margin() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let surfaces = file("workspace/surfaces.rs");
        // The margin colour and the hairline the sidebar has, on the other
        // side of the page.
        let frame = free_function(surfaces, "side_panel").unwrap();
        assert!(frame.contains(".bg(cx.theme().sidebar)"));
        assert!(frame.contains(".border_l_1()"));
        assert!(frame.contains(".border_color(cx.theme().sidebar_border)"));
        // The panel takes that frame and draws none of its own.
        let panel = method(file("workspace/files_panel.rs"), "render_files").unwrap();
        assert!(panel.contains("surfaces::side_panel(cx)"));
        for own in [".border_l_1()", ".bg(", "theme().sidebar"] {
            assert!(!panel.contains(own), "render_files uses {own}");
        }
        // The strip is one line of icons through one shape: the open tab a
        // chip of its icon and name, the others icon buttons whose tooltip
        // is their name; what has no room goes into More, which also takes
        // the open tab away and adds one back.
        let strip = method(file("workspace/panel_tabs.rs"), "panel_tabs").unwrap();
        assert!(strip.contains("surfaces::panel_tab("));
        assert!(strip.contains("tab.icon()"));
        assert!(strip.contains("panel::strip("));
        assert!(!strip.contains("flex_wrap"));
        assert!(!strip.contains("icon_button("));
        let menu = method(file("workspace/panel_tabs.rs"), "panel_tab_menu").unwrap();
        assert!(menu.contains("this.take_away_panel_tab("));
        assert!(menu.contains("missing"));
        let tab = free_function(surfaces, "panel_tab").unwrap();
        // An unchosen tab is an icon button named by its tooltip.
        assert!(tab.contains("icon_action(id, icon, name, IconTone::Faded, cx)"));
        // The chosen one is white with a hairline, icon and name in ink.
        assert!(tab.contains(".bg(theme.popover)"));
        assert!(tab.contains(".border_color(theme.border)"));
        assert!(tab.contains("IconTone::Ink"));
        assert!(!tab.contains("theme().primary"));
    }

    #[test]
    fn every_side_panel_tab_takes_the_margins_shapes() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        // Where each tab is drawn, one per tab.
        let tabs = [
            ("workspace/files_panel.rs", "render_files"),
            ("workspace/panel_tabs.rs", "render_runs_tab"),
            ("workspace/hosts_view.rs", "render_hosts_tab"),
            ("workspace/panel_tabs.rs", "render_artifacts_tab"),
            ("workspace/notebook_view.rs", "render_notebook_tab"),
            ("workspace/files_panel.rs", "render_provenance_tab"),
            ("workspace/agents_view.rs", "render_agents_tab"),
            ("workspace/favorites_view.rs", "render_highlights_tab"),
            ("workspace/memos_view.rs", "render_memos_tab"),
            ("workspace/memory_view.rs", "render_memory_tab"),
            ("workspace/aside_view.rs", "render_aside_tab"),
        ];
        assert_eq!(tabs.len(), crate::panel::PanelTab::ALL.len());
        // The tabs with actions over the whole list, or groups that have
        // their own, have header rows; the rest open on their list.
        let headed = [
            "render_files",
            "render_artifacts_tab",
            "render_highlights_tab",
            "render_memos_tab",
            "render_memory_tab",
        ];
        for (name, function) in tabs {
            let body = method(file(name), function)
                .unwrap_or_else(|| panic!("{name} has no fn {function}"));
            // Nothing to list is said one way.
            assert!(body.contains("surfaces::panel_empty("), "{function}");
            assert_eq!(
                body.contains("surfaces::panel_header("),
                headed.contains(&function),
                "{function}"
            );
            // No empty line, bold heading, hand-drawn card or shadow of
            // the tab's own. Provenance shows what a call was given and
            // returned as code in a well, as the conversation shows code.
            let own = [
                ".p_3().text_sm()",
                "font_semibold",
                ".shadow",
                ".border_1()",
            ];
            for own in own {
                assert!(!body.contains(own), "{function} uses {own}");
            }
            if function != "render_provenance_tab" {
                for own in [".bg(cx.theme().muted)", ".rounded(cx.theme().radius)"] {
                    assert!(!body.contains(own), "{function} uses {own}");
                }
            }
        }
        // The parts the tabs are made of come from the same helpers.
        let cards = [
            ("workspace/panel_tabs.rs", "clean_question"),
            ("workspace/memos_view.rs", "memo_card"),
            ("workspace/memory_view.rs", "memory_card"),
            ("workspace/memory_view.rs", "tidy_card"),
            ("workspace/hosts_view.rs", "host_storage_form"),
            ("workspace/hosts_view.rs", "host_detail_card"),
        ];
        let entries = [
            ("workspace/hosts_view.rs", "host_card"),
            ("workspace/agents_view.rs", "render_batch"),
            ("workspace/agents_view.rs", "render_swarm"),
            ("workspace/agents_view.rs", "render_agents_tab"),
            ("workspace/notebook_view.rs", "render_notebook_tab"),
            ("workspace/aside_view.rs", "render_aside_tab"),
        ];
        let rows = [("workspace/remote_files_view.rs", "render_remote_files")];
        // The lists of things are record rows: files, runs, artifacts, the
        // provenance of tool calls.
        let records = [
            ("workspace/files_panel.rs", "render_provenance_tab"),
            ("workspace/panel_tabs.rs", "render_artifacts_tab"),
            ("workspace/run_cards_view.rs", "run_record_row"),
            ("workspace/files_panel.rs", "file_row"),
            ("workspace/files_panel.rs", "file_hit_row"),
        ];
        let headers = [
            ("workspace/remote_files_view.rs", "render_remote_files"),
            ("workspace/remote_files_view.rs", "render_remote_look"),
            ("workspace/memos_view.rs", "memo_editor"),
            ("workspace/memory_view.rs", "memory_editor"),
        ];
        let sections = [
            ("workspace/panel_tabs.rs", "render_method_search"),
            ("workspace/panel_tabs.rs", "render_artifacts_tab"),
            ("workspace/agents_view.rs", "render_agents_tab"),
            ("workspace/aside_view.rs", "render_aside_tab"),
        ];
        for (shape, drawn) in [
            ("surfaces::record_row(", &records[..]),
            ("surfaces::panel_card(", &cards[..]),
            ("surfaces::panel_entry(", &entries[..]),
            ("surfaces::panel_row(", &rows[..]),
            ("surfaces::panel_header(", &headers[..]),
            ("surfaces::panel_section(", &sections[..]),
        ] {
            for (name, function) in drawn {
                let body = method(file(name), function)
                    .unwrap_or_else(|| panic!("{name} has no fn {function}"));
                assert!(body.contains(shape), "{function} has no {shape}");
                for own in ["font_semibold", ".border_1()", ".shadow"] {
                    assert!(!body.contains(own), "{function} uses {own}");
                }
            }
        }
        // The Runs tab and the conversation draw a run as the same row.
        let runs = method(file("workspace/panel_tabs.rs"), "render_runs_tab").unwrap();
        assert!(runs.contains("self.run_record_row("));
        assert!(runs.contains("surfaces::record_group("));
        assert!(!runs.contains("chip("));
        let shell = method(file("workspace/shell.rs"), "run_record").unwrap();
        assert!(shell.contains("self.run_record_row("));
        // A list in a uniform list keeps every row the panel's height.
        for function in ["file_row", "file_hit_row"] {
            let body = method(file("workspace/files_panel.rs"), function).unwrap();
            assert!(
                body.contains(".h(px(surfaces::PANEL_ROW_HEIGHT))"),
                "{function}"
            );
        }
    }

    #[test]
    fn a_document_in_the_centre_is_the_files_tabs_preview_beside_a_reading_column() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let panel = file("workspace/files_panel.rs");
        let centre = file("workspace/centre_view.rs");
        // One preview, drawn in either place from the same parts: the Files
        // tab and the centre both take `preview_parts`, which holds the
        // header, the editor's and the picture's bars and the preview.
        let parts = method(panel, "preview_parts").unwrap();
        for part in [
            "surfaces::panel_header_led(",
            "self.render_file_editor_bar(cx)",
            "self.render_picture_bar(cx)",
            "self.render_preview(window, cx)",
        ] {
            assert!(parts.contains(part), "preview_parts has no {part}");
        }
        let side = method(panel, "render_files").unwrap();
        assert_eq!(side.matches("self.preview_parts(").count(), 1);
        let document = method(centre, "render_centre_document").unwrap();
        assert!(document.contains("self.preview_parts("));
        // The centre draws no preview of its own.
        for own in [
            "TextView::",
            "Input::new(",
            "img(",
            "self.render_preview(",
            "self.render_picture(",
            "self.render_file_editor(",
        ] {
            assert!(!centre.contains(own), "centre_view.rs uses {own}");
        }
        // The conversation keeps a column to read in beside a document, and
        // the centre keeps room for both, so the side panel gives way first.
        let split = method(centre, "render_centre").unwrap();
        assert!(split.contains(".min_w(px(centre::CHAT_MIN))"));
        assert!(split.contains("page.min_w(px(centre::SPLIT_MIN))"));
        // The conversation's own rows keep the reading column in it.
        let conversation = method(file("workspace/shell.rs"), "render_conversation").unwrap();
        assert!(conversation.contains("self.render_centre(page, content, window, cx)"));
        assert!(conversation.contains("surfaces::reading_column(row)"));
    }

    #[test]
    fn titles_are_the_bundled_serif_in_regular_then_a_chinese_serif_then_the_ui_face() {
        // The bundled face leads, so the Latin is the same on every system.
        assert_eq!(super::SERIF, crate::assets::SERIF_FACE);
        // Titles are regular; the one display line is Medium, never
        // heavier. Both weights are bundled.
        assert_eq!(TITLE_WEIGHT, gpui::FontWeight::NORMAL);
        assert_eq!(DISPLAY_WEIGHT, gpui::FontWeight::MEDIUM);
        assert_eq!(
            super::serif_at(DISPLAY_WEIGHT).weight,
            gpui::FontWeight::MEDIUM
        );
        let face = super::serif();
        assert_eq!(face.family.as_ref(), crate::assets::SERIF_FACE);
        assert_eq!(face.weight, gpui::FontWeight::NORMAL);
        let fallbacks = face.fallbacks.expect("titles have fallbacks");
        assert_eq!(fallbacks.fallback_list(), SERIF_FALLBACKS.map(String::from));
        // A Chinese serif on each system first: macOS ships Songti SC,
        // Linux has a Noto or Source Han serif when one is installed.
        let (serifs, ui) = SERIF_FALLBACKS.split_at(SERIF_FALLBACK_SERIFS);
        for face in ["Songti SC", "Source Han Serif SC", "Noto Serif CJK SC"] {
            assert!(serifs.contains(&face), "{face}");
        }
        // Where there is none (Windows), the UI's Chinese face, never
        // SimSun.
        for face in ["PingFang SC", "Microsoft YaHei UI", "Noto Sans CJK SC"] {
            assert!(ui.contains(&face), "{face}");
        }
        assert!(!SERIF_FALLBACKS.contains(&"SimSun"));
    }

    /// Every column, rail, list, menu and dialog body whose content can be
    /// taller than its space scrolls under a stable id, instead of being
    /// cut. Each entry is the file, the id, and how it scrolls: `scroll`
    /// for `.overflow_y_scroll()` after the id, `page` for `page_scroll`,
    /// `menu` for a menu as tall as the window has room for.
    #[test]
    fn what_can_outgrow_its_space_scrolls() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.split_whitespace().collect::<String>())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let scrolls = [
            // Rails of the full pages.
            ("workspace/papers_view.rs", "\"papers-side\"", "scroll"),
            ("workspace/journey_view.rs", "\"journey-rail\"", "scroll"),
            ("workspace/calendar_view.rs", "\"calendar-rail\"", "scroll"),
            (
                "workspace/favorites_view.rs",
                "\"favorites-list\"",
                "scroll",
            ),
            ("workspace/settings_view.rs", "\"settings-nav\"", "scroll"),
            // The pages' content, headed by `page_head` or beside a rail.
            ("workspace/mandates_view.rs", "\"mandates-body\"", "page"),
            ("workspace/schedules_view.rs", "\"schedules-body\"", "page"),
            (
                "workspace/favorites_view.rs",
                "\"favorite-detail\"",
                "scroll",
            ),
            // The sidebar's conversations.
            ("workspace/organise_view.rs", "\"sessions\"", "scroll"),
            // Every tab of the side panel.
            ("workspace/panel_tabs.rs", "\"runs\"", "scroll"),
            ("workspace/hosts_view.rs", "\"hosts\"", "scroll"),
            ("workspace/panel_tabs.rs", "\"artifacts\"", "scroll"),
            ("workspace/memos_view.rs", "\"memos\"", "scroll"),
            ("workspace/memory_view.rs", "\"memory\"", "scroll"),
            ("workspace/favorites_view.rs", "\"highlights\"", "scroll"),
            ("workspace/notebook_view.rs", "\"notebook-cells\"", "scroll"),
            ("workspace/agents_view.rs", "\"agents\"", "scroll"),
            ("workspace/aside_view.rs", "\"aside-thread\"", "scroll"),
            ("workspace/files_panel.rs", "\"provenance-list\"", "scroll"),
            (
                "workspace/remote_files_view.rs",
                "\"remote-files\"",
                "scroll",
            ),
            // Lists and menus under a button.
            ("workspace/tabs_view.rs", "\"waiting-list\"", "menu"),
            ("workspace/shell.rs", "\"outline-menu\"", "menu"),
            ("workspace/home_view.rs", "\"start-menu\"", "menu"),
            ("workspace/files_panel.rs", "\"files-menu\"", "menu"),
            ("workspace/panel_tabs.rs", "\"panel-tab-menu\"", "menu"),
            ("workspace/centre_view.rs", "\"centre-tab-menu\"", "menu"),
            ("workspace/shell.rs", "(\"message-menu\",index)", "menu"),
            ("workspace/shell.rs", "\"composer-menu\"", "menu"),
            // Its height is capped by `menu_room`; the list inside scrolls.
            ("workspace/demos_view.rs", "\"demo-targets\"", "scroll"),
            ("workspace/organise_view.rs", "id", "menu"),
        ];
        // A dialog's body scrolls unless its dialog fixes its own height.
        assert!(
            file("workspace/surfaces.rs").contains(".when(!fixed,|body|body.overflow_y_scroll())")
        );
        let mut missing = Vec::new();
        for (name, id, how) in scrolls {
            let text = file(name);
            let ok = match how {
                "page" => text.contains(&format!("page_scroll({id}")),
                _ => text.match_indices(&format!(".id({id})")).any(|(at, _)| {
                    let after = &text[at..text.len().min(at + 400)];
                    after.contains(".overflow_y_scroll()")
                        && (how != "menu" || after.contains("menu_room("))
                }),
            };
            if !ok {
                missing.push(format!("{name} {id} ({how})"));
            }
        }
        assert!(missing.is_empty(), "{missing:#?}");
        // A menu is never taller than the window has room for, and keeps a
        // little however short the window.
        assert_eq!(
            super::room_in(1000., None, false),
            1000. - TITLE_BAR_HEIGHT - 150.
        );
        assert_eq!(super::room_in(1000., Some(420.), false), 420.);
        assert_eq!(super::room_in(300., Some(420.), false), 120.);
        assert_eq!(super::room_in(1000., None, true), 500. - TITLE_BAR_HEIGHT);
    }

    #[test]
    fn the_panel_strip_ends_on_the_bars_line_with_its_overflow_and_no_close() {
        let tabs = include_str!("panel_tabs.rs");
        let strip = tabs
            .split("pub(super) fn panel_tabs(")
            .nth(1)
            .and_then(|rest| rest.split("\n    }\n").next())
            .expect("the strip");
        // The rest of the tabs is a chevron, not `more`; the bar's toggle
        // and Escape close the panel, so it has no cross of its own.
        assert!(strip.contains("\"chevron-down\""));
        assert!(!strip.contains("\"more\""));
        assert!(!strip.contains("\"close\""));
        assert!(!strip.contains("panel-close"));
        // Its last button and the bar's stand as far from the window's
        // right edge.
        assert!(strip.contains(".pr(px(surfaces::RIGHT_INSET))"));
        let bar = include_str!("surfaces.rs")
            .split("pub(super) fn window_bar(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("the bar");
        assert_eq!(bar.matches(".pr(px(RIGHT_INSET))").count(), 2);
        assert!(!bar.contains(".pr_3()") && !bar.contains(".px_4()"));
        // The start menu's rest of the ways in is a chevron too.
        let home = include_str!("home_view.rs");
        let start = home.split("\"home-more-start\",").nth(1).expect("start");
        assert!(
            start
                .trim_start()
                .lines()
                .nth(1)
                .unwrap()
                .contains("\"chevron-down\"")
        );
    }

    #[test]
    fn a_previews_header_has_back_first_and_three_actions_after_the_name() {
        let panel = include_str!("files_panel.rs");
        let body = |name: &str| {
            panel
                .split(&format!("fn {name}("))
                .nth(1)
                .and_then(|rest| rest.split("\n    }\n").next())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        // Back leads, before the name, in every preview kind.
        assert!(body("preview_parts").contains("surfaces::panel_header_led(back, name"));
        // After the name: where to show it, attach, More; nothing else.
        let actions = body("file_actions");
        assert_eq!(actions.matches("surfaces::icon_action(").count(), 2);
        assert!(actions.contains("\"file-attach\"") && actions.contains("\"file-preview-more\""));
        assert!(actions.contains(".children(place)"));
        assert!(!actions.contains("icon_button("));
        // The rest is under More, each an icon and a word.
        let menu = body("file_preview_menu");
        for id in [
            "file-copy-path",
            "file-keep-artifact",
            "file-quote-aside",
            "file-preview-origin",
            "file-reveal",
            "file-open-system",
        ] {
            assert!(menu.contains(&format!("\"{id}\"")), "{id}");
        }
        // The side panel's place is Open in centre, the centre's the split.
        assert!(body("render_files").contains("\"file-preview-centre\""));
        assert!(
            include_str!("centre_view.rs")
                .contains("self.preview_parts(None, Some(split.into_any_element())")
        );
    }

    #[test]
    fn markdown_keeps_its_paragraphs_apart_everywhere() {
        // About 0.6 em of 14 px body text, in rems.
        assert!((super::PARAGRAPH_GAP * 16. / 14. - 0.6).abs() < 0.05);
        assert_eq!(
            super::text_style().paragraph_gap,
            gpui::rems(super::PARAGRAPH_GAP)
        );
        // Every text view goes through `surfaces::markdown`, so answers, the
        // side chat, previews, favourites and reports share the gap.
        let mut direct = Vec::new();
        for (path, text) in sources() {
            if path.ends_with("workspace/surfaces.rs") {
                continue;
            }
            if text.contains("TextView::markdown(") {
                direct.push(path.display().to_string());
            }
        }
        assert!(direct.is_empty(), "{direct:#?}");
        // The vendored library gives each block of the root its own
        // "last", so the gap is drawn (vendor/gpui-component/WISP-PATCH.md).
        let node = include_str!("../../../../vendor/gpui-component/src/text/node.rs");
        assert!(node.contains("options.is_last(ix == last)"));
    }

    #[test]
    fn answers_are_the_ui_face_in_regular_with_its_chinese_named() {
        let face = super::body_font(".SystemUIFont".into());
        assert_eq!(face.family.as_ref(), ".SystemUIFont");
        assert_eq!(face.weight, gpui::FontWeight::NORMAL);
        let fallbacks = face.fallbacks.expect("body text names its Chinese");
        assert_eq!(
            fallbacks.fallback_list(),
            super::FACT_FALLBACKS.map(String::from)
        );
        // Wisp's answers and a long question are set in it.
        let shell = include_str!("shell.rs");
        let answer = shell
            .split("// An answer with no text draws nothing")
            .nth(1)
            .expect("the answer arm");
        assert!(answer.contains(".font(surfaces::body_face(cx))"));
        let question = include_str!("surfaces.rs")
            .split("pub(super) fn question(")
            .nth(1)
            .expect("the question");
        assert!(question.contains("question.font(body_face(cx))"));
    }

    #[test]
    fn a_fact_is_the_bundled_monospace_with_even_figures() {
        let face = super::fact_face();
        assert_eq!(face.family.as_ref(), crate::assets::MONO_FACE);
        assert_eq!(face.weight, gpui::FontWeight::NORMAL);
        // A Chinese word in a fact falls back to a UI face, not a serif.
        let fallbacks = face.fallbacks.as_ref().unwrap().fallback_list();
        assert_eq!(fallbacks.first().map(String::as_str), Some("PingFang SC"));
        assert!(
            face.features
                .tag_value_list()
                .iter()
                .any(|(tag, value)| tag == "tnum" && *value == 1)
        );
        // The code and terminal font stays the setting's: the fact face is
        // not the theme's monospace.
        let files = sources();
        let (_, theme) = files
            .iter()
            .find(|(path, _)| path.ends_with("src/theme.rs"))
            .unwrap();
        assert!(!theme.contains("MONO_FACE"));
    }

    #[test]
    fn icons_come_in_three_sizes_and_icon_buttons_are_large_enough_to_hit() {
        assert_eq!(IconSize::ALL.map(IconSize::px), [16., 18., 20.]);
        assert!(ICON_BUTTON >= 26.);
        assert!(ICON_BUTTON > IconSize::Toolbar.px());
        // An icon button cannot be made without its tooltip.
        let files = sources();
        let (_, surfaces) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/surfaces.rs"))
            .unwrap();
        let action = free_function(code(surfaces), "icon_action").unwrap();
        assert!(action.contains("tooltip: impl Into<SharedString>"));
        assert!(action.contains(".tooltip(tooltip)"));
    }

    #[test]
    fn a_stamp_is_a_word_in_a_hairline_box_that_reads_on_the_page_in_both_themes() {
        use crate::present::Tone;
        assert_eq!(
            StampKind::ALL.map(StampKind::icon),
            ["circle-check", "circle-ellipsis", "circle-alert", "loader"]
        );
        for kind in StampKind::ALL {
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("assets/icons/{}.svg", kind.icon()))
                    .exists()
            );
        }
        // A stamp is a box and a word: no icon, no Medium. Only the home
        // page's bell keeps one, through `stamp_with`.
        let files = sources();
        let (_, surfaces) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/surfaces.rs"))
            .unwrap();
        let surfaces = code(surfaces);
        let plain = free_function(surfaces, "stamp").unwrap();
        assert!(!plain.contains("icon"));
        let shape = free_function(surfaces, "stamp_box").unwrap();
        assert!(!shape.contains("FontWeight"));
        assert!(shape.contains(".font(fact_face())"));
        assert!(shape.contains(".rounded(px(STAMP_RADIUS))"));
        assert!(!shape.contains(".bg("));
        let users: Vec<_> = files
            .iter()
            .filter(|(_, text)| code(text).contains("surfaces::stamp_with("))
            .map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(users, ["home_view.rs"]);
        assert_eq!(StampKind::from_tone(Tone::Neutral), None);
        assert_eq!(
            StampKind::from_tone(Tone::Attention),
            Some(StampKind::Waiting)
        );
        assert_eq!(StampKind::from_tone(Tone::Active), Some(StampKind::Running));
        // The word on the paper reaches 4.5:1: the `active` shade on the
        // light page, the colour itself on the dark one.
        for theme in crate::theme::bundled().themes {
            let colors = &theme.colors;
            let dark = theme.mode.is_dark();
            let page = colors.background.as_ref().unwrap().to_string();
            for (kind, colour, active) in [
                ("success", &colors.success, &colors.success_active),
                ("warning", &colors.warning, &colors.warning_active),
                ("danger", &colors.danger, &colors.danger_active),
                ("info", &colors.info, &colors.info_active),
            ] {
                let ink = stamp_shade(dark, colour, active)
                    .as_ref()
                    .map(|ink| ink.to_string())
                    .unwrap_or_else(|| panic!("{}: no {kind}", theme.name));
                let contrast = crate::theme::contrast(&ink, &page);
                assert!(
                    contrast >= 4.5,
                    "{}: {kind} {ink} on {page} is {contrast:.2}",
                    theme.name
                );
            }
        }
    }

    #[test]
    fn a_record_row_keeps_its_columns_when_a_cell_is_empty() {
        assert!(RECORD_ROW_HEIGHT >= 32.);
        let bare = record_row("r", "run the job");
        // No time, no time column: the icon starts the row.
        assert_eq!(bare.time_width, 0.);
        assert!(bare.actions.is_empty() && !bare.mono && bare.suffix.is_none());
        assert_eq!(bare.content.as_ref(), "run the job");
        assert!(bare.time.is_none() && bare.icon.is_none() && bare.stamp.is_none());
        assert!(!bare.value_column);
        let full = record_row("r", "bwa index")
            .time("15:41")
            .icon("flask")
            .lead("Server")
            .stamp(StampKind::Failed, "Failed")
            .value("exit 137");
        assert_eq!(full.time_width, super::RECORD_TIME_WIDTH);
        assert_eq!(full.icon, Some("flask"));
        assert_eq!(
            full.lead.map(|lead| lead.to_string()).as_deref(),
            Some("Server")
        );
        assert_eq!(
            full.stamp.as_ref().map(|(kind, _)| *kind),
            Some(StampKind::Failed)
        );
        assert!(full.value_column);
        // A row of a group with values keeps the column with none of its own.
        let kept = record_row("r", "rsync").value_column();
        assert!(kept.value_column && kept.value.is_none());
        // The time and the icon have fixed columns, empty or not, and the
        // group draws the hairline over the first row.
        let files = sources();
        let (_, surfaces) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/surfaces.rs"))
            .unwrap();
        let surfaces = code(surfaces);
        let render = &surfaces[surfaces.find("impl RenderOnce for RecordRow").unwrap()..];
        assert!(render.contains(".w(rems(self.time_width / 16.))"));
        assert!(render.contains(".gap_2p5()"));
        assert!(render.contains(".w(IconSize::Inline.rems())"));
        assert!(render.contains(".min_h(px(RECORD_ROW_HEIGHT))"));
        assert!(render.contains(".border_b_1()"));
        assert!(
            free_function(surfaces, "record_group")
                .unwrap()
                .contains(".border_t_1()")
        );
    }

    #[test]
    fn a_groups_time_column_fits_its_widest_time_and_no_more() {
        use super::{RECORD_TIME_WIDTH, record_time_width};
        // Short times keep the narrow column, close to the icon.
        assert_eq!(record_time_width(["17:04", "11-15"]), RECORD_TIME_WIDTH);
        assert_eq!(record_time_width(["昨天", "09:30"]), RECORD_TIME_WIDTH);
        // A group with no times has no time column at all.
        assert_eq!(record_time_width(Vec::<&str>::new()), 0.);
        assert!(RECORD_TIME_WIDTH >= 5. * super::FACT_COLUMN);
        // One full date widens the column for the whole group.
        let wide = record_time_width(["17:04", "2025-11-15"]);
        assert!(wide > RECORD_TIME_WIDTH);
        assert!(wide >= 10. * super::FACT_COLUMN && wide <= 80.);
        assert!(record_time_width(["Yesterday"]) >= 9. * super::FACT_COLUMN);
    }

    #[test]
    fn the_window_bar_is_the_margin_colour_with_the_traffic_lights_inside_it() {
        // Our own bar on macOS and Windows, the system's on Linux.
        assert_eq!(
            OWN_TITLE_BAR,
            cfg!(any(target_os = "macos", target_os = "windows"))
        );
        assert!(!cfg!(target_os = "linux") || !OWN_TITLE_BAR);
        let options = window_titlebar();
        assert_eq!(options.appears_transparent, OWN_TITLE_BAR);
        // Traffic lights are macOS's alone.
        assert_eq!(
            options.traffic_light_position.is_some(),
            cfg!(target_os = "macos")
        );
        // On Windows the top edge stays the system's to resize, and what
        // takes a click in the bar and is not a button is not the caption.
        {
            let files = sources();
            let (_, surfaces) = files
                .iter()
                .find(|(path, _)| path.ends_with("workspace/surfaces.rs"))
                .unwrap();
            let surfaces = code(surfaces);
            let bar = free_function(surfaces, "window_bar").unwrap();
            assert!(bar.contains("\"window-top-edge\""));
            assert!(bar.contains("cfg!(target_os = \"windows\")"));
            assert!(
                free_function(surfaces, "window_tab")
                    .unwrap()
                    .contains(".occlude()")
            );
            let (_, tabs) = files
                .iter()
                .find(|(path, _)| path.ends_with("workspace/tabs_view.rs"))
                .unwrap();
            let bell = method(tabs, "render_waiting").unwrap();
            assert!(bell.contains(".occlude()"));
            // The home page's banner is a bar to act on, not a grey block.
            let (_, home) = files
                .iter()
                .find(|(path, _)| path.ends_with("workspace/home_view.rs"))
                .unwrap();
            let page = method(home, "render_home").unwrap();
            assert!(page.contains("surfaces::bar(\"home-configure\", cx)"));
            assert!(!page.contains(".bg(cx.theme().muted)"));
            assert!(page.contains("home::last_cell_spans("));
        }
        // The lights (about 14 px high, 52 wide) sit in the bar, centred,
        // and the bar's own content starts past them.
        let (x, y) = TRAFFIC_LIGHTS;
        assert!((y * 2. + 14. - TITLE_BAR_HEIGHT).abs() <= 1.);
        assert!(x + 52. < TRAFFIC_LIGHTS_INSET);
        // In either theme and any palette it takes the margin's colour
        // and line, not a colour of its own.
        let files = sources();
        let (_, surfaces) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/surfaces.rs"))
            .unwrap();
        let bar = free_function(code(surfaces), "window_bar").unwrap();
        assert_eq!(bar.matches(".bg(cx.theme().sidebar)").count(), 2);
        assert_eq!(bar.matches("theme.sidebar_border").count(), 2);
        assert!(!bar.contains("title_bar"));
        // Every page is framed by it; the home page puts its top bar in it.
        let (_, workspace) = files
            .iter()
            .find(|(path, _)| path.ends_with("src/workspace.rs"))
            .unwrap();
        assert!(workspace.contains("bar_content = Some(self.render_home_bar(window, cx)"));
        assert_eq!(workspace.matches("surfaces::window_frame(").count(), 2);
        let (_, main) = files
            .iter()
            .find(|(path, _)| path.ends_with("src/main.rs"))
            .unwrap();
        assert!(main.contains("titlebar: Some(workspace::window_titlebar())"));
        assert!(main.contains("assets::register_fonts(cx)"));
    }

    /// The text of one arm of a `match` in `body`, from `"name" => ` to
    /// the next arm named in `next`.
    fn arm<'a>(body: &'a str, name: &str, next: &str) -> &'a str {
        let from = body
            .find(&format!("\"{name}\" => {{"))
            .unwrap_or_else(|| panic!("no arm {name}"));
        let rest = &body[from..];
        &rest[..rest
            .find(next)
            .unwrap_or_else(|| panic!("no {next} after {name}"))]
    }

    #[test]
    fn the_project_pages_bar_holds_the_tabs_and_at_most_four_icon_buttons() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        // The bar's right end: search, terminals, side panel, More, each an
        // icon button with its tooltip.
        let tabs = file("workspace/tabs_view.rs");
        let actions = method(tabs, "render_bar_actions").unwrap();
        assert_eq!(actions.matches("surfaces::icon_action(").count(), 4);
        assert!(!actions.contains("icon_button("));
        for name in ["\"search\"", "\"terminal\"", "\"panel\"", "\"more\""] {
            assert!(actions.contains(name), "{name}");
        }
        // The sidebar's toggle at its left end, then the tabs, each its
        // object's icon and name.
        let bar = method(tabs, "render_project_bar").unwrap();
        assert!(bar.contains("\"panel-left\""));
        assert!(bar.contains("self.render_tabs(cx)"));
        let strip = method(tabs, "render_tabs").unwrap();
        assert!(strip.contains("surfaces::window_tab("));
        assert!(strip.contains("\"chat\""));
        let centre = method(file("workspace/centre_view.rs"), "centre_tabs").unwrap();
        assert!(centre.contains("surfaces::window_tab("));
        assert!(centre.contains("\"doc\""));
        // The conversation's own header row is gone: the page holds no tab
        // strip or header of its own, and the bar is the window's.
        let conversation = method(file("workspace/shell.rs"), "render_conversation").unwrap();
        for gone in ["render_tabs(", "render_topbar("] {
            assert!(!conversation.contains(gone), "{gone}");
        }
        assert!(!file("workspace/shell.rs").contains("fn render_topbar("));
        assert!(file("src/workspace.rs").contains(
            "bar_content = Some(self.render_project_bar(window, cx).into_any_element())"
        ));
        // What the header held is in the More menu, each an icon and a word.
        let menu = method(
            file("workspace/conversation_actions.rs"),
            "conversation_menu",
        )
        .unwrap();
        assert!(!menu.contains("menu_row("));
        for kept in [
            "Layer::Outline",
            "this.refresh_lists(",
            "this.begin_rename(",
            "this.toggle_pin(",
            "self.group_button(",
            "self.transfer_button(",
            "this.export_conversation(",
            "this.open_review(",
            "this.toggle_archive(",
            "this.confirm_delete(",
        ] {
            assert!(menu.contains(kept), "{kept}");
        }
    }

    #[test]
    fn the_transcript_is_entries_on_a_record_line() {
        assert_eq!(super::ENTRY_GUTTER, 64.);
        assert_eq!(super::entry_label(12), "№ 12");
        let files = sources();
        let (_, shell) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/shell.rs"))
            .unwrap();
        // Every row is framed as part of the record, numbered by one
        // function.
        let row = method(shell, "row").unwrap();
        assert!(row.contains("present::entry_number(&self.rows, index, self.earlier_users)"));
        assert!(row.contains("surfaces::entry_row("));
        assert!(row.contains("surfaces::EntryDot::Live"));
        // The line is each row's own left edge, so rows stack without gaps.
        let (_, surfaces) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/surfaces.rs"))
            .unwrap();
        let entry = free_function(code(surfaces), "entry_row").unwrap();
        assert!(entry.contains(".border_l_1()"));
        assert!(!entry.contains(".gap_"));
        let body = method(shell, "row_body").unwrap();
        // The question is a title on the paper: no bubble, no fill, not to
        // the right.
        let user = arm(body, "user", "\"tool\" => {");
        assert!(user.contains("surfaces::question("));
        for bubble in [".bg(", ".rounded(", ".items_end()", ".justify_end()"] {
            assert!(!user.contains(bubble), "the question uses {bubble}");
        }
        // Tool calls, thoughts, reviews and branches brought back are
        // record rows, drawn only through `record_row`.
        let tool = arm(body, "tool", "\"reasoning\" if");
        assert!(tool.contains("surfaces::record_row("));
        assert!(tool.contains("present::tool_icon(item)"));
        assert!(tool.contains("present::tool_outcome("));
        assert!(tool.contains("self.transcript_record("));
        for card in [".border_1()", ".rounded(", "chip(", "icon(\"wrench\")"] {
            assert!(!tool.contains(card), "a tool row uses {card}");
        }
        for name in [
            "reasoning\" => {",
            "review\" => match",
            "branch_merge\" => {",
        ] {
            let at = body.find(name).unwrap_or_else(|| panic!("{name}"));
            let rest = &body[at..];
            let rest = &rest[..rest.find("self.transcript_record(").unwrap()];
            assert!(rest.contains("surfaces::record_row("), "{name}");
            assert!(!rest.contains(".border_1()"), "{name}");
        }
        let record = method(shell, "transcript_record").unwrap();
        assert!(record.contains("surfaces::record_group("));
        // A short question is a title, a long one body text; both open an
        // entry 28 px below the last, and their actions take no line.
        assert!(user.contains("present::title_question(&text)"));
        assert!(user.contains(".pt(self.entry_gap(index))"));
        assert!(user.contains(".absolute()"));
        // A call that runs something is one row, the run's: its title,
        // state and time, ask aside and open in Runs at its end, and the
        // rest of its card only when it is opened.
        assert!(tool.contains("self.run_record("));
        assert!(tool.contains("open.then(|| self.run_card("));
        let run = method(shell, "run_record").unwrap();
        assert!(run.contains("self.run_record_row("));
        assert!(run.contains("\"external-link\""));
        let (_, cards) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/run_cards_view.rs"))
            .unwrap();
        let shared = method(cards, "run_record_row").unwrap();
        for part in [
            "\"flask\"",
            "present::run_status(",
            "present::run_elapsed_ms(",
            ".or(fallback_ms)",
            "present::duration_label(",
            "\"bubble\"",
        ] {
            assert!(shared.contains(part), "{part}");
        }
        // A tool's name is an identifier, set as one; what it works on
        // leads when its input says.
        assert!(tool.contains("present::tool_summary(item)"));
        assert!(tool.contains(".mono()"));
        // Under a message: copy, favourite, ask aside and More, as icon
        // buttons; the rest in the More menu.
        let actions = method(shell, "message_action_items").unwrap();
        assert_eq!(actions.matches("surfaces::icon_action(").count(), 4);
        // Asking aside is the follow-up bubble of the vocabulary.
        assert!(actions.contains("\"bubble\""));
        assert!(!actions.contains("circle-help"));
        assert!(!actions.contains("icon_button("));
        let menu = method(shell, "message_menu").unwrap();
        for kept in [
            "this.remember_row(",
            "this.quote_row(",
            "this.reuse_turn(",
            "this.ask_rewind(",
            "this.branch_from(",
            "this.open_review_of(",
            "this.ask_explore_from(",
        ] {
            assert!(menu.contains(kept), "{kept}");
        }
    }

    #[test]
    fn text_is_regular_and_medium_is_kept_to_where_the_rule_allows_it() {
        // Weight is not how the interface ranks things: colour, ground and
        // teal are. No semibold or bold anywhere; Medium only for the
        // display line, the wordmark, and a terminal's bold, which is
        // content. Settings follow it too: their filled button's label is
        // the regular weight the component library gives it.
        let allowed = [
            ("workspace/surfaces.rs", "DISPLAY_WEIGHT"),
            ("workspace/home_view.rs", "render_home_bar"),
            ("workspace/terminal_view.rs", "*"),
        ];
        let mut found = Vec::new();
        for (path, text) in sources() {
            let name = path.to_string_lossy().replace('\\', "/");
            let code = code(&text);
            for heavy in [
                "SEMIBOLD",
                "FontWeight::BOLD",
                "EXTRA_BOLD",
                "font_semibold()",
                "font_bold()",
            ] {
                if code.contains(heavy) {
                    found.push(format!("{name} uses {heavy}"));
                }
            }
            for (at, _) in code.match_indices("FontWeight::MEDIUM") {
                let function = function_at(code, at).unwrap_or_default();
                let line_start = code[..at].rfind('\n').map_or(0, |start| start + 1);
                let line =
                    &code[line_start..code[at..].find('\n').map_or(code.len(), |end| at + end)];
                let ok = allowed.iter().any(|(file, place)| {
                    name.ends_with(file)
                        && (*place == "*" || *place == function || line.contains(place))
                });
                if !ok {
                    found.push(format!("{name}: MEDIUM in fn {function}"));
                }
            }
            if code.contains("font_medium()") {
                found.push(format!("{name} uses font_medium()"));
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn full_pages_keep_their_rails_short_and_their_records_as_record_rows() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        // A page's rail says what the page is for in a help icon's tooltip,
        // not in a paragraph.
        for (name, function) in [
            ("workspace/journey_view.rs", "render_journey"),
            ("workspace/papers_view.rs", "render_papers"),
            ("workspace/calendar_view.rs", "render_calendar"),
            ("workspace/schedules_view.rs", "render_schedules"),
            ("workspace/mandates_view.rs", "render_mandates"),
            ("workspace/favorites_view.rs", "render_favorites"),
        ] {
            let body: String = method(file(name), function)
                .unwrap_or_else(|| panic!("{name} has no fn {function}"))
                .split_whitespace()
                .collect();
            assert!(
                body.contains("surfaces::help(") || body.contains("surfaces::page_head("),
                "{function}"
            );
            assert!(
                !body.contains(".px_2().pb_2().text_xs().text_color(muted).child(tr\u{28}"),
                "{function} keeps a paragraph in its rail"
            );
            assert!(
                !body.contains(".px_2().pb_1().text_xs().text_color(muted).child(tr\u{28}"),
                "{function} keeps a paragraph in its rail"
            );
        }
        // The days' records are record rows, the kind an icon, not a word.
        for (name, function) in [
            ("workspace/journey_view.rs", "render_journey"),
            ("workspace/calendar_view.rs", "render_calendar"),
        ] {
            let body = method(file(name), function).unwrap();
            assert!(body.contains("surfaces::record_row("), "{function}");
            assert!(body.contains("journey::kind_icon("), "{function}");
            assert!(!body.contains("journey::kind_label("), "{function}");
        }
        // Their records sit in a centred column, not pinned to the left.
        for (name, function) in [
            ("workspace/calendar_view.rs", "render_calendar"),
            ("workspace/schedules_view.rs", "render_schedules"),
            ("workspace/mandates_view.rs", "render_mandates"),
        ] {
            let body = method(file(name), function).unwrap();
            assert!(body.contains("surfaces::page_column()"), "{function}");
            assert!(body.contains("surfaces::page_scroll("), "{function}");
        }
        for (name, function) in [
            ("workspace/journey_view.rs", "render_journey"),
            ("workspace/papers_view.rs", "render_papers"),
        ] {
            let body = method(file(name), function).unwrap();
            assert!(
                body.contains(".max_w(px(surfaces::READING_WIDTH))"),
                "{function}"
            );
        }
        // The views switch is segmented; the month a fact between arrows.
        let journey = method(file("workspace/journey_view.rs"), "render_journey").unwrap();
        assert!(journey.contains("surfaces::segmented("));
        assert!(journey.contains("surfaces::fact(month.label()"));
        // A paper's actions: one outlined icon and word, the rest icon
        // buttons, Remove last in the danger colour.
        let paper = method(file("workspace/papers_view.rs"), "paper_body").unwrap();
        assert!(paper.contains("surfaces::outlined_action(\"paper-finalize\""));
        assert!(paper.contains("surfaces::danger_action(\"paper-remove\""));
        assert!(!paper.contains("Button::new(\"paper-rename\")"));
        // A schedule's actions are icons.
        let schedule = method(file("workspace/schedules_view.rs"), "schedule_row").unwrap();
        assert!(!schedule.contains(".label(tr(\"Run now\"))"));
        // What waits needs no stamp of its own in the bell's list.
        let waiting = method(file("workspace/tabs_view.rs"), "waiting_list").unwrap();
        assert!(!waiting.contains(".stamp("));
    }

    #[test]
    fn pages_lists_dialogs_and_the_palette_follow_the_rules() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let squeeze = |text: &str| -> String { text.split_whitespace().collect() };
        // Pages with nothing to list keep no rail: back, the name and New
        // head the centred column.
        for (name, function) in [
            ("workspace/mandates_view.rs", "render_mandates"),
            ("workspace/schedules_view.rs", "render_schedules"),
        ] {
            let body = method(file(name), function).unwrap();
            assert!(body.contains("surfaces::page_head("), "{function}");
            assert!(
                !body.contains(".bg(cx.theme().sidebar)"),
                "{function} has a rail"
            );
            assert!(!body.contains("nav_row("), "{function} has a rail");
        }
        // A mandate's actions are icons, two of them with words, and what
        // it asks is a white card with Answer.
        let mandate = squeeze(method(file("workspace/mandates_view.rs"), "mandate_card").unwrap());
        assert!(!mandate.contains(".label(tr(label))"));
        assert!(mandate.contains("surfaces::outlined_action((\"mandate-run\",index),\"play\""));
        assert!(mandate.contains("surfaces::danger_action((\"mandate-delete\",index)"));
        assert!(mandate.contains("surfaces::bar((\"mandate-request\",index),cx)"));
        assert!(!mandate.contains("tone_color(Tone::Attention,cx).opacity("));
        // Templates are chips with their prompt in the tooltip; the off
        // state is not said twice.
        let schedules = method(file("workspace/schedules_view.rs"), "render_schedules").unwrap();
        assert!(schedules.contains("surfaces::choice_chip("));
        let task = method(file("workspace/schedules_view.rs"), "schedule_row").unwrap();
        assert!(task.contains(".when(row.enabled,"));
        // The weekdays go through the table.
        let calendar = method(file("workspace/calendar_view.rs"), "render_calendar").unwrap();
        assert!(calendar.contains(".child(tr(name))"));
        assert!(calendar.contains("surfaces::choice_chip("));
        // Favorites: a segmented kind filter and record rows.
        let favorites = method(file("workspace/favorites_view.rs"), "render_favorites").unwrap();
        assert!(favorites.contains("surfaces::segmented("));
        assert!(favorites.contains("surfaces::record_row("));
        // Titles drawn from a message's first line drop Markdown marks.
        for (name, function) in [
            ("workspace/home_view.rs", "render_home"),
            ("workspace/tabs_view.rs", "waiting_list"),
            ("workspace/favorites_view.rs", "render_favorites"),
            ("workspace/journey_view.rs", "render_journey"),
        ] {
            let body = method(file(name), function).unwrap();
            assert!(body.contains("present::title_line("), "{function}");
        }
        // A segmented control hugs its segments.
        let segmented = free_function(code(file("workspace/surfaces.rs")), "segmented").unwrap();
        assert!(segmented.contains("AlignSelf::FlexStart"));
        // The palette lists record rows, the chosen one on white with a line.
        let palette = method(file("workspace/palette_view.rs"), "render_palette").unwrap();
        assert!(palette.contains("surfaces::record_row("));
        assert!(!palette.contains("icon_button("));
        // A list's data-supplied description is one line.
        let connections = method(
            file("workspace/settings_sections.rs"),
            "render_connection_settings",
        )
        .unwrap();
        assert!(connections.contains("surfaces::one_line_note("));
        // Fields in dialogs are white: none is the library's input as is.
        for (name, function) in [
            ("workspace/home_view.rs", "render_new_project"),
            ("workspace/privacy_view.rs", "render_privacy"),
            ("workspace/project_settings.rs", "render_project_settings"),
            ("workspace/project_sync_view.rs", "render_join_synced"),
            ("workspace/explore_view.rs", "render_start_exploration"),
            ("workspace/share_view.rs", "render_share"),
        ] {
            let body = method(file(name), function).unwrap();
            assert!(!body.contains("Input::new("), "{function}");
        }
    }

    #[test]
    fn the_sidebar_says_states_with_icons_and_keeps_its_heading_short() {
        let files = sources();
        let file = |name: &str| {
            files
                .iter()
                .find(|(path, _)| path.ends_with(name))
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        let organise = file("workspace/organise_view.rs");
        // A conversation's state is an icon in its colour with the word as
        // its tooltip, not a word that takes the title's room.
        let row = method(organise, "session_row").unwrap();
        assert!(row.contains("present::session_mark("));
        assert!(row.contains(".tooltip("));
        assert!(!row.contains("chip("));
        // The open conversation is white with a hairline.
        assert!(row.contains(".bg(cx.theme().popover).border_color(cx.theme().border)"));
        // The list's heading: its icon and count, then at most three icon
        // buttons and More.
        let heading = method(organise, "render_list_heading").unwrap();
        let shown = &heading[heading.find("let sorting").unwrap()..];
        assert_eq!(shown.matches("surfaces::icon_action(").count(), 4);
        assert!(shown.contains("\"list-more\""));
        assert!(!shown.contains("icon_button("));
        let menu = method(organise, "list_menu").unwrap();
        assert!(menu.contains("this.open_imports("));
        assert!(menu.contains("this.show_archived = !this.show_archived"));
        // Navigation rows take their icon from the shared sizes and tones.
        let shell = file("workspace/shell.rs");
        let nav = free_function(code(shell), "nav_row").unwrap();
        assert!(nav.contains("surfaces::sized_icon("));
        let sidebar = method(shell, "render_sidebar").unwrap();
        for (id, glyph) in [
            ("\"journey\"", "\"research-trail\""),
            ("\"papers\"", "\"book\""),
            ("\"files\"", "\"folder\""),
            ("\"memos\"", "\"note-plus\""),
            ("\"favorites\"", "\"star\""),
        ] {
            let at = sidebar.find(id).unwrap_or_else(|| panic!("{id}"));
            assert!(
                sidebar[at..]
                    .trim_start_matches(id)
                    .trim_start_matches(',')
                    .trim_start()
                    .starts_with(glyph),
                "{id}"
            );
        }
        // Short of room the navigation folds into one row of icon buttons
        // with the same seven places, each named by its tooltip.
        assert!(sidebar.contains("present::nav_folds("));
        assert!(sidebar.contains("self.folded_nav("));
        let folded = method(shell, "folded_nav").unwrap();
        assert!(folded.contains("surfaces::icon_action("));
        for (id, glyph, opens) in [
            ("\"new-chat\"", "\"plus\"", "this.create_session("),
            (
                "\"files\"",
                "\"folder\"",
                "this.show_panel_tab(PanelTab::Files",
            ),
            (
                "\"memos\"",
                "\"note-plus\"",
                "this.show_panel_tab(PanelTab::Memos",
            ),
            (
                "\"capabilities\"",
                "\"sparkles\"",
                "this.open_capabilities(",
            ),
            ("\"journey\"", "\"research-trail\"", "this.open_journey("),
            ("\"papers\"", "\"book\"", "this.open_papers("),
            ("\"favorites\"", "\"star\"", "this.open_favorites("),
        ] {
            for part in [id, glyph, opens] {
                assert!(folded.contains(part), "the folded row has no {part}");
                assert!(sidebar.contains(part), "the column has no {part}");
            }
        }
        // What waits for the user is not in the sidebar: it is the bell in
        // the window's bar, shown only while something waits, beside the
        // four icon buttons and not one of them; its list is record rows.
        assert!(!sidebar.contains("\"bell\""));
        assert!(!sidebar.contains("self.inbox"));
        let tabs = file("workspace/tabs_view.rs");
        let bar = method(tabs, "render_project_bar").unwrap();
        let bell = bar
            .find("self.render_waiting(")
            .expect("the bell in the bar");
        assert!(bell < bar.find("self.render_bar_actions(").unwrap());
        let waiting = method(tabs, "render_waiting").unwrap();
        assert!(waiting.contains("if count == 0 {"));
        assert!(waiting.contains("\"bell\""));
        assert!(waiting.contains("Layer::Inbox"));
        assert!(!waiting.contains("surfaces::icon_action("));
        let list = method(tabs, "waiting_list").unwrap();
        assert!(list.contains("surfaces::record_row("));
        assert!(list.contains("this.layers.close(Layer::Inbox)"));
        assert!(list.contains("this.open_session("));
        let home = method(file("workspace/home_view.rs"), "render_home_bar").unwrap();
        assert!(home.contains("self.render_waiting("));
    }

    #[test]
    fn the_composer_is_one_box_with_plus_conditions_and_send() {
        let files = sources();
        let (_, shell) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/shell.rs"))
            .unwrap();
        let squeeze = |text: &str| -> String { text.split_whitespace().collect() };
        let composer = [
            "render_composer",
            "render_composer_plus",
            "render_composer_conditions",
            "render_composer_actions",
        ]
        .map(|name| squeeze(method(shell, name).unwrap()))
        .join("");
        // Everything that puts something into the message is in `+`, and
        // the modes are switched there; none is a button of its own.
        let menu = squeeze(method(shell, "plus_menu").unwrap());
        for way in [
            "this.attach_files(",
            "this.toggle_attach_palette(",
            "Layer::QuickMenu",
            "this.open_slash_menu(",
            "this.toggle_plan_first(",
            "this.toggle_full_access(",
            "this.toggle_auto_review(",
            "this.toggle_fast(",
        ] {
            assert!(menu.contains(way), "the + menu has no {way}");
        }
        for gone in [
            "icon_button(\"attach-files\"",
            "icon_button(\"attach-context\"",
            "self.slash_button(",
            "Button::new(\"plan-first\")",
            "Button::new(\"access\")",
            "Button::new(\"auto-review\")",
            "Button::new(\"quick-actions\")",
            "Button::new(\"send\")",
        ] {
            assert!(!composer.contains(gone), "the composer still has {gone}");
        }
        assert!(composer.contains("Layer::PlusMenu"));
        // A mode shows in the line only while it is on, in teal.
        assert!(composer.contains("ifon{conditions.push(surfaces::condition("));
        assert!(composer.contains("surfaces::IconTone::On"));
        // The conditions are segments with an icon, a value and a tooltip.
        for segment in [
            "surfaces::condition(\"model\",\"bot\"",
            "surfaces::condition(\"effort\",\"gauge\"",
            "surfaces::condition(\"runs-on\",settings::runs_on_icon(",
            "surfaces::condition(\"expert\",\"user\"",
        ] {
            assert!(composer.contains(segment), "{segment}");
        }
        let condition = squeeze(free_function(code(shell_free(&files)), "condition").unwrap());
        assert!(condition.contains("tooltip:implInto<SharedString>"));
        assert!(condition.contains(".tooltip("));
        assert!(!condition.contains(".border_1()"));
        assert_eq!(super::CONDITION_GAP, 14.);
        assert_eq!(crate::settings::runs_on_icon("", None), "computer");
        assert_eq!(crate::settings::runs_on_icon("lab", None), "server");
        assert_eq!(crate::settings::runs_on_icon("", Some("lab")), "server");
        // Send is a square teal icon button, stop in its place while a turn
        // runs; `+` first and send last on the first line, the conditions
        // between wrapping.
        assert!(composer.contains("surfaces::send_button(\"send\",\"arrow-up\""));
        assert!(composer.contains("surfaces::send_button(\"stop\",\"stop\""));
        let send = squeeze(free_function(code(shell_free(&files)), "send_button").unwrap());
        assert!(send.contains(".size(px(ICON_BUTTON))"));
        assert!(send.contains(".primary()"));
        let line = &composer[composer.find(".child(plus)").expect("the + button")..];
        let wrap = line.find(".flex_wrap()").expect("the conditions wrap");
        let segments = line.find(".children(conditions)").expect("the conditions");
        let end = line.find(".child(actions)").expect("send");
        assert!(wrap < segments && segments < end);
        // An answer the entry goes on after lays its actions over its
        // corner, so no line of them stands before what follows.
        let body = squeeze(method(shell, "row_body").unwrap());
        assert!(body.contains("self.message_actions(index,!ends_entry,cx)"));
    }

    /// `surfaces.rs`, where the shared shapes are.
    fn shell_free(files: &[(std::path::PathBuf, String)]) -> &str {
        files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/surfaces.rs"))
            .map(|(_, text)| text.as_str())
            .unwrap()
    }

    #[test]
    fn the_home_page_lists_recent_conversations_as_record_rows() {
        let files = sources();
        let (_, home) = files
            .iter()
            .find(|(path, _)| path.ends_with("workspace/home_view.rs"))
            .unwrap();
        let page = method(home, "render_home").unwrap();
        assert!(page.contains("surfaces::record_row("));
        assert!(page.contains("surfaces::record_group("));
        assert!(page.contains("StampKind::from_tone("));
        // Its places to go are icon and word, docs and settings icon
        // buttons, every one with a tooltip.
        let bar = method(home, "render_home_bar").unwrap();
        assert_eq!(bar.matches("surfaces::icon_action(").count(), 2);
        assert!(bar.contains("surfaces::labelled_action("));
        assert!(!bar.contains("icon_button("));
        // A project's waiting work is a stamp with the bell. Its actions
        // show on the cell's hover over its corner, taking no height, each
        // an icon button with a tooltip, and the cell opens the project.
        let cell = method(home, "project_cell").unwrap();
        assert!(cell.contains("surfaces::stamp_with("));
        assert!(cell.contains("\"bell\""));
        assert!(cell.contains(".group(group.clone())"));
        assert!(cell.contains(".invisible()"));
        assert!(cell.contains(".group_hover(group, |style| style.visible())"));
        assert!(cell.contains(".absolute()"));
        assert!(!cell.contains("icon_button("));
        assert!(cell.contains("this.enter_project(id.clone(), window, cx)"));
        // The ways to start stay on one line; the rarer ones are a menu
        // that joins the Escape stack.
        let start = &page[page.find("let start = div()").unwrap()..];
        let start = &start[..start.find(';').unwrap()];
        assert!(!start.contains("flex_wrap"));
        assert!(page.contains("Layer::StartMenu"));
        let menu = method(home, "start_menu").unwrap();
        for way in [
            "this.open_examples(",
            "this.open_webview_import(",
            "this.open_join_synced(",
            "this.create_sample_project(",
        ] {
            assert!(menu.contains(way), "{way}");
        }
        assert!(menu.contains("this.layers.close(Layer::StartMenu)"));
    }

    #[test]
    fn views_size_text_only_by_role() {
        // Caption and body are text_xs and text_sm; title and display come
        // from this file. A view that reaches for another size makes a
        // fifth role; a heading in text_lg is a Title. The terminal's grid
        // has a size of its own.
        let mut found = Vec::new();
        for (path, text) in sources() {
            let name = path.file_name().and_then(|name| name.to_str());
            if matches!(name, Some("surfaces.rs" | "terminal_view.rs")) {
                continue;
            }
            for other in [
                ".text_base()",
                ".text_lg()",
                ".text_xl()",
                ".text_2xl()",
                ".text_3xl()",
                ".text_size(",
            ] {
                if text.contains(other) {
                    found.push(format!("{} uses {other}", path.display()));
                }
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }

    /// The settings files: their name and the code outside their tests.
    fn settings_files() -> Vec<(String, String)> {
        sources()
            .into_iter()
            .filter_map(|(path, text)| {
                let name = path.file_name()?.to_str()?.to_owned();
                (name.starts_with("settings_") && path.parent()?.ends_with("workspace"))
                    .then(|| (name, code(&text).to_owned()))
            })
            .collect()
    }

    /// The body of the method `name` at an `impl`'s indent in `source`.
    fn method_body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
        let start = source.find(&format!("fn {name}("))?;
        let end = source[start..]
            .find("\n    }\n")
            .map_or(source.len(), |end| start + end);
        Some(&source[start..end])
    }

    /// The settings' pages are more than a short window holds: the list
    /// scrolls under the back row, the title and the search, which stay.
    #[test]
    fn the_settings_navigation_scrolls_under_its_head() {
        let view = include_str!("settings_view.rs");
        let frame = &view[view.find("fn render_settings(").unwrap()..];
        let list = &frame[frame.find(".id(\"settings-nav\")").expect("the list")..];
        let list = &list[..list.find(';').unwrap()];
        for needed in [".flex_1()", ".min_h_0()", ".overflow_y_scroll()"] {
            assert!(list.contains(needed), "{needed}");
        }
        // The head is added to the column before the list, not inside it.
        let head = frame
            .find("surfaces::input(&self.settings_search, cx)")
            .unwrap();
        assert!(head < frame.find(".id(\"settings-nav\")").unwrap());
        assert!(frame.contains("let nav = nav.child(list);"));
    }

    #[test]
    fn every_settings_page_is_the_one_column_of_setting_rows() {
        let files = settings_files();
        assert!(files.len() >= 18, "{}", files.len());
        let file = |name: &str| {
            files
                .iter()
                .find(|(file, _)| file == name)
                .map(|(_, text)| text.as_str())
                .unwrap_or_else(|| panic!("no {name}"))
        };
        // The frame: every page in one centred column, its name a Title at
        // the top; no header band and no page of its own width.
        let frame = method_body(file("settings_view.rs"), "render_settings").unwrap();
        let squeezed: String = frame.split_whitespace().collect();
        assert!(squeezed.contains("surfaces::settings_column(self.settings_section.label(),body"));
        assert!(!frame.contains("h(px(52."), "a header band is back");
        assert!((620. ..=720.).contains(&super::SETTINGS_WIDTH));
        let page = method_body(file("settings_sections.rs"), "settings_page").unwrap();
        assert!(
            !page.contains("w(px(") && !page.contains(".p_6()"),
            "{page}"
        );
        // Each page starts from `settings_page`, Models through its list
        // and form, which sit one under the other in the column.
        let mut pages = 0;
        let mut pet = sources();
        pet.retain(|(path, _)| path.ends_with("workspace/pet_view.rs"));
        let pet: Vec<(String, String)> = pet
            .into_iter()
            .map(|(_, text)| ("pet_view.rs".to_owned(), code(&text).to_owned()))
            .collect();
        for (name, text) in files.iter().chain(&pet) {
            for (at, _) in text.match_indices("fn render_") {
                let rest = &text[at + "fn ".len()..];
                let function = rest.split('(').next().unwrap_or_default();
                // `render_settings` is the frame, checked above.
                if !function.ends_with("_settings") || function == "render_settings" {
                    continue;
                }
                pages += 1;
                let body = method_body(text, function).unwrap();
                assert!(
                    body.split_whitespace()
                        .collect::<String>()
                        .contains("self.settings_page(cx)"),
                    "{name}: {function} is not drawn from settings_page"
                );
            }
        }
        assert!(pages >= 20, "{pages}");
        let models = method_body(file("settings_view.rs"), "render_model_form").unwrap();
        assert!(models.contains("setting("));
        let models_page = frame.split("Section::Models =>").nth(1).unwrap();
        let models_page = models_page.split("Section::General").next().unwrap();
        assert!(models_page.contains(".v_flex()") && !models_page.contains(".h_flex()"));
        // Every file that draws a page lays its settings out as `setting`
        // rows (or `stacked_setting` where the control needs the width);
        // the subscriptions are record rows under the models.
        for (name, text) in &files {
            if matches!(name.as_str(), "settings_subscriptions.rs") {
                continue;
            }
            assert!(text.contains("setting("), "{name} has no setting row");
        }
        // The one row shape: the name at the regular weight, the control
        // centred on the row, a hairline under it. `setting` draws it
        // through `item_row`, which a skill or a connector shares.
        let sections = file("settings_sections.rs");
        let row = &sections[sections.find("pub(super) fn setting(").unwrap()..];
        let row = &row[..row.find("\n}\n").unwrap()];
        assert!(row.contains("item_row("), "{row}");
        let shape = &sections[sections.find("fn item_row(").unwrap()..];
        let shape = &shape[..shape.find("\n}\n").unwrap()];
        assert!(shape.contains(".items_center()") && shape.contains(".border_b_1()"));
        assert!(!shape.contains("font_weight"));
    }

    #[test]
    fn settings_use_white_fields_segments_and_switches_at_the_regular_weight() {
        // A field is white with the fields' line, never the grey fill.
        let field = &code(include_str!("surfaces.rs"))[code(include_str!("surfaces.rs"))
            .find("pub(super) fn input(")
            .unwrap()..];
        let field = &field[..field.find("\n}\n").unwrap()];
        assert!(field.contains(".bg(cx.theme().popover)"));
        assert!(field.contains(".border_color(cx.theme().input)"));
        assert!(!field.contains("theme().muted)"));
        let mut found = Vec::new();
        for (name, text) in settings_files() {
            // Fields come from `input`; a choice is a segment or a chip, not
            // a selected button; Regular everywhere.
            for banned in [
                "Input::new(",
                ".selected(",
                "FontWeight::MEDIUM",
                "FontWeight::SEMIBOLD",
                "FontWeight::BOLD",
                "font_medium()",
                "font_semibold()",
                "font_bold()",
            ] {
                if text.contains(banned) {
                    found.push(format!("{name} uses {banned}"));
                }
            }
            // At most one filled button on a page; the files draw a page
            // each, or (settings_sections.rs) several that each have at
            // most the one the General page keeps.
            let filled = text.matches(".primary()").count();
            if filled > 1 {
                found.push(format!("{name} has {filled} filled buttons"));
            }
        }
        assert!(found.is_empty(), "{found:#?}");
        // Pages that show together share one filled button: Chat apps draws
        // the lab's card, Models the subscriptions.
        let files = settings_files();
        let count = |name: &str| {
            files
                .iter()
                .find(|(file, _)| file == name)
                .map_or(0, |(_, text)| text.matches(".primary()").count())
        };
        assert!(count("settings_channels.rs") + count("settings_lab.rs") <= 1);
        assert!(count("settings_view.rs") + count("settings_subscriptions.rs") <= 1);
        // One of a few is a segmented control: appearance (with its
        // icons), language and the send key among them.
        let general = files
            .iter()
            .find(|(file, _)| file == "settings_sections.rs")
            .map(|(_, text)| method_body(text, "render_general_settings").unwrap())
            .unwrap()
            .to_owned();
        assert!(
            general.matches("surfaces::segmented(").count() >= 3,
            "{general}"
        );
        for (choice, icon) in [
            ("appearance-", "\"monitor\""),
            ("language-", "plain_segment"),
            ("send-", "plain_segment"),
        ] {
            let at = general
                .find(choice)
                .unwrap_or_else(|| panic!("no {choice}"));
            let before = &general[..at];
            let segment = before.rfind("surfaces::segment").unwrap();
            assert!(
                before
                    .rfind("Button::new")
                    .is_none_or(|button| button < segment),
                "{choice}"
            );
            assert!(general.contains(icon), "{icon}");
        }
        for icon in ["\"sun\"", "\"moon\""] {
            assert!(general.contains(icon), "{icon}");
        }
        assert!(general.contains("surfaces::stepper(") || general.contains("size_choice("));
    }

    #[test]
    fn a_segment_and_a_chip_are_reached_with_the_keyboard() {
        // They stand in for buttons, so Tab reaches them and Enter or
        // Space press them, as `pressable` gives them.
        let source = code(include_str!("surfaces.rs"));
        for helper in ["fn segment(", "fn plain_segment(", "fn choice_chip("] {
            let at = source.find(helper).unwrap_or_else(|| panic!("no {helper}"));
            let body = &source[at..];
            let body = &body[..body.find("\n}\n").unwrap()];
            assert!(body.contains("-> Pressable"), "{helper}");
            assert!(body.contains("pressable(id)"), "{helper}");
        }
        let render = &source[source.find("impl RenderOnce for Pressable").unwrap()..];
        assert!(render.contains("track_focus(&focus.tab_stop(true))"));
    }

    /// What each call of `name` in `source` is given, split at its
    /// top-level commas, with string and character literals kept whole.
    fn call_args(source: &str, name: &str) -> Vec<Vec<String>> {
        let mut calls = Vec::new();
        for (at, _) in source.match_indices(name) {
            let before = source[..at].chars().next_back();
            if before.is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
                || source[..at].ends_with("fn ")
            {
                continue;
            }
            let rest = &source[at + name.len()..];
            let chars: Vec<char> = rest.chars().collect();
            let (mut depth, mut quoted, mut index) = (1, false, 0);
            let mut args = vec![String::new()];
            while index < chars.len() {
                let ch = chars[index];
                match ch {
                    '\\' if quoted => {
                        args.last_mut().unwrap().push(ch);
                        index += 1;
                        if let Some(next) = chars.get(index) {
                            args.last_mut().unwrap().push(*next);
                        }
                        index += 1;
                        continue;
                    }
                    '"' => quoted = !quoted,
                    '(' | '[' | '{' if !quoted => depth += 1,
                    ')' | ']' | '}' if !quoted => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    ',' if !quoted && depth == 1 => {
                        args.push(String::new());
                        index += 1;
                        continue;
                    }
                    _ => {}
                }
                args.last_mut().unwrap().push(ch);
                index += 1;
            }
            calls.push(args);
        }
        calls
    }

    /// The first string literal in `arg`, as the compiler reads it: a
    /// line continuation drops the break and the indent after it.
    fn first_literal(arg: &str) -> Option<String> {
        let start = arg.find('"')? + 1;
        let mut text = String::new();
        let mut chars = arg[start..].chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '"' => return Some(text),
                '\\' => match chars.next() {
                    Some('\n') => {
                        while chars.peek().is_some_and(|ch| ch.is_whitespace()) {
                            chars.next();
                        }
                    }
                    Some(other) => text.push(other),
                    None => {}
                },
                other => text.push(other),
            }
        }
        None
    }

    #[test]
    fn a_settings_note_is_one_short_line_in_either_language() {
        // What does not fit in `NOTE_LIMIT` characters goes into a help
        // icon's tooltip (`explained_setting`, `settings_note_help`,
        // `settings_heading_help`), not a longer note.
        let chinese: std::collections::HashMap<&str, &str> =
            crate::i18n_zh::ZH.iter().copied().collect();
        let note_at = [
            ("setting(", 1),
            ("stacked_setting(", 1),
            ("explained_setting(", 1),
            ("settings_note(", 0),
            ("settings_note_help(", 0),
        ];
        let mut checked = 0;
        let mut found = Vec::new();
        for (name, text) in settings_files() {
            for (call, at) in note_at {
                for args in call_args(&text, call) {
                    let Some(note) = args.get(at).and_then(|arg| first_literal(arg)) else {
                        continue;
                    };
                    checked += 1;
                    if note.chars().count() > super::NOTE_LIMIT {
                        found.push(format!("{name}: {note}"));
                    }
                    if let Some(zh) = chinese.get(note.as_str())
                        && zh.chars().count() > super::NOTE_LIMIT
                    {
                        found.push(format!("{name}: {zh}"));
                    }
                }
            }
        }
        assert!(checked > 40, "{checked}");
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn the_scanner_reads_a_calls_arguments_and_its_first_literal() {
        let source = "setting(tr(\"Name\"), tr(\"One, \\\n    two\"), div().w(px(3.)), cx); \
                      stacked_setting(a, b)";
        let calls = call_args(source, "setting(");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].len(), 4);
        assert_eq!(first_literal(&calls[0][1]).as_deref(), Some("One, two"));
    }
}
