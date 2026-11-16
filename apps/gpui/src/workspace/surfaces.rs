//! The shapes every page shares, from `docs/gpui-design-language.md`: the
//! two text roles above body size with their serif face, the card, and the
//! conversation's reading column. Views take these instead of choosing a
//! size, a face or a border of their own.
use gpui::{prelude::*, *};
use gpui_component::ActiveTheme;

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
    use super::{SERIF_FALLBACKS, rows_of};

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
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut folders = vec![root];
        let mut found = Vec::new();
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(folder).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                    continue;
                }
                let name = path.file_name().and_then(|name| name.to_str());
                if matches!(name, Some("surfaces.rs" | "terminal_view.rs")) {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
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
        }
        assert!(found.is_empty(), "{found:#?}");
    }
}
