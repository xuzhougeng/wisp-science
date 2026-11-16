//! The shapes every page shares, from `docs/gpui-design-language.md`: the
//! two text roles above body size with their serif face, the card, and the
//! conversation's reading column, and the switch of an on/off setting.
//! Views take these instead of choosing a size, a face, a border or a
//! control of their own.
use std::rc::Rc;

use gpui::{prelude::*, *};
use gpui_component::{ActiveTheme, Disableable, switch::Switch, tooltip::Tooltip};

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
    use super::{SERIF_FALLBACKS, rows_of, switch};

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
        // fifth role. The terminal's grid has a size of its own.
        let mut found = Vec::new();
        for (path, text) in sources() {
            let name = path.file_name().and_then(|name| name.to_str());
            if matches!(name, Some("surfaces.rs" | "terminal_view.rs")) {
                continue;
            }
            for other in [
                ".text_base()",
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
