//! Wording and small rules for favorites: passages, code and figures saved
//! from conversations into one library for every project. No graphics state.
use crate::i18n::tr;
use wisp_server::desktop::LibraryItem;

/// Which kind of saved item a list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Text,
    Code,
    Figures,
}

impl Filter {
    pub const ALL: [Filter; 4] = [Filter::All, Filter::Text, Filter::Code, Filter::Figures];

    pub fn label(self) -> &'static str {
        match self {
            Filter::All => tr("All"),
            Filter::Text => tr("Text"),
            Filter::Code => tr("Code"),
            Filter::Figures => tr("Figures"),
        }
    }

    /// The host's name for the kind; `None` asks for every kind.
    pub fn kind(self) -> Option<&'static str> {
        match self {
            Filter::All => None,
            Filter::Text => Some("text"),
            Filter::Code => Some("code"),
            Filter::Figures => Some("figure"),
        }
    }
}

/// What an item is, in a word or two.
pub fn kind_label(kind: &str, language: Option<&str>) -> String {
    match (
        kind,
        language
            .map(str::trim)
            .filter(|language| !language.is_empty()),
    ) {
        ("code", Some(language)) => format!("Code · {language}"),
        ("code", None) => "Code".into(),
        ("figure", _) => "Figure".into(),
        _ => "Text".into(),
    }
}

pub fn icon(kind: &str) -> &'static str {
    match kind {
        "code" => "code",
        "figure" => "image",
        _ => "doc",
    }
}

/// Where an item came from, as its names were when it was saved.
pub fn source_line(project: &str, conversation: &str) -> String {
    match (project.trim(), conversation.trim()) {
        ("", "") => String::new(),
        (project, "") => project.to_owned(),
        ("", conversation) => conversation.to_owned(),
        (project, conversation) => format!("{project} · {conversation}"),
    }
}

/// The saved item that holds exactly this passage, if the conversation has
/// one. A message is saved whole, so its text is its identity.
pub fn saved_id<'a>(items: &'a [LibraryItem], text: &str) -> Option<&'a str> {
    items
        .iter()
        .find(|item| item.kind == "text" && item.code == text)
        .map(|item| item.id.as_str())
}

/// Whether an item is words that can go into the message box. A figure is
/// not.
pub fn has_text(kind: &str) -> bool {
    kind != "figure"
}

/// An item's body as Markdown: prose as it is, code inside a fence that no
/// run of backticks in the code can close early.
pub fn as_markdown(kind: &str, language: Option<&str>, body: &str) -> String {
    if kind != "code" {
        return body.to_owned();
    }
    let longest = body
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    let fence = "`".repeat(longest.max(2) + 1);
    let language = language.unwrap_or_default().trim();
    format!(
        "{fence}{language}\n{}\n{fence}",
        body.trim_end_matches('\n')
    )
}

/// Whether a project file can be saved as a figure, by its name. The host
/// decides for good; this only hides the button where it cannot work.
pub fn is_image(path: &str) -> bool {
    let extension = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    path.contains('.')
        && matches!(
            extension.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp"
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, kind: &str, code: &str) -> LibraryItem {
        LibraryItem {
            id: id.into(),
            kind: kind.into(),
            title: code.lines().next().unwrap_or_default().into(),
            language: None,
            code: code.into(),
            content_type: None,
            source_project_id: "p".into(),
            source_project_name: "P".into(),
            source_session_id: "s".into(),
            source_session_title: "S".into(),
            source_path: None,
            created_at: 0,
        }
    }

    #[test]
    fn a_message_is_known_as_saved_by_its_exact_text() {
        let items = [
            item("code", "code", "the finding"),
            item("text", "text", "the finding"),
            item("other", "text", "something else"),
        ];
        assert_eq!(saved_id(&items, "the finding"), Some("text"));
        assert_eq!(saved_id(&items, "the finding "), None);
        assert_eq!(saved_id(&[], "the finding"), None);
    }

    #[test]
    fn items_are_labelled_by_kind_and_source() {
        assert_eq!(kind_label("code", Some(" python ")), "Code · python");
        assert_eq!(kind_label("code", Some(" ")), "Code");
        assert_eq!(kind_label("figure", Some("r")), "Figure");
        assert_eq!(kind_label("text", None), "Text");
        assert_eq!(source_line("Lab", "Kinase screen"), "Lab · Kinase screen");
        assert_eq!(source_line("Lab", " "), "Lab");
        assert_eq!(source_line("", ""), "");
        assert_eq!(Filter::Figures.kind(), Some("figure"));
        assert_eq!(Filter::All.kind(), None);
        assert!(has_text("code") && has_text("text") && !has_text("figure"));
    }

    #[test]
    fn code_is_fenced_past_any_backticks_it_contains() {
        assert_eq!(as_markdown("text", None, "**bold**"), "**bold**");
        assert_eq!(
            as_markdown("code", Some("python"), "print(1)\n"),
            "```python\nprint(1)\n```"
        );
        let nested = "```r\nplot(x)\n```";
        let fenced = as_markdown("code", None, nested);
        assert!(fenced.starts_with("````\n") && fenced.ends_with("\n````"));
        assert!(fenced.contains(nested));
    }

    #[test]
    fn only_image_files_are_offered_as_figures() {
        for path in ["figs/plot.png", "a.JPG", "x.y.webp", "logo.svg"] {
            assert!(is_image(path), "{path}");
        }
        for path in ["notes.txt", "png", "figs/plot", "data.csv", ""] {
            assert!(!is_image(path), "{path}");
        }
    }
}
