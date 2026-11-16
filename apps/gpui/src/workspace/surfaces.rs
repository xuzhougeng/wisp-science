//! The shapes every page shares, from `docs/gpui-design-language.md`: the
//! two text roles above body size with their serif face, the card, the
//! dialog, the conversation's reading column with the bars above its
//! composer, and the switch of an on/off setting.
//! Views take these instead of choosing a size, a face, a border or a
//! control of their own.
use std::rc::Rc;

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, StyledExt, button::Button, switch::Switch, tooltip::Tooltip,
};

use super::shell::icon_button;

/// The serif face of titles.
const SERIF: &str = "Georgia";

/// What a title falls back to for the characters Georgia does not have:
/// each system's Chinese serif, so a title is serif in either language.
const SERIF_FALLBACKS: [&str; 4] = [
    "Songti SC",
    "Source Han Serif SC",
    "Noto Serif CJK SC",
    "SimSun",
];

fn serif() -> Font {
    let mut face = font(SERIF);
    face.weight = FontWeight::SEMIBOLD;
    face.fallbacks = Some(FontFallbacks::from_fonts(
        SERIF_FALLBACKS
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
    ));
    face
}

/// The one large line a page may have: the home greeting, the question an
/// empty conversation asks.
pub(super) fn display(text: impl Into<SharedString>) -> Div {
    div()
        .font(serif())
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
    use super::{DialogWidth, READING_WIDTH, SERIF_FALLBACKS, dialog, rows_of, switch};

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
            ("workspace/settings_hooks.rs", "render_hook_failures"),
            ("workspace/memory_view.rs", "render_memory_proposal"),
            ("workspace/follow_ups_view.rs", "render_follow_ups"),
            ("workspace/transfers_view.rs", "render_transfers"),
            ("workspace/queue_view.rs", "queue_strip"),
            ("workspace/runtime_console_view.rs", "render_console"),
            ("workspace/slash_view.rs", "timer_strip"),
            ("workspace/shell.rs", "read_only_card"),
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
            "render_hook_failures",
            "render_memory_proposal",
            "render_follow_ups",
            "render_transfers",
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
            ("workspace/organise_view.rs", "popup"),
            ("workspace/demos_view.rs", "example_copy_menu"),
            ("workspace/terminal_view.rs", "terminal_hosts_menu"),
            ("workspace/palette_view.rs", "render_palette"),
            ("workspace/notices_view.rs", "render_session_notices"),
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
            ("workspace/explore_view.rs", "render_exploration_preview"),
            ("workspace/explore_view.rs", "render_start_exploration"),
            ("workspace/project_settings.rs", "render_project_settings"),
            ("workspace/project_sync_view.rs", "render_join_synced"),
            ("workspace/trajectory_view.rs", "render_trajectory"),
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

    #[test]
    fn titles_have_a_chinese_serif_on_each_system() {
        // macOS ships Songti SC, Windows SimSun, Linux a Noto or Source Han
        // serif when one is installed.
        for face in ["Songti SC", "SimSun", "Noto Serif CJK SC"] {
            assert!(SERIF_FALLBACKS.contains(&face), "{face}");
        }
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
}
