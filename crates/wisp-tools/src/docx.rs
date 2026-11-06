//! Bounded DOCX reading and native text revisions. The source package is never
//! overwritten. Only the selected runs in word/document.xml are rewritten;
//! every other ZIP entry is copied in its original compressed form.

use crate::env::{ToolEnv, ToolEvent, ToolResult};
use crate::tool::{arg_str, Tool};
use async_trait::async_trait;
use roxmltree::{Document, Node};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{Cursor, Read, Write};
use std::ops::Range;
use std::path::Path;
use wisp_llm::ToolSchema;
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const MAIN: &str = "word/document.xml";
const MAX_FILE: u64 = 50 * 1024 * 1024;
const MAX_EXPANDED: u64 = 128 * 1024 * 1024;
const MAX_XML: u64 = 8 * 1024 * 1024;
const MAX_TEXT: usize = 64 * 1024;
const MAX_RESULT: usize = 1024 * 1024;

type Result<T> = std::result::Result<T, String>;

pub mod review;

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn is_word(node: Node<'_, '_>, name: &str) -> bool {
    node.has_tag_name((W, name))
}

fn parse(xml: &str) -> Result<Document<'_>> {
    Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 300_000,
            entity_resolver: None,
        },
    )
    .map_err(err)
}

fn check_extension(path: &Path) -> Result<()> {
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("docx"))
    {
        return Err("This tool supports .docx files only".into());
    }
    Ok(())
}

fn read_source(path: &Path) -> Result<Vec<u8>> {
    check_extension(path)?;
    let file = std::fs::File::open(path).map_err(err)?;
    let metadata = file.metadata().map_err(err)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE {
        return Err("DOCX must be a regular file of at most 50 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err("DOCX grew beyond 50 MiB while reading".into());
    }
    Ok(bytes)
}

struct Package<'a> {
    archive: ZipArchive<Cursor<&'a [u8]>>,
    xml: String,
}

impl<'a> Package<'a> {
    fn open(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() as u64 > MAX_FILE {
            return Err("DOCX exceeds 50 MiB".into());
        }
        let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(err)?;
        if archive.len() > 4096 {
            return Err("DOCX has too many ZIP entries".into());
        }
        let mut names = HashSet::new();
        let mut expanded = 0u64;
        for i in 0..archive.len() {
            let entry = archive.by_index(i).map_err(err)?;
            let name = entry.name();
            if entry.enclosed_name().is_none()
                || name.contains('\\')
                || name.contains(':')
                || name.split('/').any(|part| part == "..")
                || !names.insert(name.to_string())
            {
                return Err("DOCX has an unsafe or duplicate ZIP entry".into());
            }
            expanded = expanded
                .checked_add(entry.size())
                .ok_or("ZIP size overflow")?;
            if expanded > MAX_EXPANDED {
                return Err("Expanded DOCX exceeds 128 MiB".into());
            }
            if name.to_ascii_lowercase().contains("vbaproject")
                || name.starts_with("_xmlsignatures/")
            {
                return Err("Macro-bearing or digitally signed packages are unsupported".into());
            }
        }
        // Do not silently operate on a decoy document.xml in another package
        // type or on a package whose actual main document lives elsewhere.
        let rels = read_xml_entry(&mut archive, "_rels/.rels")?;
        let rels = parse(&rels)?;
        let office: Vec<_> = rels.descendants().filter(|n| {
            n.has_tag_name(("http://schemas.openxmlformats.org/package/2006/relationships", "Relationship"))
                && n.attribute("Type") == Some("http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument")
        }).collect();
        if office.len() != 1
            || !matches!(
                office[0].attribute("Target"),
                Some("word/document.xml" | "/word/document.xml")
            )
            || office[0]
                .attribute("TargetMode")
                .is_some_and(|m| m != "Internal")
        {
            return Err("Unsupported DOCX main document relationship".into());
        }
        let types = read_xml_entry(&mut archive, "[Content_Types].xml")?;
        let types = parse(&types)?;
        if !types.descendants().any(|n| {
            n.has_tag_name(("http://schemas.openxmlformats.org/package/2006/content-types", "Override"))
                && n.attribute("PartName") == Some("/word/document.xml")
                && n.attribute("ContentType") == Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml")
        }) {
            return Err("Package is not a supported Word DOCX document".into());
        }
        let xml = read_xml_entry(&mut archive, MAIN)?;
        let doc = parse(&xml)?;
        if !is_word(doc.root_element(), "document") {
            return Err("Only Transitional WordprocessingML is supported in this prototype".into());
        }
        Ok(Self { archive, xml })
    }

    fn with_xml(mut self, xml: &str) -> Result<Vec<u8>> {
        if xml.len() as u64 > MAX_XML {
            return Err("Edited document XML exceeds 8 MiB".into());
        }
        parse(xml)?;
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for i in 0..self.archive.len() {
            let entry = self.archive.by_index(i).map_err(err)?;
            if entry.name() == MAIN {
                writer
                    .start_file(
                        MAIN,
                        SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated),
                    )
                    .map_err(err)?;
                writer.write_all(xml.as_bytes()).map_err(err)?;
            } else {
                writer.raw_copy_file(entry).map_err(err)?;
            }
        }
        let bytes = writer.finish().map_err(err)?.into_inner();
        if bytes.len() as u64 > MAX_FILE {
            return Err("Edited DOCX exceeds 50 MiB".into());
        }
        Ok(bytes)
    }
}

fn read_xml_entry(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<String> {
    let entry = archive.by_name(name).map_err(err)?;
    if entry.size() > MAX_XML {
        return Err(format!("{name} exceeds the 8 MiB XML limit"));
    }
    let mut xml = String::new();
    entry
        .take(MAX_XML + 1)
        .read_to_string(&mut xml)
        .map_err(err)?;
    if xml.len() as u64 > MAX_XML {
        return Err(format!("{name} expanded beyond the XML limit"));
    }
    Ok(xml)
}

fn paragraph_text(p: Node<'_, '_>) -> String {
    p.descendants()
        .filter(|n| is_word(*n, "t"))
        .filter(|n| {
            !n.ancestors()
                .any(|a| is_word(a, "del") || is_word(a, "moveFrom"))
        })
        .filter_map(|n| n.text())
        .collect()
}

// Mark ranges as well as individual marker paragraphs: fields, bookmarks and
// comments can start before a paragraph and finish after it. Malformed ranges
// protect the entire document rather than inviting an unsafe edit.
fn protected_ranges(doc: &Document<'_>) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut open = std::collections::HashMap::new();
    let mut fields = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        if n.tag_name().namespace() != Some(W) {
            continue;
        }
        let name = n.tag_name().name();
        if name == "fldChar" {
            match n.attribute((W, "fldCharType")) {
                Some("begin") => fields.push(n.range().start),
                Some("end") => match fields.pop() {
                    Some(start) => ranges.push(start..n.range().end),
                    None => return vec![0..usize::MAX],
                },
                Some("separate") if !fields.is_empty() => {}
                _ => return vec![0..usize::MAX],
            }
        }
        for kind in [
            "bookmark",
            "commentRange",
            "moveFromRange",
            "moveToRange",
            "perm",
        ] {
            if name == format!("{kind}Start") {
                let Some(id) = n.attribute((W, "id")) else {
                    return vec![0..usize::MAX];
                };
                if open.insert((kind, id), n.range().start).is_some() {
                    return vec![0..usize::MAX];
                }
            } else if name == format!("{kind}End") {
                let Some(id) = n.attribute((W, "id")) else {
                    return vec![0..usize::MAX];
                };
                match open.remove(&(kind, id)) {
                    Some(start) => ranges.push(start..n.range().end),
                    None => return vec![0..usize::MAX],
                }
            }
        }
    }
    if !open.is_empty() || !fields.is_empty() {
        return vec![0..usize::MAX];
    }
    ranges
}

fn editable(p: Node<'_, '_>, protected: &[Range<usize>]) -> Result<()> {
    if !p.parent().is_some_and(|n| is_word(n, "body")) {
        return Err("Only top-level body paragraphs are editable".into());
    }
    if protected
        .iter()
        .any(|r| r.start < p.range().end && p.range().start < r.end)
    {
        return Err("Paragraph intersects a field, bookmark, comment or protected range".into());
    }
    for n in p.children().filter(|n| n.is_element()) {
        if is_word(n, "pPr") {
            // Paragraph-mark revisions, numbering changes and section breaks
            // require semantics beyond inline text replacement.
            if n.descendants().any(|n| {
                n.is_element()
                    && (n.tag_name().name().ends_with("Change")
                        || matches!(
                            n.tag_name().name(),
                            "ins" | "del" | "moveFrom" | "moveTo" | "sectPr"
                        ))
            }) {
                return Err("Paragraph has revisions or a section break".into());
            }
            continue;
        }
        if !is_word(n, "r") {
            return Err(
                "Paragraph contains revisions, links, fields or other complex content".into(),
            );
        }
        for child in n.children().filter(|n| n.is_element()) {
            if is_word(child, "rPr") {
                if child.descendants().any(|n| {
                    n.is_element()
                        && (n.tag_name().name().ends_with("Change")
                            || matches!(n.tag_name().name(), "ins" | "del" | "moveFrom" | "moveTo"))
                }) {
                    return Err("Run contains formatting revisions".into());
                }
            } else if !is_word(child, "t") || child.children().any(|n| n.is_element()) {
                return Err("Only plain text runs and their formatting are supported".into());
            }
        }
    }
    Ok(())
}

fn inspect(bytes: &[u8], offset: usize, limit: usize) -> Result<Value> {
    let package = Package::open(bytes)?;
    let doc = parse(&package.xml)?;
    let protected = protected_ranges(&doc);
    let paragraphs: Vec<_> = doc.descendants().filter(|n| is_word(*n, "p")).collect();
    let mut rows = Vec::new();
    let mut budget = 0;
    for (index, p) in paragraphs.iter().enumerate().skip(offset).take(limit) {
        let text = paragraph_text(*p);
        let reason = editable(*p, &protected).err();
        let row = json!({"paragraph_id": format!("p{}", index + 1), "text": text,
            "editable": reason.is_none(), "reason": reason});
        budget += row.to_string().len();
        if budget > MAX_RESULT {
            if rows.is_empty() {
                return Err("Paragraph exceeds the 1 MiB result limit".into());
            }
            break;
        }
        rows.push(row);
    }
    let next = offset.saturating_add(rows.len());
    Ok(json!({"source_sha256": sha256(bytes), "paragraphs": rows,
        "total_paragraphs": paragraphs.len(),
        "next_offset": (next < paragraphs.len()).then_some(next),
        "view": "current text (insertions included, deleted text omitted); complex paragraphs are read-only"}))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// Find the end of a start tag without mistaking '>' in an attribute for it.
fn opening_tag(xml: &str) -> Result<&str> {
    let mut quote = None;
    for (i, c) in xml.char_indices() {
        match (quote, c) {
            (Some(q), c) if q == c => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, '>') => return Ok(&xml[..=i]),
            _ => {}
        }
    }
    Err("Invalid XML start tag".into())
}

fn run_fragment(
    run: Node<'_, '_>,
    xml: &str,
    text: &str,
    deleted: bool,
    prefix: &str,
) -> Result<String> {
    if text.is_empty() {
        return Ok(String::new());
    }
    let raw = &xml[run.range()];
    let open = opening_tag(raw)?;
    let end = raw.rfind("</").ok_or("Text run has no closing tag")?;
    let properties = run
        .children()
        .find(|n| is_word(*n, "rPr"))
        .map(|n| &xml[n.range()])
        .unwrap_or("");
    let tag = if deleted { "delText" } else { "t" };
    Ok(format!("{open}{properties}<{prefix}:{tag} xmlns:{prefix}=\"{W}\" xml:space=\"preserve\">{}</{prefix}:{tag}>{}", escape(text), &raw[end..]))
}

struct Edit<'a> {
    paragraph_id: &'a str,
    old: &'a str,
    new: &'a str,
    source_sha256: &'a str,
}

fn propose(bytes: &[u8], edit: Edit<'_>, date: &str) -> Result<(Vec<u8>, Value)> {
    if sha256(bytes) != edit.source_sha256 {
        return Err("Source DOCX changed; read_docx again before proposing edits".into());
    }
    if edit.old.is_empty()
        || edit.old == edit.new
        || edit.old.len() > MAX_TEXT
        || edit.new.len() > MAX_TEXT
    {
        return Err(
            "old must be nonempty and different from new; each is limited to 64 KiB".into(),
        );
    }
    if edit
        .new
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{fffe}' | '\u{ffff}'))
    {
        return Err("new must be single-paragraph text without control characters".into());
    }
    let index = edit
        .paragraph_id
        .strip_prefix('p')
        .and_then(|s| s.parse::<usize>().ok())
        .and_then(|n| n.checked_sub(1))
        .ok_or("Invalid paragraph_id")?;
    let package = Package::open(bytes)?;
    let doc = parse(&package.xml)?;
    let paragraph = doc
        .descendants()
        .filter(|n| is_word(*n, "p"))
        .nth(index)
        .ok_or("paragraph_id was not found; read_docx again")?;
    editable(paragraph, &protected_ranges(&doc))?;
    let text = paragraph_text(paragraph);
    let matched = text.find(edit.old);
    if matched.is_none() || matched != text.rfind(edit.old) {
        return Err("old must match exactly once in the selected paragraph".into());
    }
    // Common edges stay ordinary text, so appending a word produces an insert
    // and shortening a phrase produces a delete rather than a whole-run rewrite.
    let prefix_len: usize = edit
        .old
        .chars()
        .zip(edit.new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let old_tail = &edit.old[prefix_len..];
    let new_tail = &edit.new[prefix_len..];
    let suffix_len: usize = old_tail
        .chars()
        .rev()
        .zip(new_tail.chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let matched = matched.ok_or("old was not found")?;
    let start = matched + prefix_len;
    let end = matched + edit.old.len() - suffix_len;
    let replacement = &edit.new[prefix_len..edit.new.len() - suffix_len];
    let mut cursor = 0;
    let runs: Vec<_> = paragraph
        .children()
        .filter(|n| is_word(*n, "r"))
        .filter_map(|n| {
            let text = paragraph_text(n);
            let range = cursor..cursor + text.len();
            cursor = range.end;
            (!text.is_empty()).then_some((n, range, text))
        })
        .collect();
    let first = runs
        .iter()
        .position(|(_, r, _)| r.start <= start && start < r.end)
        .or_else(|| {
            (start == text.len())
                .then(|| runs.len().checked_sub(1))
                .flatten()
        })
        .ok_or("Cannot locate text run")?;
    let last = if start == end {
        first
    } else {
        runs.iter()
            .rposition(|(_, r, _)| r.start < end && r.end > start)
            .ok_or("Cannot locate final text run")?
    };
    // Empty runs between the endpoints may carry formatting/annotations. Keep
    // the prototype conservative instead of dropping them during reconstruction.
    let span = runs[first].0.range().start..runs[last].0.range().end;
    if paragraph.children().filter(|n| is_word(*n, "r")).any(|n| {
        n.range().start >= span.start && n.range().end <= span.end && paragraph_text(n).is_empty()
    }) {
        return Err("Edit crosses an empty formatting run".into());
    }
    let mut prefix = "wispRev".to_string();
    while package.xml.contains(&prefix) {
        prefix.push('_');
    }
    let mut next_id = 0u32;
    for n in doc.descendants() {
        if let Some(id) = n.attribute((W, "id")) {
            let id = id
                .parse::<u32>()
                .map_err(|_| "Unsupported existing annotation ID")?;
            next_id = next_id.max(id.checked_add(1).ok_or("Revision ID overflow")?);
        }
    }
    let mut fragments = run_fragment(
        runs[first].0,
        &package.xml,
        &runs[first].2[..start - runs[first].1.start],
        false,
        &prefix,
    )?;
    let mut deleted = String::new();
    for (run, range, text) in &runs[first..=last] {
        let a = start.max(range.start) - range.start;
        let b = end.min(range.end).max(range.start) - range.start;
        if a < b {
            deleted.push_str(&run_fragment(
                *run,
                &package.xml,
                &text[a..b],
                true,
                &prefix,
            )?);
        }
    }
    let mut revisions = Vec::new();
    for (kind, content) in [
        ("del", deleted),
        (
            "ins",
            run_fragment(runs[first].0, &package.xml, replacement, false, &prefix)?,
        ),
    ] {
        if content.is_empty() {
            continue;
        }
        let id = next_id;
        next_id = next_id.checked_add(1).ok_or("Revision ID overflow")?;
        fragments.push_str(&format!("<{prefix}:{kind} xmlns:{prefix}=\"{W}\" {prefix}:id=\"{id}\" {prefix}:author=\"Wisp\" {prefix}:date=\"{date}\">{content}</{prefix}:{kind}>"));
        revisions.push(json!({"id": id, "kind": kind}));
    }
    fragments.push_str(&run_fragment(
        runs[last].0,
        &package.xml,
        &runs[last].2[end - runs[last].1.start..],
        false,
        &prefix,
    )?);
    let mut xml = package.xml.clone();
    xml.replace_range(span, &fragments);
    // Borrowed XML nodes must not survive moving the archive for serialization.
    let output = package.with_xml(&xml)?;
    let summary = json!({"source_sha256": edit.source_sha256,
        "output_sha256": sha256(&output), "paragraph_id": edit.paragraph_id,
        "revisions": revisions, "status": "pending_review",
        "review": "Open the output in Word and accept or reject the Wisp revisions. Source was not overwritten."});
    Ok((output, summary))
}

pub struct ReadDocxTool;
pub struct SuggestDocxEditTool;

#[async_trait]
impl Tool for ReadDocxTool {
    fn name(&self) -> &str {
        "read_docx"
    }
    fn read_only(&self) -> bool {
        true
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(),
            "Read DOCX paragraphs with snapshot-bound IDs, editable flags and source_sha256 for suggest_docx_edit. No Word installation needed. Use offset/limit to page; complex paragraphs are read-only.",
            json!({"type":"object", "properties": {
                "path":{"type":"string"}, "offset":{"type":"integer", "minimum":0},
                "limit":{"type":"integer", "minimum":1, "maximum":100}
            }, "required":["path"]}))
    }
    fn preview(&self, args: &Value) -> String {
        arg_str(args, "path").unwrap_or_default()
    }
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let result: Result<Value> = (|| {
            if env.is_cancelled() {
                return Err("interrupted by user".into());
            }
            let path = env.resolve_read_path(&arg_str(args, "path")?, false)?;
            let offset = page_arg(args, "offset", 0)?;
            let limit = page_arg(args, "limit", 50)?;
            if !(1..=100).contains(&limit) {
                return Err("limit must be 1..100".into());
            }
            inspect(&read_source(&path)?, offset, limit)
        })();
        match result {
            Ok(value) => ToolResult::ok(value.to_string()),
            Err(error) => ToolResult::fail(error),
        }
    }
}

fn page_arg(args: &Value, name: &str, default: usize) -> Result<usize> {
    match args.get(name) {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| format!("{name} must be a nonnegative integer")),
    }
}

#[async_trait]
impl Tool for SuggestDocxEditTool {
    fn name(&self) -> &str {
        "suggest_docx_edit"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema::new(self.name(),
            "Propose a text edit as native Word tracked changes in a NEW .docx output; never overwrites source or existing output. First use read_docx for paragraph_id/source_sha256. old must match once, new is single-paragraph text (empty means delete). Common edges stay unchanged; inserted text inherits the first affected run's formatting. Only plain top-level body paragraphs are editable. Review the output in Word; reread each output before further edits. Existing revisions and complex paragraphs cannot be edited by this prototype.",
            json!({"type":"object", "properties": {
                "path":{"type":"string", "description":"Source DOCX"},
                "output_path":{"type":"string", "description":"New DOCX path inside project; parent directory must exist"},
                "source_sha256":{"type":"string"}, "paragraph_id":{"type":"string"},
                "old":{"type":"string"}, "new":{"type":"string"}
            }, "required":["path", "output_path", "source_sha256", "paragraph_id", "old", "new"]}))
    }
    fn preview(&self, args: &Value) -> String {
        arg_str(args, "output_path").unwrap_or_default()
    }
    async fn run(&self, args: &Value, env: &dyn ToolEnv) -> ToolResult {
        let result: Result<(String, Value)> = (|| {
            if env.is_cancelled() {
                return Err("interrupted by user".into());
            }
            let source = env.resolve_read_path(&arg_str(args, "path")?, false)?;
            let output_path = arg_str(args, "output_path")?;
            let output = crate::safety::validate_file_path(env.project_root(), &output_path)?;
            check_extension(&output)?;
            // symlink_metadata also detects dangling links; create_new below is
            // the actual race-safe no-overwrite gate on both Windows and Unix.
            if output.symlink_metadata().is_ok() {
                return Err("Output already exists; choose a new .docx path".into());
            }
            let bytes = read_source(&source)?;
            let (edited, mut summary) = propose(
                &bytes,
                Edit {
                    paragraph_id: &arg_str(args, "paragraph_id")?,
                    old: &arg_str(args, "old")?,
                    new: &arg_str(args, "new")?,
                    source_sha256: &arg_str(args, "source_sha256")?,
                },
                &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            )?;
            if env.is_cancelled() {
                return Err("interrupted by user".into());
            }
            // Detect a source saved by Word while the suggestion was prepared.
            if read_source(&source)? != bytes {
                return Err("Source changed while preparing revision; read_docx again".into());
            }
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)
                .map_err(err)?;
            if let Err(error) = file.write_all(&edited).and_then(|()| file.sync_all()) {
                drop(file);
                let _ = std::fs::remove_file(&output);
                return Err(err(error));
            }
            summary["output_path"] = json!(output_path);
            Ok((output_path, summary))
        })();
        match result {
            Ok((path, summary)) => {
                env.emit(ToolEvent::FileChanged { path }).await;
                ToolResult::ok(summary.to_string())
            }
            Err(error) => ToolResult::fail(error),
        }
    }
}

#[cfg(test)]
mod tests;
