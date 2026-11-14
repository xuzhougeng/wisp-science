//! Project file browser state. Every request carries a ticket; a response is
//! applied only while its ticket is still the latest one for the same project,
//! so rapid navigation and project switches never show another folder's rows.
//! Paths are project-relative with `/` separators on every platform, which is
//! the only form the host accepts.
use crate::i18n::tr;
use wisp_dto::{
    DirEntry, FileContent,
    native_files::{Directory, Preview, SCHEMA},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    generation: u64,
    pub project: String,
    pub path: String,
}

#[derive(Default)]
pub struct FilesState {
    generation: u64,
    project: Option<String>,
    /// Directory currently listed; empty for the project root.
    pub path: String,
    pub entries: Vec<DirEntry>,
    pub loaded: bool,
    pub loading: bool,
    /// File being previewed, known before its content arrives.
    pub preview_path: Option<String>,
    pub preview: Option<FileContent>,
    pub error: Option<String>,
}

impl FilesState {
    /// Forgets the listing when the project changes. Switching conversations
    /// inside one project keeps the folder the user was looking at.
    pub fn set_project(&mut self, project: Option<&str>) {
        if self.project.as_deref() == project {
            return;
        }
        *self = Self {
            generation: self.generation + 1,
            project: project.map(str::to_owned),
            ..Self::default()
        };
    }

    /// Starts a directory or preview request for `path`, superseding any
    /// request still in flight. `None` until a project is selected.
    pub fn begin(&mut self, path: &str) -> Option<Ticket> {
        let project = self.project.clone()?;
        self.generation += 1;
        self.loading = true;
        self.error = None;
        Some(Ticket {
            generation: self.generation,
            project,
            path: path.to_owned(),
        })
    }

    /// Starts a preview request, replacing any preview already shown.
    pub fn begin_preview(&mut self, path: &str) -> Option<Ticket> {
        let ticket = self.begin(path)?;
        self.preview = None;
        self.preview_path = Some(path.to_owned());
        Some(ticket)
    }

    fn current(&self, ticket: &Ticket) -> bool {
        ticket.generation == self.generation && Some(&ticket.project) == self.project.as_ref()
    }

    pub fn accept_directory(&mut self, ticket: &Ticket, directory: Directory) -> bool {
        if !self.current(ticket)
            || directory.schema != SCHEMA
            || directory.project_id != ticket.project
            || directory.path != ticket.path
        {
            return false;
        }
        self.loading = false;
        self.loaded = true;
        self.path = directory.path;
        self.entries = directory.entries;
        true
    }

    pub fn accept_preview(&mut self, ticket: &Ticket, preview: Preview) -> bool {
        if !self.current(ticket)
            || preview.schema != SCHEMA
            || preview.project_id != ticket.project
            || preview.requested_path != ticket.path
        {
            return false;
        }
        self.loading = false;
        self.preview = Some(preview.content);
        true
    }

    pub fn fail(&mut self, ticket: &Ticket, error: String) -> bool {
        if !self.current(ticket) {
            return false;
        }
        self.loading = false;
        self.error = Some(error);
        true
    }

    /// Points the browser at `path` before its listing is loaded, for a
    /// file opened from outside the panel. Anything in flight is dropped.
    pub fn look_in(&mut self, path: &str) {
        self.generation += 1;
        self.path = path.to_owned();
        self.entries.clear();
        self.loaded = false;
        self.loading = false;
        self.error = None;
    }

    /// Returns to the listing and drops a preview response still in flight.
    pub fn close_preview(&mut self) {
        self.generation += 1;
        self.loading = false;
        self.preview = None;
        self.preview_path = None;
    }
}

/// What the name field in the Files panel is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Naming {
    /// A new folder in the folder that is listed.
    Folder,
    /// A new, empty file in the folder that is listed.
    File,
    /// A new name for the entry at this path.
    Rename(String),
}

impl Naming {
    pub fn title(&self) -> &'static str {
        match self {
            Naming::Folder => tr("Name the new folder"),
            Naming::File => tr("Name the new file"),
            Naming::Rename(_) => tr("New name"),
        }
    }

    /// What the field holds when it opens: the name being changed.
    pub fn start(&self) -> &str {
        match self {
            Naming::Rename(path) => file_name(path),
            Naming::Folder | Naming::File => "",
        }
    }
}

/// What is said after an entry was moved to the project's trash, so the
/// user knows it is not erased and where to find it. `to` is where it
/// went, relative to the project.
pub fn trashed_note(to: &str) -> String {
    let folder = parent(to).unwrap_or_default();
    format!(
        "{} is in {folder} in the project folder. Nothing was erased.",
        file_name(to)
    )
}

pub fn join(directory: &str, name: &str) -> String {
    if directory.is_empty() {
        name.to_owned()
    } else {
        format!("{directory}/{name}")
    }
}

/// Parent of a project-relative path; `None` at the project root.
pub fn parent(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    Some(
        path.rsplit_once('/')
            .map(|(parent, _)| parent.to_owned())
            .unwrap_or_default(),
    )
}

pub fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

pub fn is_markdown(path: &str) -> bool {
    let name = file_name(path).to_ascii_lowercase();
    name.ends_with(".md") || name.ends_with(".markdown")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewMode {
    /// The host found no text to show.
    Binary,
    Markdown,
    /// Verbatim text in the read-only viewer.
    Text,
}

/// Rendered Markdown is parsed in full, so very large documents stay verbatim.
const MARKDOWN_LIMIT: usize = 256 * 1024;

pub fn preview_mode(content: &FileContent) -> PreviewMode {
    match &content.text {
        None => PreviewMode::Binary,
        Some(text) if is_markdown(&content.path) && text.len() <= MARKDOWN_LIMIT => {
            PreviewMode::Markdown
        }
        Some(_) => PreviewMode::Text,
    }
}

/// One line under the preview title: size, and whether the host cut the file.
pub fn preview_note(content: &FileContent) -> String {
    let size = content.total_bytes.map(size_label);
    match (size, content.truncated) {
        (Some(size), true) => format!("{size} · showing the first 1 MB"),
        (Some(size), false) => size,
        (None, true) => "Showing the first 1 MB".into(),
        (None, false) => String::new(),
    }
}

pub fn size_label(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// A folder of an SSH host that the Files tab is looking at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBrowse {
    pub alias: String,
    /// The folder that was asked for; empty is the home folder.
    pub path: String,
    pub listing: Option<wisp_server::desktop::RemoteDirectory>,
    pub error: Option<String>,
}

/// The folder above `path` on a host, or none at the top of its disk. The
/// path is the host's own real one, as a listing reports it.
pub fn remote_parent(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    let (above, _) = path.rsplit_once('/')?;
    Some(if above.is_empty() {
        "/".into()
    } else {
        above.into()
    })
}

/// A folder inside `path` on a host.
pub fn remote_child(path: &str, name: &str) -> String {
    format!("{}/{name}", path.trim_end_matches('/'))
}

/// Where a file came from, in a line: the tool and the conversation, or
/// that nothing is known.
pub fn origin_line(origin: Option<&wisp_server::desktop::FileOrigin>) -> String {
    match origin {
        None => {
            crate::i18n::tr("No conversation of this project is known to have written this file.")
                .to_owned()
        }
        Some(origin) => {
            let title = match origin.title.trim() {
                "" => crate::i18n::tr("a conversation"),
                title => title,
            };
            crate::i18n::fill(
                crate::i18n::tr("Written by {} in “{}”."),
                &[&origin.tool, &title],
            )
        }
    }
}

/// What the call ran and what it read, as Markdown; nothing when the call
/// kept neither.
pub fn origin_detail(origin: &wisp_server::desktop::FileOrigin) -> String {
    let mut parts = Vec::new();
    if !origin.source.trim().is_empty() {
        parts.push(crate::present::fenced(&origin.source, &origin.language));
    }
    if !origin.read.is_empty() {
        let read = origin
            .read
            .iter()
            .map(|path| format!("- `{path}`"))
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("{}\n\n{read}", crate::i18n::tr("It read:")));
    }
    parts.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hosts_folders_are_walked_by_their_real_paths() {
        assert_eq!(
            remote_parent("/home/lab/data").as_deref(),
            Some("/home/lab")
        );
        assert_eq!(remote_parent("/home/").as_deref(), Some("/"));
        assert_eq!(remote_parent("/home").as_deref(), Some("/"));
        // The top of the disk has nothing above it, and neither has a
        // folder that has not been listed yet.
        assert_eq!(remote_parent("/"), None);
        assert_eq!(remote_parent(""), None);
        assert_eq!(remote_child("/home/lab", "data"), "/home/lab/data");
        assert_eq!(remote_child("/", "scratch"), "/scratch");
        // A name is taken as it is, whatever is in it.
        assert_eq!(remote_child("/data/", "run 1; x"), "/data/run 1; x");
    }

    #[test]
    fn a_files_origin_names_the_call_and_shows_what_it_ran() {
        let origin = wisp_server::desktop::FileOrigin {
            session: "s1".into(),
            title: " Count the reads ".into(),
            tool: "python".into(),
            language: "python".into(),
            source: "df.to_csv('counts.csv')".into(),
            read: vec!["data/reads.fq".into(), "config.yml".into()],
        };
        assert_eq!(
            origin_line(Some(&origin)),
            "Written by python in “Count the reads”."
        );
        assert_eq!(
            origin_detail(&origin),
            "```python\ndf.to_csv('counts.csv')\n```\n\nIt read:\n\n- `data/reads.fq`\n- `config.yml`"
        );
        // A conversation without a title, and a call that kept nothing.
        let bare = wisp_server::desktop::FileOrigin {
            title: String::new(),
            source: "  ".into(),
            read: vec![],
            ..origin
        };
        assert_eq!(
            origin_line(Some(&bare)),
            "Written by python in “a conversation”."
        );
        assert_eq!(origin_detail(&bare), "");
        assert!(origin_line(None).starts_with("No conversation of this project"));
    }

    fn directory(project: &str, path: &str, names: &[&str]) -> Directory {
        Directory {
            schema: SCHEMA.into(),
            project_id: project.into(),
            session_id: Some("s".into()),
            context_id: "local".into(),
            path: path.into(),
            entries: names
                .iter()
                .map(|name| DirEntry {
                    name: (*name).into(),
                    is_dir: false,
                    size: 1,
                    modified_unix_millis: None,
                })
                .collect(),
        }
    }

    fn preview(project: &str, path: &str, text: &str) -> Preview {
        Preview {
            schema: SCHEMA.into(),
            project_id: project.into(),
            session_id: Some("s".into()),
            context_id: "local".into(),
            requested_path: path.into(),
            content: FileContent {
                path: path.into(),
                mime: "text/plain".into(),
                text: Some(text.into()),
                base64: None,
                truncated: false,
                total_bytes: Some(text.len() as u64),
            },
        }
    }

    #[test]
    fn no_request_starts_without_a_project() {
        let mut files = FilesState::default();
        assert!(files.begin("").is_none());
        assert!(!files.loading);
    }

    #[test]
    fn rapid_navigation_keeps_only_the_latest_directory() {
        let mut files = FilesState::default();
        files.set_project(Some("p"));
        let root = files.begin("").unwrap();
        let data = files.begin("data").unwrap();
        assert!(!files.accept_directory(&root, directory("p", "", &["old.txt"])));
        assert!(files.loading);
        assert!(!files.accept_directory(&data, directory("p", "other", &["x"])));
        assert!(!files.accept_directory(&data, directory("q", "data", &["x"])));
        let mut wrong_schema = directory("p", "data", &["x"]);
        wrong_schema.schema = "wisp.native-files.v0".into();
        assert!(!files.accept_directory(&data, wrong_schema));
        assert!(files.accept_directory(&data, directory("p", "data", &["结果.csv"])));
        assert!(files.loaded && !files.loading);
        assert_eq!(files.path, "data");
        assert_eq!(files.entries[0].name, "结果.csv");
    }

    #[test]
    fn project_switch_rejects_the_previous_project_and_clears_the_listing() {
        let mut files = FilesState::default();
        files.set_project(Some("p"));
        let ticket = files.begin("").unwrap();
        assert!(files.accept_directory(&ticket, directory("p", "", &["a"])));
        let stale = files.begin("data").unwrap();
        files.set_project(Some("q"));
        assert!(files.entries.is_empty() && !files.loaded && files.path.is_empty());
        assert!(!files.accept_directory(&stale, directory("p", "data", &["a"])));
        assert!(!files.fail(&stale, "late".into()));
        assert!(files.error.is_none());
        // Returning to the first project never revives its old ticket either.
        files.set_project(Some("p"));
        assert!(!files.accept_directory(&stale, directory("p", "data", &["a"])));
        // Selecting another conversation in the same project keeps the folder.
        let ticket = files.begin("").unwrap();
        assert!(files.accept_directory(&ticket, directory("p", "", &["a"])));
        files.set_project(Some("p"));
        assert!(files.loaded);
    }

    #[test]
    fn revealing_a_file_points_the_browser_at_its_folder() {
        let mut files = FilesState::default();
        files.set_project(Some("p"));
        let stale = files.begin("").unwrap();
        files.look_in("data/raw");
        // The listing that was in flight belongs to another folder.
        assert!(!files.accept_directory(&stale, directory("p", "", &["a"])));
        assert_eq!(files.path, "data/raw");
        assert!(!files.loaded && !files.loading && files.entries.is_empty());
        let ticket = files.begin_preview("data/raw/reads.fastq").unwrap();
        assert!(files.accept_preview(&ticket, preview("p", "data/raw/reads.fastq", "@r1")));
        // Closing the preview leaves that folder to be listed, not the root.
        files.close_preview();
        assert_eq!(files.path, "data/raw");
        assert!(!files.loaded);
    }

    #[test]
    fn closing_a_preview_drops_its_late_response() {
        let mut files = FilesState::default();
        files.set_project(Some("p"));
        let ticket = files.begin_preview("notes.md").unwrap();
        assert_eq!(files.preview_path.as_deref(), Some("notes.md"));
        files.close_preview();
        assert!(!files.loading && files.preview_path.is_none());
        assert!(!files.accept_preview(&ticket, preview("p", "notes.md", "late")));
        assert!(files.preview.is_none());
        let ticket = files.begin_preview("notes.md").unwrap();
        assert!(!files.accept_preview(&ticket, preview("p", "other.md", "x")));
        assert!(files.accept_preview(&ticket, preview("p", "notes.md", "# 笔记")));
        // Opening another file never leaves the previous content on screen.
        files.begin_preview("data/table.csv").unwrap();
        assert!(files.preview.is_none());
        let ticket = files.begin_preview("notes.md").unwrap();
        assert!(files.accept_preview(&ticket, preview("p", "notes.md", "# 笔记")));
        assert_eq!(
            files.preview.as_ref().unwrap().text.as_deref(),
            Some("# 笔记")
        );
    }

    #[test]
    fn failures_surface_only_for_the_current_request() {
        let mut files = FilesState::default();
        files.set_project(Some("p"));
        let old = files.begin("a").unwrap();
        let new = files.begin("b").unwrap();
        assert!(!files.fail(&old, "old".into()));
        assert!(files.fail(&new, "Directory is too large".into()));
        assert_eq!(files.error.as_deref(), Some("Directory is too large"));
        assert!(!files.loading);
        files.begin("b").unwrap();
        assert!(files.error.is_none());
    }

    #[test]
    fn paths_stay_project_relative_with_forward_slashes() {
        assert_eq!(join("", "data"), "data");
        assert_eq!(join("data/raw", "reads.fastq"), "data/raw/reads.fastq");
        assert_eq!(parent(""), None);
        assert_eq!(parent("data"), Some(String::new()));
        assert_eq!(parent("data/raw/reads.fastq"), Some("data/raw".into()));
        assert_eq!(file_name("data/raw/reads.fastq"), "reads.fastq");
        assert_eq!(file_name("README.md"), "README.md");
        assert!(is_markdown("docs/README.MD"));
        assert!(is_markdown("notes.markdown"));
        assert!(!is_markdown("md/table.csv"));
    }

    #[test]
    fn preview_mode_follows_content_and_size() {
        let mut content = preview("p", "notes.md", "# Title").content;
        assert_eq!(preview_mode(&content), PreviewMode::Markdown);
        assert_eq!(preview_note(&content), "7 B");
        content.text = Some("x".repeat(MARKDOWN_LIMIT + 1));
        assert_eq!(preview_mode(&content), PreviewMode::Text);
        content.path = "table.csv".into();
        content.text = Some("a,b".into());
        content.truncated = true;
        content.total_bytes = Some(3 * 1024 * 1024);
        assert_eq!(preview_mode(&content), PreviewMode::Text);
        assert_eq!(preview_note(&content), "3.0 MB · showing the first 1 MB");
        content.text = None;
        content.truncated = false;
        content.total_bytes = None;
        assert_eq!(preview_mode(&content), PreviewMode::Binary);
        assert_eq!(preview_note(&content), "");
    }

    #[test]
    fn sizes_use_binary_units() {
        assert_eq!(size_label(0), "0 B");
        assert_eq!(size_label(1023), "1023 B");
        assert_eq!(size_label(1024), "1.0 KB");
        assert_eq!(size_label(1536), "1.5 KB");
        assert_eq!(size_label(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(size_label(u64::MAX), "16777216.0 TB");
    }

    #[test]
    fn the_name_field_starts_with_what_is_renamed_and_a_deletion_says_where_it_went() {
        assert_eq!(Naming::Folder.start(), "");
        assert_eq!(Naming::File.start(), "");
        assert_eq!(
            Naming::Rename("data/raw/counts.csv".into()).start(),
            "counts.csv"
        );
        assert_eq!(Naming::Rename("counts.csv".into()).start(), "counts.csv");
        for naming in [Naming::Folder, Naming::File, Naming::Rename("a".into())] {
            assert!(!naming.title().is_empty());
        }
        assert_eq!(
            trashed_note(".wisp/trash/1700-0/counts.csv"),
            "counts.csv is in .wisp/trash/1700-0 in the project folder. Nothing was erased."
        );
    }
}
