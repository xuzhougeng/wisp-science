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

/// The order a folder's entries are listed in. Folders always come first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileSort {
    #[default]
    Name,
    Newest,
    Largest,
}

impl FileSort {
    pub const ALL: [FileSort; 3] = [FileSort::Name, FileSort::Newest, FileSort::Largest];

    /// The one after this, for a button that steps through them.
    pub fn next(self) -> Self {
        match self {
            FileSort::Name => FileSort::Newest,
            FileSort::Newest => FileSort::Largest,
            FileSort::Largest => FileSort::Name,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FileSort::Name => tr("By name"),
            FileSort::Newest => tr("Newest first"),
            FileSort::Largest => tr("Largest first"),
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            FileSort::Name => "name",
            FileSort::Newest => "newest",
            FileSort::Largest => "largest",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|sort| sort.key() == key.trim())
            .unwrap_or_default()
    }
}

/// Puts `entries` in `sort`'s order: folders first, then by the chosen
/// measure, ties by name without regard to case.
pub fn sort_entries(entries: &mut [DirEntry], sort: FileSort) {
    use std::cmp::Ordering;
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| match sort {
                FileSort::Name => Ordering::Equal,
                FileSort::Newest => b.modified_unix_millis.cmp(&a.modified_unix_millis),
                // A folder's size is not its contents'; folders stay by name.
                FileSort::Largest if a.is_dir => Ordering::Equal,
                FileSort::Largest => b.size.cmp(&a.size),
            })
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
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
    /// How folders are listed; kept when the project changes.
    pub sort: FileSort,
    /// Files found by name across the project, while a name is typed.
    pub search: NameSearch,
    /// The folder is shown as tiles, pictures as small copies of
    /// themselves, instead of rows.
    pub grid: bool,
}

/// What an upload did, in a line: how many files were copied in, and the
/// first that was not, with why.
pub fn upload_note(results: &[wisp_dto::LocalFileUploadResult]) -> String {
    let copied = results
        .iter()
        .filter(|result| result.path.is_some())
        .count();
    let mut note = crate::i18n::fill(
        tr("Copied {} of {} files here."),
        &[&copied, &results.len()],
    );
    if let Some(failed) = results.iter().find(|result| result.path.is_none()) {
        let name = std::path::Path::new(&failed.source)
            .file_name()
            .map_or(failed.source.clone(), |name| {
                name.to_string_lossy().into_owned()
            });
        note.push(' ');
        note.push_str(&crate::i18n::fill(
            tr("{}: {}"),
            &[&name, &failed.error.clone().unwrap_or_default()],
        ));
    }
    note
}

/// Files found by name across the project. Answers come back out of
/// order, so each search carries a ticket and only the latest one counts.
#[derive(Default)]
pub struct NameSearch {
    ticket: u64,
    /// What is being looked for; empty when nothing is.
    pub query: String,
    /// The paths found for it, once they are back.
    pub hits: Option<Vec<String>>,
}

/// Files listed for one search at most.
pub const NAME_HITS: usize = 200;

impl NameSearch {
    /// Starts looking for `typed`. Returns the ticket and the name to look
    /// for, or nothing when the field was emptied, which ends the search.
    pub fn begin(&mut self, typed: &str) -> Option<(u64, String)> {
        self.ticket += 1;
        self.query = typed.trim().to_owned();
        self.hits = None;
        (!self.query.is_empty()).then(|| (self.ticket, self.query.clone()))
    }

    /// Takes the paths found by search `ticket`, if it is still the latest.
    pub fn accept(&mut self, ticket: u64, hits: Vec<String>) -> bool {
        if ticket != self.ticket || self.query.is_empty() {
            return false;
        }
        self.hits = Some(hits);
        true
    }

    /// Whether a search is showing in place of the folder.
    pub fn active(&self) -> bool {
        !self.query.is_empty()
    }
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
            sort: self.sort,
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
        sort_entries(&mut self.entries, self.sort);
        true
    }

    /// Lists the folder in another order, at once.
    pub fn set_sort(&mut self, sort: FileSort) {
        self.sort = sort;
        sort_entries(&mut self.entries, sort);
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
    crate::i18n::fill(
        tr("{} is in {} in the project folder. Nothing was erased."),
        &[&file_name(to), &folder],
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

/// A document the host can turn into text to read: PDF, Word, Excel,
/// PowerPoint, OpenDocument, RTF and EPUB.
pub fn is_document(path: &str) -> bool {
    const KINDS: &[&str] = &[
        "pdf", "doc", "docx", "docm", "odt", "rtf", "epub", "ppt", "pps", "pot", "pptx", "pptm",
        "ppsx", "ppsm", "xls", "xlsx", "xlsm", "xlsb", "ods", "odp",
    ];
    file_name(path)
        .rsplit_once('.')
        .is_some_and(|(_, kind)| KINDS.contains(&kind.to_ascii_lowercase().as_str()))
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

pub fn is_html(path: &str) -> bool {
    let name = file_name(path).to_ascii_lowercase();
    [".html", ".htm", ".xhtml"]
        .iter()
        .any(|kind| name.ends_with(kind))
}

/// Whether a file is shown as a page: an HTML file small enough to lay out
/// in full (a larger one is shown as its source).
pub fn html_page(content: &FileContent) -> bool {
    is_html(&content.path)
        && content
            .text
            .as_ref()
            .is_some_and(|text| text.len() <= MARKDOWN_LIMIT)
}

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
        (Some(size), true) => crate::i18n::fill(tr("{} · showing the first 1 MB"), &[&size]),
        (Some(size), false) => size,
        (None, true) => tr("Showing the first 1 MB").into(),
        (None, false) => String::new(),
    }
}

/// How many files, in words: 1 file, 3 files.
pub fn count(files: usize) -> String {
    match files {
        1 => tr("1 file").into(),
        n => crate::i18n::fill(tr("{} files"), &[&n]),
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
    /// A file of the folder being looked at, in place of the listing.
    pub look: Option<RemoteLook>,
}

/// A file of a host being looked at in the Files tab. Its bytes are only
/// read, never kept: a picture lives in the window while it is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteLook {
    pub alias: String,
    pub path: String,
    /// Its size, as the listing said and then as the host answered.
    pub size: Option<u64>,
    /// What can be shown of it, once the host has answered.
    pub shown: Option<Result<RemoteShown, String>>,
}

/// What a file of a host is shown as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteShown {
    /// Text, cut to its first megabyte when `truncated`.
    Text { text: String, truncated: bool },
    /// A picture; the window holds it.
    Picture,
    /// A picture larger than a preview fetches, which was not fetched.
    TooLarge(u64),
    /// Not text and not a picture.
    NotShown,
}

impl RemoteLook {
    pub fn new(alias: String, path: String, listed: Option<u64>) -> Self {
        Self {
            alias,
            path,
            size: listed,
            shown: None,
        }
    }

    /// Takes what the host answered; a picture's bytes stay with the
    /// caller.
    pub fn answered(&mut self, file: &wisp_server::desktop::RemoteFile) {
        use wisp_server::desktop::RemoteFileBody;
        self.size = file.size.or(self.size);
        self.shown = Some(Ok(match &file.body {
            RemoteFileBody::Text { text, truncated } => RemoteShown::Text {
                text: text.clone(),
                truncated: *truncated,
            },
            RemoteFileBody::Picture { .. } => RemoteShown::Picture,
            RemoteFileBody::TooLarge { limit } => RemoteShown::TooLarge(*limit),
            RemoteFileBody::NotShown => RemoteShown::NotShown,
        }));
    }

    /// The line under its name: its size, and that only its head is
    /// shown when it was cut.
    pub fn note(&self) -> String {
        let cut = matches!(
            self.shown,
            Some(Ok(RemoteShown::Text {
                truncated: true,
                ..
            }))
        );
        match (self.size.map(size_label), cut) {
            (Some(size), true) => crate::i18n::fill(tr("{} · showing the first 1 MB"), &[&size]),
            (Some(size), false) => size,
            (None, true) => tr("Showing the first 1 MB").into(),
            (None, false) => String::new(),
        }
    }

    /// What is said in place of the file when it is not drawn.
    pub fn message(&self) -> Option<String> {
        match self.shown.as_ref()? {
            Ok(RemoteShown::TooLarge(limit)) => Some(crate::i18n::fill(
                tr("This picture is larger than {}, so it was not fetched. Copy it to this computer to look at it."),
                &[&size_label(*limit)],
            )),
            Ok(RemoteShown::NotShown) => Some(
                tr("This kind of file is not shown from a host. Copy it to this computer to look at it.")
                    .into(),
            ),
            Ok(_) => None,
            Err(error) => Some(error.clone()),
        }
    }
}

/// Whether text of a file is laid out as Markdown: a Markdown file small
/// enough to parse in full, as a project file is.
pub fn shows_as_markdown(path: &str, text: &str) -> bool {
    is_markdown(path) && text.len() <= MARKDOWN_LIMIT
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

/// What the call ran, what it printed and what it read, as Markdown, as
/// the WebView's provenance of a file shows its Code, Log and Inputs;
/// nothing when the call kept none of them.
pub fn origin_detail(origin: &wisp_server::desktop::FileOrigin) -> String {
    let mut parts = Vec::new();
    if !origin.source.trim().is_empty() {
        parts.push(crate::present::fenced(&origin.source, &origin.language));
    }
    if !origin.output.trim().is_empty() {
        parts.push(format!(
            "{}\n\n{}",
            crate::i18n::tr("It printed:"),
            crate::present::fenced(origin.output.trim_end(), "text")
        ));
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

/// A delimited text file laid out as a table: the first rows and
/// columns, each cell bounded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Rows of the file that are not shown.
    pub more_rows: usize,
    /// Columns of the widest row that are not shown.
    pub more_columns: usize,
}

pub const TABLE_ROWS: usize = 200;
pub const TABLE_COLUMNS: usize = 30;
const TABLE_CELL: usize = 80;

/// One record of a delimited file, by the usual quoting rules: a quoted
/// field may hold the delimiter, a line break and a doubled quote.
fn records(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut records = vec![];
    let mut record = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' if quoted => quoted = false,
            '"' if field.is_empty() => quoted = true,
            ch if ch == delimiter && !quoted => record.push(std::mem::take(&mut field)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            ch => field.push(ch),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    // A blank line is not a record.
    records.retain(|record| record.len() > 1 || !record[0].trim().is_empty());
    records
}

/// A notebook output is cut here, so one long log does not bury the rest.
const NOTEBOOK_OUTPUT: usize = 4_000;

/// A Jupyter notebook (`.ipynb`) as Markdown to read: its text cells as
/// they are, each code cell fenced in the notebook's language with its run
/// number, and what the cell printed, returned or raised after it. Figures
/// are named, not drawn. `None` when the file is not a notebook that can be
/// read, so it is shown as text.
pub fn notebook(path: &str, text: &str) -> Option<String> {
    if !file_name(path).to_ascii_lowercase().ends_with(".ipynb") {
        return None;
    }
    let book: serde_json::Value = serde_json::from_str(text).ok()?;
    let cells = book.get("cells")?.as_array()?;
    // Text kept as one string or as a list of lines.
    let joined = |value: Option<&serde_json::Value>| -> String {
        match value {
            Some(serde_json::Value::String(text)) => text.clone(),
            Some(serde_json::Value::Array(lines)) => lines
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<String>(),
            _ => String::new(),
        }
    };
    let clipped = |text: String| -> String {
        let text = text.trim_end().to_owned();
        if text.chars().count() <= NOTEBOOK_OUTPUT {
            return text;
        }
        let mut cut: String = text.chars().take(NOTEBOOK_OUTPUT).collect();
        cut.push_str("\n…");
        cut
    };
    // A fence longer than any run of backticks inside, so nothing closes it.
    let fenced = |text: &str, language: &str| -> String {
        let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat(longest.max(2) + 1);
        format!("{fence}{language}\n{text}\n{fence}")
    };
    let metadata = book.get("metadata");
    let language = metadata
        .and_then(|m| {
            m.pointer("/kernelspec/language")
                .or_else(|| m.pointer("/language_info/name"))
        })
        .and_then(serde_json::Value::as_str)
        .unwrap_or("python")
        .to_owned();
    let mut parts = Vec::new();
    for cell in cells {
        let source = joined(cell.get("source"));
        match cell.get("cell_type").and_then(serde_json::Value::as_str) {
            Some("markdown") => {
                if !source.trim().is_empty() {
                    parts.push(source.trim_end().to_owned());
                }
            }
            Some("code") => {
                let run = cell
                    .get("execution_count")
                    .and_then(serde_json::Value::as_u64)
                    .map(|count| format!("[{count}]"))
                    .unwrap_or_else(|| "[ ]".into());
                parts.push(format!(
                    "`{run}`\n\n{}",
                    fenced(source.trim_end(), &language)
                ));
                let outputs = cell
                    .get("outputs")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                for output in outputs {
                    let data = output.get("data");
                    let shown = match output
                        .get("output_type")
                        .and_then(serde_json::Value::as_str)
                    {
                        Some("stream") => Some(clipped(joined(output.get("text")))),
                        Some("error") => {
                            let name = joined(output.get("ename"));
                            let value = joined(output.get("evalue"));
                            Some(format!("{name}: {value}"))
                        }
                        _ if data.is_some_and(|data| {
                            data.get("image/png").is_some()
                                || data.get("image/jpeg").is_some()
                                || data.get("image/svg+xml").is_some()
                        }) =>
                        {
                            parts.push(format!("*{}*", tr("A figure is here.")));
                            None
                        }
                        _ => data
                            .map(|data| clipped(joined(data.get("text/plain"))))
                            .filter(|text| !text.is_empty()),
                    };
                    if let Some(shown) = shown.filter(|text| !text.trim().is_empty()) {
                        parts.push(fenced(&shown, "text"));
                    }
                }
            }
            Some(_) if !source.trim().is_empty() => parts.push(fenced(source.trim_end(), "text")),
            _ => {}
        }
    }
    Some(if parts.is_empty() {
        tr("This notebook has no cells.").to_owned()
    } else {
        parts.join("\n\n")
    })
}

/// The table in a `.csv` or `.tsv` file, when it has one: at least a
/// header and two columns. Anything else is shown as text.
pub fn table(path: &str, text: &str) -> Option<Table> {
    let name = file_name(path).to_ascii_lowercase();
    let delimiter = if name.ends_with(".csv") {
        ','
    } else if name.ends_with(".tsv") || name.ends_with(".tab") {
        '\t'
    } else {
        return None;
    };
    let mut all = records(text, delimiter);
    let widest = all.iter().map(Vec::len).max()?;
    if widest < 2 {
        return None;
    }
    let clip = |cell: &String| -> String {
        let cell = cell.trim();
        if cell.chars().count() <= TABLE_CELL {
            cell.to_owned()
        } else {
            let mut cut: String = cell.chars().take(TABLE_CELL).collect();
            cut.push('…');
            cut
        }
    };
    let shown = widest.min(TABLE_COLUMNS);
    let row = |record: &Vec<String>| -> Vec<String> {
        (0..shown)
            .map(|column| record.get(column).map(clip).unwrap_or_default())
            .collect()
    };
    let header = row(&all.remove(0));
    let more_rows = all.len().saturating_sub(TABLE_ROWS);
    Some(Table {
        header,
        rows: all.iter().take(TABLE_ROWS).map(row).collect(),
        more_rows,
        more_columns: widest - shown,
    })
}

/// What a table leaves out, in a line; nothing when it shows everything.
pub fn table_note(table: &Table) -> Option<String> {
    use crate::i18n::{fill, tr};
    match (table.more_rows, table.more_columns) {
        (0, 0) => None,
        (rows, 0) => Some(fill(tr("{} more rows are not shown."), &[&rows])),
        (0, columns) => Some(fill(tr("{} more columns are not shown."), &[&columns])),
        (rows, columns) => Some(fill(
            tr("{} more rows and {} more columns are not shown."),
            &[&rows, &columns],
        )),
    }
}

/// Where a project file is on disk: the project's folder joined with a
/// path of the project's own listing. A path that would leave the folder
/// is not one of those and gives nothing.
pub fn on_disk(root: &std::path::Path, relative: &str) -> Option<std::path::PathBuf> {
    use std::path::Component;
    let path = std::path::Path::new(relative);
    let inside = !relative.trim().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    inside.then(|| root.join(path))
}

/// At most this many entries are drawn as tiles; the list shows them all.
pub const GRID_LIMIT: usize = 400;

/// A picture larger than this is drawn as its kind, not as itself.
const THUMBNAIL_BYTES: u64 = 16 * 1024 * 1024;

/// Whether a tile shows the file itself: a picture the app can draw, not
/// too large to read for a tile.
pub fn thumbnail(name: &str, size: u64) -> bool {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp"
    ) && size <= THUMBNAIL_BYTES
}

// Editing a script where it is previewed, as the WebView's source editor
// (`RpCodeEditor` in `ui/src/app_support/previews.rs`) does: only R and
// Python sources, the languages a conversation has an interpreter for, and
// only one the preview loaded whole. Everything else stays read-only.

/// A script that can be edited and run in the preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Python,
    R,
}

impl Script {
    /// By its name, as the WebView tells them: `.py` and `.pyw` are Python,
    /// `.r` in either case is R.
    pub fn of(path: &str) -> Option<Self> {
        let (_, kind) = file_name(path).rsplit_once('.')?;
        match kind.to_ascii_lowercase().as_str() {
            "py" | "pyw" => Some(Self::Python),
            "r" => Some(Self::R),
            _ => None,
        }
    }

    pub fn runtime(self) -> wisp_server::desktop::RuntimeLanguage {
        match self {
            Self::Python => wisp_server::desktop::RuntimeLanguage::Python,
            Self::R => wisp_server::desktop::RuntimeLanguage::R,
        }
    }

    /// The language as the shared statement finder names it.
    pub fn statement_language(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::R => "r",
        }
    }
}

/// The most lines a script may have and still be edited; the WebView shows
/// a longer one read-only.
pub const EDIT_LINES: usize = 8_000;

/// Whether the previewed file opens in the editor: `None` for a file that
/// is not a script (read-only, with nothing to say), `Some(Ok)` for a
/// script to edit, `Some(Err)` with why a script is shown read-only. A
/// script the host cut short is never editable: saving the head would
/// lose the rest.
pub fn editable(content: &FileContent) -> Option<Result<Script, &'static str>> {
    let script = Script::of(&content.path)?;
    let Some(text) = &content.text else {
        return Some(Err(tr(
            "This file is not text, so it cannot be edited here.",
        )));
    };
    if content.truncated {
        return Some(Err(tr(
            "Only the first 1 MB is shown, so this script is read-only here.",
        )));
    }
    // The host reads a file as UTF-8 and puts a replacement character
    // where it could not; saving that would change bytes no one touched.
    if text.contains('\u{FFFD}') {
        return Some(Err(tr(
            "This script is not plain UTF-8 text, so it is read-only here.",
        )));
    }
    // As the WebView clips a preview: at the line limit's newline, when
    // anything follows it.
    let cut = text
        .match_indices('\n')
        .nth(EDIT_LINES - 1)
        .is_some_and(|(at, _)| at + 1 < text.len());
    if cut {
        return Some(Err(tr(
            "This script has more than 8,000 lines, so it is read-only here.",
        )));
    }
    Some(Ok(script))
}

/// The file's text as the editor shows it: every line ending a plain
/// newline.
pub fn editor_text(disk: &str) -> String {
    disk.replace("\r\n", "\n")
}

/// Whether the file ends its lines with CR LF: when most of its line
/// endings are.
fn crlf(disk: &str) -> bool {
    let crlf = disk.matches("\r\n").count();
    let lf = disk.matches('\n').count() - crlf;
    crlf > 0 && crlf >= lf
}

/// What is written for the editor's `edited` text over a file that held
/// `disk`: its lines end as the file's did (CR LF on a Windows-made
/// file), and a file that ended with a newline still does. A file of mixed
/// endings takes the more common one.
pub fn saved_text(disk: &str, edited: &str) -> String {
    let mut text = editor_text(edited);
    if disk.ends_with('\n') && !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if crlf(disk) {
        text = text.replace('\n', "\r\n");
    }
    text
}

/// Whether the editor holds anything the file does not: the editor's text
/// against the file's as the editor showed it.
pub fn is_dirty(disk: &str, edited: &str) -> bool {
    editor_text(disk) != edited
}

/// The script being edited in the preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editing {
    pub project: String,
    pub path: String,
    pub script: Script,
    /// The file's text when the edit began, exactly as on disk: what a
    /// save checks the file still holds.
    pub base: String,
    pub dirty: bool,
    pub saving: bool,
    /// The file on disk is no longer `base`: the assistant or another
    /// program changed it after the edit began.
    pub changed_on_disk: bool,
    /// Why the last save failed.
    pub problem: Option<String>,
}

/// Unsaved edits, per project and file, kept while the window is open:
/// the WebView keeps them per file the same way, so switching files or
/// closing the preview loses nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Drafts(std::collections::BTreeMap<(String, String), Draft>);

/// One file's unsaved edit and the text it began from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Draft {
    pub base: String,
    pub text: String,
}

impl Drafts {
    /// Keeps `text` as the draft of `path` while it differs from `base`;
    /// one edited back to the file's text is no draft.
    pub fn note(&mut self, project: &str, path: &str, base: &str, text: &str) {
        let key = (project.to_owned(), path.to_owned());
        if is_dirty(base, text) {
            self.0.insert(
                key,
                Draft {
                    base: base.to_owned(),
                    text: text.to_owned(),
                },
            );
        } else {
            self.0.remove(&key);
        }
    }

    pub fn get(&self, project: &str, path: &str) -> Option<&Draft> {
        self.0.get(&(project.to_owned(), path.to_owned()))
    }

    /// Forgets the draft of `path`: it was saved, or thrown away on purpose.
    pub fn forget(&mut self, project: &str, path: &str) {
        self.0.remove(&(project.to_owned(), path.to_owned()));
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The files with unsaved edits, by name, in order.
    pub fn names(&self) -> Vec<String> {
        self.0
            .keys()
            .map(|(_, path)| file_name(path).to_owned())
            .collect()
    }
}

/// What opening a file again finds of an edit left in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reopened {
    /// No draft: the editor shows the file.
    Clean,
    /// The draft goes back into the editor.
    Draft,
    /// The draft goes back into the editor, but the file changed on disk
    /// after the edit began; saving it would replace that change.
    ChangedOnDisk,
}

pub fn reopen(draft: Option<&Draft>, disk: &str) -> Reopened {
    match draft {
        None => Reopened::Clean,
        Some(draft) if draft.base == disk => Reopened::Draft,
        Some(_) => Reopened::ChangedOnDisk,
    }
}

/// What Run sends for the editor's text and its selection (UTF-16
/// offsets, as the input reports them): the selection when there is one,
/// otherwise the whole statement under the caret, as in the WebView (the
/// shared [`wisp_dto::source_run`]). With the statement comes where the
/// caret goes next, as a line and a column in characters, both from 0.
pub fn run_code(
    text: &str,
    script: Script,
    start_utf16: u32,
    end_utf16: u32,
) -> Option<(String, Option<(u32, u32)>)> {
    let run = wisp_dto::source_run::source_execution(
        text,
        start_utf16,
        end_utf16,
        script.statement_language(),
    )?;
    let next = run.next_caret_utf16.map(|caret| {
        let byte = wisp_dto::source_run::utf16_to_byte_index(text, caret);
        line_and_column(text, byte)
    });
    Some((run.code, next))
}

/// The line and the column in characters, both from 0, of byte `byte`.
fn line_and_column(text: &str, byte: usize) -> (u32, u32) {
    let before = &text[..byte.min(text.len())];
    let line = before.matches('\n').count();
    let start = before.rfind('\n').map_or(0, |newline| newline + 1);
    (line as u32, before[start..].chars().count() as u32)
}

/// How a whole-script run reads in the console, as the WebView echoes it.
pub fn script_echo(path: &str) -> String {
    format!("script {path}")
}

/// What the window says before it closes over unsaved edits.
pub fn unsaved_note(names: &[String]) -> String {
    match names {
        [one] => crate::i18n::fill(
            tr("{} has edits that are not saved. They are lost if the window closes."),
            &[one],
        ),
        many => crate::i18n::fill(
            tr("{} files have edits that are not saved ({}). They are lost if the window closes."),
            &[&many.len(), &many.join(", ")],
        ),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_tile_shows_pictures_it_can_draw_and_not_too_large() {
        assert!(thumbnail("volcano.PNG", 2_000));
        assert!(thumbnail("figure.svg", 10));
        assert!(!thumbnail("counts.csv", 10));
        assert!(!thumbnail("scan.tiff", 10));
        assert!(!thumbnail("noextension", 10));
        assert!(!thumbnail("huge.jpg", THUMBNAIL_BYTES + 1));
    }

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
    fn a_hosts_file_says_how_much_of_it_is_shown_and_why_not() {
        use wisp_server::desktop::{RemoteFile, RemoteFileBody};
        let answer = |size, body| RemoteFile {
            alias: "gpu".into(),
            path: "/data/a.csv".into(),
            size,
            body,
        };
        let mut look = RemoteLook::new("gpu".into(), "/data/a.csv".into(), Some(2048));
        // Before the host answers, the listing's size.
        assert_eq!(look.note(), "2.0 KB");
        assert_eq!(look.message(), None);
        look.answered(&answer(
            Some(5 * 1024 * 1024),
            RemoteFileBody::Text {
                text: "a,b\n".into(),
                truncated: true,
            },
        ));
        assert_eq!(look.note(), "5.0 MB · showing the first 1 MB");
        assert_eq!(look.message(), None);
        look.answered(&answer(
            Some(3),
            RemoteFileBody::Text {
                text: "a\n".into(),
                truncated: false,
            },
        ));
        assert_eq!(look.note(), "3 B");
        // A picture's bytes stay with the window, not in the look.
        look.answered(&answer(
            Some(4),
            RemoteFileBody::Picture {
                content_type: "image/png".into(),
                bytes: vec![1, 2, 3, 4],
            },
        ));
        assert_eq!(look.shown, Some(Ok(RemoteShown::Picture)));
        // An answer without a size keeps the listing's.
        let mut big = RemoteLook::new("gpu".into(), "/fig/big.png".into(), Some(40 << 20));
        big.answered(&answer(None, RemoteFileBody::TooLarge { limit: 32 << 20 }));
        assert_eq!(big.note(), "40.0 MB");
        assert!(big.message().unwrap().contains("larger than 32.0 MB"));
        big.answered(&answer(Some(9), RemoteFileBody::NotShown));
        assert!(big.message().unwrap().contains("Copy it to this computer"));
        big.shown = Some(Err("Remote file request failed (exit 66)".into()));
        assert_eq!(
            big.message().as_deref(),
            Some("Remote file request failed (exit 66)")
        );
        assert!(shows_as_markdown("/n/README.md", "# x"));
        assert!(!shows_as_markdown("/n/README.txt", "# x"));
        assert!(!shows_as_markdown(
            "/n/big.md",
            &"x".repeat(MARKDOWN_LIMIT + 1)
        ));
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
            output: String::new(),
        };
        assert_eq!(
            origin_line(Some(&origin)),
            "Written by python in “Count the reads”."
        );
        assert_eq!(
            origin_detail(&origin),
            "```python\ndf.to_csv('counts.csv')\n```\n\nIt read:\n\n- `data/reads.fq`\n- `config.yml`"
        );
        // What it printed, the WebView's Log, comes between.
        let printed = wisp_server::desktop::FileOrigin {
            output: "wrote 12 rows\n".into(),
            ..origin.clone()
        };
        assert_eq!(
            origin_detail(&printed),
            "```python\ndf.to_csv('counts.csv')\n```\n\nIt printed:\n\n```text\nwrote 12 rows\n```\n\nIt read:\n\n- `data/reads.fq`\n- `config.yml`"
        );
        // A conversation without a title, and a call that kept nothing.
        let bare = wisp_server::desktop::FileOrigin {
            title: String::new(),
            source: "  ".into(),
            read: vec![],
            output: " \n".into(),
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
    fn a_delimited_file_is_laid_out_as_a_table() {
        let text = "sample,reads,note\r\nS01,1200,\"clean, no adapters\"\nS02,  950 ,\"said \"\"odd\"\"\nand more\"\n\nS03,10\n";
        let found = table("results/qc.CSV", text).unwrap();
        assert_eq!(found.header, ["sample", "reads", "note"]);
        assert_eq!(
            found.rows,
            [
                vec!["S01", "1200", "clean, no adapters"],
                vec!["S02", "950", "said \"odd\"\nand more"],
                // A short row is padded, a blank line skipped.
                vec!["S03", "10", ""],
            ]
        );
        assert_eq!((found.more_rows, found.more_columns), (0, 0));
        assert_eq!(table_note(&found), None);

        let tabbed = table("counts.tsv", "gene\tcount\nrecA\t7\n").unwrap();
        assert_eq!(tabbed.rows, [vec!["recA", "7"]]);
        // Not a table: another kind of file, one column, nothing at all.
        assert_eq!(table("notes.txt", "a,b\n1,2\n"), None);
        assert_eq!(table("list.csv", "only\nvalues\n"), None);
        assert_eq!(table("empty.csv", ""), None);
    }

    #[test]
    fn a_big_table_shows_its_first_rows_and_columns_and_says_so() {
        let wide: Vec<String> = (0..TABLE_COLUMNS + 5).map(|n| format!("c{n}")).collect();
        let mut text = wide.join(",");
        for row in 0..TABLE_ROWS + 12 {
            text.push_str(&format!("\n{row},{}", "x".repeat(200)));
        }
        let found = table("big.csv", &text).unwrap();
        assert_eq!(found.header.len(), TABLE_COLUMNS);
        assert_eq!(found.rows.len(), TABLE_ROWS);
        assert_eq!((found.more_rows, found.more_columns), (12, 5));
        assert_eq!(found.rows[0][1].chars().count(), 81);
        assert_eq!(
            table_note(&found).unwrap(),
            "12 more rows and 5 more columns are not shown."
        );
    }

    #[test]
    fn a_project_file_is_found_on_disk_and_nothing_outside_the_project() {
        let root = std::path::Path::new("/data/lab");
        assert_eq!(
            on_disk(root, "results/fit.csv"),
            Some(std::path::PathBuf::from("/data/lab/results/fit.csv"))
        );
        assert_eq!(
            on_disk(root, "./notes.md"),
            Some(std::path::PathBuf::from("/data/lab/./notes.md"))
        );
        for outside in ["", " ", "../secret", "results/../../secret", "/etc/passwd"] {
            assert_eq!(on_disk(root, outside), None, "{outside}");
        }
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
    fn an_html_file_is_shown_as_a_page_unless_it_is_too_large() {
        let mut content = preview("p", "report/index.HTML", "<h1>Title</h1>").content;
        assert!(is_html("a.htm") && is_html("b.xhtml") && !is_html("html/notes.txt"));
        assert!(html_page(&content));
        content.text = Some("x".repeat(MARKDOWN_LIMIT + 1));
        assert!(!html_page(&content));
        content.text = None;
        assert!(!html_page(&content));
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

    fn entry(name: &str, is_dir: bool, size: u64, modified: Option<u64>) -> DirEntry {
        DirEntry {
            name: name.into(),
            is_dir,
            size,
            modified_unix_millis: modified,
        }
    }

    #[test]
    fn a_folder_is_listed_folders_first_in_the_chosen_order() {
        let mut entries = vec![
            entry("b.csv", false, 10, Some(3)),
            entry("Data", true, 0, Some(1)),
            entry("a.txt", false, 500, Some(1)),
            entry("figs", true, 0, Some(9)),
            entry("C.md", false, 50, None),
        ];
        let names = |entries: &[DirEntry]| {
            entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect::<Vec<_>>()
        };
        sort_entries(&mut entries, FileSort::Name);
        assert_eq!(names(&entries), ["Data", "figs", "a.txt", "b.csv", "C.md"]);
        sort_entries(&mut entries, FileSort::Newest);
        // A file with no time goes last among the files.
        assert_eq!(names(&entries), ["figs", "Data", "b.csv", "a.txt", "C.md"]);
        sort_entries(&mut entries, FileSort::Largest);
        assert_eq!(names(&entries), ["Data", "figs", "a.txt", "C.md", "b.csv"]);
        // The choice steps round and is read back from what is saved.
        assert_eq!(FileSort::Largest.next(), FileSort::Name);
        for sort in FileSort::ALL {
            assert_eq!(FileSort::from_key(sort.key()), sort);
        }
        assert_eq!(FileSort::from_key("sideways"), FileSort::Name);
    }

    #[test]
    fn an_upload_says_what_it_copied_and_what_it_could_not() {
        let result = |source: &str, path: Option<&str>, error: Option<&str>| {
            wisp_dto::LocalFileUploadResult {
                source: source.into(),
                path: path.map(str::to_owned),
                error: error.map(str::to_owned),
            }
        };
        assert_eq!(
            upload_note(&[result("/a/x.csv", Some("data/x.csv"), None)]),
            "Copied 1 of 1 files here."
        );
        assert_eq!(
            upload_note(&[
                result("/a/x.csv", Some("data/x.csv"), None),
                result("/a/gone.csv", None, Some("not found")),
            ]),
            "Copied 1 of 2 files here. gone.csv: not found"
        );
    }

    #[test]
    fn a_name_search_keeps_only_its_latest_answer() {
        let mut files = FilesState::default();
        files.set_project(Some("a"));
        let (first, query) = files.search.begin("  reads ").unwrap();
        assert_eq!(query, "reads");
        assert!(files.search.active());
        let (second, _) = files.search.begin("reads.csv").unwrap();
        // An answer to the earlier search arrives late and is not shown.
        assert!(!files.search.accept(first, vec!["old.csv".into()]));
        assert!(files.search.accept(second, vec!["data/reads.csv".into()]));
        assert_eq!(
            files.search.hits.as_deref(),
            Some(&["data/reads.csv".to_string()][..])
        );
        // Emptied, the folder shows again and a late answer is dropped.
        assert_eq!(files.search.begin(" "), None);
        assert!(!files.search.active());
        assert!(!files.search.accept(second, vec!["x".into()]));
        // Another project starts without a search.
        files.search.begin("reads");
        files.set_project(Some("b"));
        assert!(!files.search.active());
    }

    #[test]
    fn the_order_stays_when_the_project_changes() {
        let mut files = FilesState::default();
        files.set_project(Some("a"));
        files.set_sort(FileSort::Largest);
        files.set_project(Some("b"));
        assert_eq!(files.sort, FileSort::Largest);
    }

    #[test]
    fn a_notebook_reads_as_its_cells_and_what_they_printed() {
        let book = serde_json::json!({
            "metadata": {"kernelspec": {"language": "python"}},
            "cells": [
                {"cell_type": "markdown", "source": ["# QC\n", "Reads per sample."]},
                {"cell_type": "code", "execution_count": 3,
                 "source": "import pandas as pd\nqc = pd.read_csv('qc.csv')",
                 "outputs": [
                    {"output_type": "stream", "name": "stdout", "text": ["loaded 12 rows\n"]},
                    {"output_type": "execute_result", "data": {"text/plain": ["   reads\n0  100"]}},
                    {"output_type": "display_data", "data": {"image/png": "iVBOR", "text/plain": "<Figure>"}},
                    {"output_type": "error", "ename": "KeyError", "evalue": "'depth'"}
                 ]},
                {"cell_type": "code", "execution_count": null, "source": "", "outputs": []}
            ]
        })
        .to_string();
        let read = notebook("analysis/qc.ipynb", &book).unwrap();
        assert!(read.starts_with("# QC\nReads per sample."));
        assert!(read.contains("`[3]`\n\n```python\nimport pandas as pd"));
        assert!(read.contains("```text\nloaded 12 rows\n```"));
        assert!(read.contains("```text\n   reads\n0  100\n```"));
        assert!(read.contains("*A figure is here.*"));
        assert!(!read.contains("<Figure>"));
        assert!(read.contains("KeyError: 'depth'"));
        assert!(read.contains("`[ ]`"));
        // Backticks in the code cannot close its fence.
        let ticks = serde_json::json!({"cells": [
            {"cell_type": "code", "source": "s = '```'", "outputs": []}
        ]})
        .to_string();
        assert!(
            notebook("t.ipynb", &ticks)
                .unwrap()
                .contains("````python\ns = '```'\n````")
        );
        // Not a notebook, or not one that can be read: shown as text.
        assert_eq!(notebook("qc.json", &book), None);
        assert_eq!(notebook("cut.ipynb", "{\"cells\": [{\"cell_"), None);
        assert_eq!(
            notebook("empty.ipynb", "{\"cells\": []}").unwrap(),
            "This notebook has no cells."
        );
    }

    #[test]
    fn documents_are_told_by_their_kind() {
        for path in [
            "paper.pdf",
            "data/Report.DOCX",
            "sheet.xlsx",
            "talk.pptx",
            "notes.odt",
        ] {
            assert!(is_document(path), "{path}");
        }
        for path in ["qc.csv", "README.md", "figure.png", "pdf", "archive.zip"] {
            assert!(!is_document(path), "{path}");
        }
    }

    fn content(path: &str, text: Option<&str>, truncated: bool) -> FileContent {
        FileContent {
            path: path.into(),
            mime: "text/plain".into(),
            text: text.map(str::to_owned),
            base64: None,
            truncated,
            total_bytes: text.map(|text| text.len() as u64),
        }
    }

    #[test]
    fn only_r_and_python_scripts_loaded_whole_are_edited() {
        assert_eq!(Script::of("analysis/qc.py"), Some(Script::Python));
        assert_eq!(Script::of("gui.PYW"), Some(Script::Python));
        assert_eq!(Script::of("plot.R"), Some(Script::R));
        assert_eq!(Script::of("plot.r"), Some(Script::R));
        for path in [
            "notes.md",
            "report.Rmd",
            "run.sh",
            "data.csv",
            "r",
            "py",
            "a.py/b",
        ] {
            assert_eq!(Script::of(path), None, "{path}");
        }
        assert_eq!(
            editable(&content("qc.py", Some("print(1)\n"), false)),
            Some(Ok(Script::Python))
        );
        // Other text is read, with nothing to say about it.
        assert_eq!(editable(&content("notes.md", Some("# hi"), false)), None);
        // A script the host cut, could not read as UTF-8, or too long to
        // number is shown read-only, and says why.
        for refused in [
            content("qc.py", Some("x = 1\n"), true),
            content("qc.py", None, false),
            content("qc.py", Some("caf\u{FFFD}\n"), false),
            content(
                "plot.R",
                Some(&format!("{}y", "x\n".repeat(EDIT_LINES))),
                false,
            ),
        ] {
            assert!(
                matches!(editable(&refused), Some(Err(why)) if !why.is_empty()),
                "{refused:?}"
            );
        }
        // A script of exactly the limit is still editable.
        let at_limit = "x\n".repeat(EDIT_LINES);
        assert_eq!(
            editable(&content("plot.R", Some(&at_limit), false)),
            Some(Ok(Script::R))
        );
    }

    #[test]
    fn a_save_keeps_the_files_line_endings_and_final_newline() {
        // Shown with plain newlines.
        assert_eq!(editor_text("a\r\nb\r\n"), "a\nb\n");
        // Written back as the file had them.
        assert_eq!(saved_text("a\r\nb\r\n", "a\nc\n"), "a\r\nc\r\n");
        assert_eq!(saved_text("a\nb\n", "a\nc\n"), "a\nc\n");
        // A file that ended with a newline still does; one that did not is
        // written as edited.
        assert_eq!(saved_text("a\nb\n", "a\nc"), "a\nc\n");
        assert_eq!(saved_text("a\r\nb\r\n", "a\nc"), "a\r\nc\r\n");
        assert_eq!(saved_text("a\nb", "a\nc"), "a\nc");
        assert_eq!(saved_text("a\nb", "a\nc\n"), "a\nc\n");
        // Emptied stays empty.
        assert_eq!(saved_text("a\n", ""), "");
        // Mixed endings take the more common one.
        assert_eq!(saved_text("a\r\nb\r\nc\n", "x\ny\n"), "x\r\ny\r\n");
        assert_eq!(saved_text("a\nb\nc\r\n", "x\ny\n"), "x\ny\n");
        // An edit that changes nothing is not one, whatever the endings.
        assert!(!is_dirty("a\r\nb\r\n", "a\nb\n"));
        assert!(is_dirty("a\r\nb\r\n", "a\nb"));
        assert!(is_dirty("x = 1\n", "x = 2\n"));
        assert!(!is_dirty("", ""));
    }

    #[test]
    fn drafts_are_kept_per_project_and_file_until_saved_or_undone() {
        let mut drafts = Drafts::default();
        assert!(drafts.is_empty());
        drafts.note("lab", "qc.py", "x = 1\n", "x = 2\n");
        drafts.note("lab", "plot.R", "plot(1)\n", "plot(2)\n");
        // The same path in another project is another file.
        drafts.note("other", "qc.py", "x = 1\n", "x = 3\n");
        assert_eq!(drafts.get("lab", "qc.py").unwrap().text, "x = 2\n");
        assert_eq!(drafts.get("other", "qc.py").unwrap().text, "x = 3\n");
        assert_eq!(drafts.names(), ["plot.R", "qc.py", "qc.py"]);
        // Typed back to the file's text, it is no draft.
        drafts.note("lab", "qc.py", "x = 1\n", "x = 1\n");
        assert_eq!(drafts.get("lab", "qc.py"), None);
        // Line endings alone do not make one.
        drafts.note("lab", "win.py", "a\r\n", "a\n");
        assert_eq!(drafts.get("lab", "win.py"), None);
        drafts.forget("lab", "plot.R");
        drafts.forget("other", "qc.py");
        assert!(drafts.is_empty());
    }

    #[test]
    fn a_draft_reopened_over_a_file_changed_on_disk_says_so() {
        let draft = Draft {
            base: "x = 1\n".into(),
            text: "x = 2\n".into(),
        };
        assert_eq!(reopen(None, "x = 1\n"), Reopened::Clean);
        assert_eq!(reopen(Some(&draft), "x = 1\n"), Reopened::Draft);
        assert_eq!(reopen(Some(&draft), "x = 9\n"), Reopened::ChangedOnDisk);
        // Byte for byte: a file rewritten with other line endings changed.
        assert_eq!(reopen(Some(&draft), "x = 1\r\n"), Reopened::ChangedOnDisk);
    }

    #[test]
    fn run_sends_the_selection_or_the_statement_and_moves_on() {
        let text = "x <- 1\n绘图 <- c(1,\n  2)\nprint(绘图)\n";
        let utf16 = |prefix: &str| prefix.encode_utf16().count() as u32;
        // A selection goes as it is, and the caret stays.
        let start = utf16("x <- 1\n");
        let end = start + utf16("绘图 <- c(1,");
        assert_eq!(
            run_code(text, Script::R, start, end),
            Some(("绘图 <- c(1,".into(), None))
        );
        // A caret runs its whole statement, across lines, and goes to the
        // next one: line 3, column 0.
        let caret = start + 1;
        assert_eq!(
            run_code(text, Script::R, caret, caret),
            Some(("绘图 <- c(1,\n  2)".into(), Some((3, 0))))
        );
        // Python blocks run whole.
        let py = "def f():\n    return 1\nf()\n";
        assert_eq!(
            run_code(py, Script::Python, 0, 0),
            Some(("def f():\n    return 1".into(), Some((2, 0))))
        );
        // Nothing to run on an empty line at the end, or in nothing.
        assert_eq!(run_code("x = 1\n", Script::Python, 6, 6), None);
        assert_eq!(run_code("", Script::Python, 0, 0), None);
        // A whole script goes by its path, as the WebView echoes it.
        assert_eq!(script_echo("analysis/qc.py"), "script analysis/qc.py");
        assert_eq!(Script::Python.statement_language(), "python");
        assert_eq!(Script::R.statement_language(), "r");
        assert_eq!(
            Script::R.runtime(),
            wisp_server::desktop::RuntimeLanguage::R
        );
    }

    #[test]
    fn the_window_names_what_closing_it_would_lose() {
        assert_eq!(
            unsaved_note(&["qc.py".into()]),
            "qc.py has edits that are not saved. They are lost if the window closes."
        );
        assert_eq!(
            unsaved_note(&["qc.py".into(), "plot.R".into()]),
            "2 files have edits that are not saved (qc.py, plot.R). They are lost if the \
             window closes."
        );
    }
}
