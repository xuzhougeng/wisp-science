//! Durable resource references for newly persisted assistant messages.
//!
//! Agent Markdown is preserved verbatim. File destinations are parsed once at
//! persistence time, validated against the message's project, snapshotted into
//! content-addressed project storage, and bound to an immutable artifact
//! version. The UI consumes these bindings and never reparses their paths.

use pulldown_cmark::{Event, Options, Parser, Tag};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use wisp_store::{MessageResourceLink, Store};

const MAX_SNAPSHOT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct MarkdownResource {
    ordinal: i64,
    reference: String,
    kind: String,
}

#[derive(Clone, Debug)]
struct ResolvedFile {
    real: PathBuf,
    display_name: String,
    kind: String,
    mime_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UiMessageResource {
    pub id: String,
    pub ordinal: i64,
    pub original_reference: String,
    pub artifact_id: Option<String>,
    pub artifact_version_id: Option<String>,
    pub display_name: String,
    pub kind: String,
    pub mime_type: String,
    pub status: String,
    pub error: Option<String>,
}

impl From<&MessageResourceLink> for UiMessageResource {
    fn from(link: &MessageResourceLink) -> Self {
        Self {
            id: link.id.clone(),
            ordinal: link.ordinal,
            original_reference: link.original_reference.clone(),
            artifact_id: link.artifact_id.clone(),
            artifact_version_id: link.artifact_version_id.clone(),
            display_name: link.display_name.clone(),
            kind: link.resource_kind.clone(),
            mime_type: link.mime_type.clone(),
            status: link.status.clone(),
            error: link.error.clone(),
        }
    }
}

fn markdown_resources(markdown: &str) -> Vec<MarkdownResource> {
    let markdown = rewrite_codex_image_tags(markdown);
    let parser = Parser::new_ext(&markdown, Options::ENABLE_TABLES);
    let mut resources = Vec::new();
    for event in parser {
        let (reference, kind) = match event {
            Event::Start(Tag::Image { dest_url, .. }) => (dest_url, "image"),
            Event::Start(Tag::Link { dest_url, .. }) => (dest_url, "file"),
            _ => continue,
        };
        let reference = reference.trim();
        if reference.is_empty() || is_external_reference(reference) {
            continue;
        }
        resources.push(MarkdownResource {
            ordinal: resources.len() as i64,
            reference: reference.to_string(),
            kind: kind.to_string(),
        });
    }
    resources
}

/// Normalize Codex ACP image blocks before the one-time Markdown parse. The
/// original message remains untouched; only the binding input is normalized.
fn rewrite_codex_image_tags(markdown: &str) -> Cow<'_, str> {
    if !markdown.contains("<image") {
        return Cow::Borrowed(markdown);
    }
    let mut out = String::with_capacity(markdown.len());
    let mut rest = markdown;
    let mut changed = false;
    while let Some(start) = rest.find("<image") {
        out.push_str(&rest[..start]);
        let tag_source = &rest[start..];
        let Some(open_end) = tag_source.find('>') else {
            out.push_str(tag_source);
            rest = "";
            break;
        };
        let Some(close_offset) = tag_source[open_end + 1..].find("</image>") else {
            out.push_str(tag_source);
            rest = "";
            break;
        };
        let whole_end = open_end + 1 + close_offset + "</image>".len();
        let open_tag = &tag_source[..=open_end];
        if let Some(path) = image_tag_attribute(open_tag, "path") {
            out.push_str("[ACP image](<");
            out.push_str(path.trim());
            out.push_str(">)");
            changed = true;
        } else {
            out.push_str(&tag_source[..whole_end]);
        }
        rest = &tag_source[whole_end..];
    }
    out.push_str(rest);
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(markdown)
    }
}

fn image_tag_attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=");
    let start = tag.find(&needle)? + needle.len();
    let value = &tag[start..];
    let quote = value.chars().next()?;
    if quote == '\'' || quote == '"' {
        let value = &value[1..];
        return Some(&value[..value.find(quote)?]);
    }
    if quote == '[' {
        return Some(&value[1..value.find(']')?]);
    }
    Some(&value[..value.find([' ', '>']).unwrap_or(value.len())])
}

fn is_external_reference(reference: &str) -> bool {
    let lower = reference.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("data:")
        || lower.starts_with('#')
}

fn percent_decode(reference: &str) -> String {
    let bytes = reference.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hi = (bytes[index + 1] as char).to_digit(16);
            let lo = (bytes[index + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn strip_markdown_wrapper(reference: &str) -> &str {
    let trimmed = reference.trim();
    if let Some(inner) = trimmed
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
    {
        return inner.trim();
    }
    for quote in ['\'', '"'] {
        if let Some(inner) = trimmed
            .strip_prefix(quote)
            .and_then(|value| value.strip_suffix(quote))
        {
            return inner.trim();
        }
    }
    trimmed
}

/// Codex file-link examples use `/abs/path/...` as a display placeholder. On
/// Windows an ACP agent can accidentally preserve that prefix in a real link,
/// producing `/abs/path/D:/project/file` instead of `D:/project/file`. Keep the
/// compatibility rule deliberately narrow: only strip the exact placeholder
/// (or the WebView's single leading slash) when what follows is a drive path.
#[cfg(any(windows, test))]
fn normalize_windows_webview_reference(reference: &str) -> &str {
    fn is_drive_path(value: &str) -> bool {
        let bytes = value.as_bytes();
        bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\')
    }

    if let Some(candidate) = reference.strip_prefix("/abs/path/") {
        if is_drive_path(candidate) {
            return candidate;
        }
    }
    if let Some(candidate) = reference.strip_prefix('/') {
        if is_drive_path(candidate) {
            return candidate;
        }
    }
    reference
}

fn reference_path(reference: &str) -> Result<PathBuf, String> {
    let reference = strip_markdown_wrapper(reference);
    if reference.to_ascii_lowercase().starts_with("file:") {
        let url = url::Url::parse(reference)
            .map_err(|error| format!("invalid file URI '{reference}': {error}"))?;
        return url
            .to_file_path()
            .map_err(|_| format!("file URI cannot be converted to a native path: {reference}"));
    }
    let decoded = percent_decode(reference);
    #[cfg(windows)]
    let decoded = normalize_windows_webview_reference(&decoded).to_string();
    Ok(PathBuf::from(decoded))
}

fn kind_and_mime(path: &Path, requested_kind: &str) -> Option<(String, String)> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let (kind, mime) = match ext.as_str() {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "gif" => ("image", "image/gif"),
        "webp" => ("image", "image/webp"),
        "svg" => ("image", "image/svg+xml"),
        "pdf" => ("pdf", "application/pdf"),
        "docx" => (
            "docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        "xlsx" => (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        "pptx" => (
            "pptx",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        "md" | "markdown" => ("markdown", "text/markdown"),
        "bib" => ("text", "text/x-bibtex"),
        "csv" => ("csv", "text/csv"),
        "tsv" => ("csv", "text/tab-separated-values"),
        "json" => ("json", "application/json"),
        "ipynb" => ("notebook", "application/x-ipynb+json"),
        "html" | "htm" => ("html", "text/html"),
        "txt" | "log" => ("text", "text/plain"),
        _ => return None,
    };
    let kind = if requested_kind == "image" && kind == "image" {
        "image"
    } else {
        kind
    };
    Some((kind.into(), mime.into()))
}

fn resolve_reference(
    root: &Path,
    reference: &str,
    requested_kind: &str,
) -> Result<ResolvedFile, String> {
    let mut candidates = vec![reference_path(reference)?];
    // Some ACP Markdown destinations contain one unmatched terminal quote.
    // Treat it as syntax only after the literal path fails, so a legitimate
    // apostrophe in a filename remains valid.
    let trimmed = reference.trim();
    if trimmed.ends_with(['\'', '"']) && trimmed.len() > 1 {
        if let Ok(candidate) = reference_path(&trimmed[..trimmed.len() - 1]) {
            candidates.push(candidate);
        }
    }
    let mut last_error = None;
    for candidate in candidates {
        let candidate_text = candidate.to_string_lossy();
        match wisp_tools::safety::validate_file_path(root, &candidate_text) {
            Ok(real) => {
                let metadata = std::fs::metadata(&real).map_err(|error| error.to_string())?;
                if metadata.len() > MAX_SNAPSHOT_BYTES {
                    return Err(format!(
                        "resource exceeds {} byte preview snapshot limit",
                        MAX_SNAPSHOT_BYTES
                    ));
                }
                let (kind, mime_type) = kind_and_mime(&real, requested_kind)
                    .ok_or_else(|| "unsupported preview resource type".to_string())?;
                let display_name = real
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("resource")
                    .to_string();
                return Ok(ResolvedFile {
                    real,
                    display_name,
                    kind,
                    mime_type,
                });
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| "resource path could not be resolved".into()))
}

fn snapshot_path(root: &Path, checksum: &str, source: &Path) -> PathBuf {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| format!(".{extension}"))
        .unwrap_or_default();
    root.join(".wisp")
        .join("artifacts")
        .join("sha256")
        .join(&checksum[..2])
        .join(format!("{checksum}{extension}"))
}

fn source_artifact_identity(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        normalized.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        normalized
    }
}

async fn bind_resolved(
    store: &Store,
    root: &Path,
    project_id: &str,
    frame_id: &str,
    resource: &MarkdownResource,
    resolved: ResolvedFile,
) -> Result<MessageResourceLink, String> {
    let bytes = std::fs::read(&resolved.real).map_err(|error| error.to_string())?;
    let checksum = wisp_sync::sha256_hex(&bytes);
    let snapshot = snapshot_path(root, &checksum, &resolved.real);
    if !snapshot.exists() {
        if let Some(parent) = snapshot.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        std::fs::write(&snapshot, &bytes).map_err(|error| error.to_string())?;
    }
    let source_identity = source_artifact_identity(&resolved.real);
    let artifact_key = wisp_sync::sha256_hex(format!("{project_id}\0{source_identity}").as_bytes());
    let artifact_id = format!("resource-{}", &artifact_key[..32]);
    let relative_snapshot = snapshot
        .strip_prefix(root)
        .unwrap_or(&snapshot)
        .to_string_lossy()
        .replace('\\', "/");
    let current = store
        .latest_artifact_version(&artifact_id)
        .await
        .map_err(|error| error.to_string())?;
    let version_id = if let Some(version) =
        current.filter(|version| version.checksum.as_deref() == Some(checksum.as_str()))
    {
        version.id
    } else {
        let version_id = store
            .save_artifact(
                &artifact_id,
                project_id,
                frame_id,
                &resolved.display_name,
                &resolved.mime_type,
                &relative_snapshot,
            )
            .await
            .map_err(|error| error.to_string())?;
        store
            .set_artifact_version_file_metadata(&version_id, bytes.len() as i64, &checksum)
            .await
            .map_err(|error| error.to_string())?;
        version_id
    };
    Ok(MessageResourceLink {
        id: uuid::Uuid::new_v4().to_string(),
        frame_id: frame_id.to_string(),
        message_seq: 0,
        ordinal: resource.ordinal,
        original_reference: resource.reference.clone(),
        artifact_id: Some(artifact_id),
        artifact_version_id: Some(version_id),
        display_name: resolved.display_name,
        resource_kind: resolved.kind,
        mime_type: resolved.mime_type,
        status: "ready".into(),
        error: None,
        created_at: chrono::Utc::now().timestamp(),
    })
}

pub(crate) async fn bind_new_message_resources(
    store: &Store,
    root: &Path,
    project_id: &str,
    frame_id: &str,
    message_seq: i64,
    markdown: &str,
) -> Vec<MessageResourceLink> {
    let markdown = resource_scan_text(markdown);
    let resources = markdown_resources(&markdown);
    if resources.is_empty() {
        return Vec::new();
    }
    let mut resolved_cache = HashMap::<String, Result<ResolvedFile, String>>::new();
    let mut links = Vec::with_capacity(resources.len());
    for resource in resources {
        let resolved = resolved_cache
            .entry(resource.reference.clone())
            .or_insert_with(|| resolve_reference(root, &resource.reference, &resource.kind))
            .clone();
        let mut link = match resolved {
            Ok(resolved) => bind_resolved(store, root, project_id, frame_id, &resource, resolved)
                .await
                .unwrap_or_else(|error| failed_link(frame_id, &resource, error)),
            Err(error) => failed_link(frame_id, &resource, error),
        };
        link.message_seq = message_seq;
        links.push(link);
    }
    if let Err(error) = store
        .replace_message_resource_links(frame_id, message_seq, &links)
        .await
    {
        tracing::warn!(%frame_id, message_seq, %error, "failed to persist message resources");
        return Vec::new();
    }
    links
}

/// Delegated workers return JSON contracts rather than ordinary Markdown.
/// Decode all string fields before scanning so links in a summary or evidence
/// field have the same durable-resource behavior as regular assistant text.
fn resource_scan_text(content: &str) -> Cow<'_, str> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content.trim()) else {
        return Cow::Borrowed(content);
    };
    let mut strings = Vec::new();
    collect_json_strings(&value, &mut strings);
    if strings.is_empty() {
        Cow::Borrowed(content)
    } else {
        Cow::Owned(strings.join("\n"))
    }
}

fn collect_json_strings<'a>(value: &'a serde_json::Value, strings: &mut Vec<&'a str>) {
    match value {
        serde_json::Value::String(value) => strings.push(value),
        serde_json::Value::Array(values) => {
            for value in values {
                collect_json_strings(value, strings);
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                collect_json_strings(value, strings);
            }
        }
        _ => {}
    }
}

fn failed_link(frame_id: &str, resource: &MarkdownResource, error: String) -> MessageResourceLink {
    let display_name = reference_path(&resource.reference)
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| resource.reference.clone());
    MessageResourceLink {
        id: uuid::Uuid::new_v4().to_string(),
        frame_id: frame_id.to_string(),
        message_seq: 0,
        ordinal: resource.ordinal,
        original_reference: resource.reference.clone(),
        artifact_id: None,
        artifact_version_id: None,
        display_name,
        resource_kind: resource.kind.clone(),
        mime_type: "application/octet-stream".into(),
        status: "unresolved".into(),
        error: Some(error),
        created_at: chrono::Utc::now().timestamp(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_local_markdown_resources_in_document_order() {
        let resources = markdown_resources(
            "[report](<reports/a b.md>) ![plot](figures/a.png) [web](https://example.com)",
        );
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].reference, "reports/a b.md");
        assert_eq!(resources[0].kind, "file");
        assert_eq!(resources[1].reference, "figures/a.png");
        assert_eq!(resources[1].kind, "image");
    }

    #[test]
    fn extracts_links_from_structured_agent_result_strings() {
        let text = resource_scan_text(
            r#"{"summary":"Created [report](report.md)","evidence":["![plot](plot.png)"]}"#,
        );
        let resources = markdown_resources(&text);
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].reference, "report.md");
        assert_eq!(resources[1].reference, "plot.png");
    }

    #[test]
    fn extracts_codex_acp_image_blocks() {
        let resources = markdown_resources(
            r#"before <image name=[Image #1] path="D:/work/plot one.png">ignored</image> after"#,
        );
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].reference, "D:/work/plot one.png");
        assert_eq!(resources[0].kind, "file");
    }

    #[test]
    fn document_resources_bind_as_previewable_artifacts() {
        assert_eq!(
            kind_and_mime(Path::new("analysis.ipynb"), "file"),
            Some((
                "notebook".to_string(),
                "application/x-ipynb+json".to_string()
            ))
        );
        assert_eq!(
            kind_and_mime(Path::new("manuscript.docx"), "file"),
            Some((
                "docx".to_string(),
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    .to_string()
            ))
        );
        assert_eq!(
            kind_and_mime(Path::new("results.xlsx"), "file"),
            Some((
                "xlsx".to_string(),
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()
            ))
        );
        assert_eq!(
            kind_and_mime(Path::new("talk.pptx"), "file"),
            Some((
                "pptx".to_string(),
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                    .to_string()
            ))
        );
        assert_eq!(
            kind_and_mime(Path::new("references.bib"), "file"),
            Some(("text".to_string(), "text/x-bibtex".to_string()))
        );
    }

    #[test]
    fn normalizes_codex_windows_placeholder_without_rewriting_posix_paths() {
        assert_eq!(
            normalize_windows_webview_reference("/abs/path/D:/ZZM/paper/report.md"),
            "D:/ZZM/paper/report.md"
        );
        assert_eq!(
            normalize_windows_webview_reference("/D:/ZZM/paper/report.md"),
            "D:/ZZM/paper/report.md"
        );
        assert_eq!(
            normalize_windows_webview_reference("/abs/path/reports/report.md"),
            "/abs/path/reports/report.md"
        );
        assert_eq!(
            normalize_windows_webview_reference("reports/report.md"),
            "reports/report.md"
        );
    }

    #[test]
    fn decodes_file_uris_and_percent_encoded_paths() {
        let _path = reference_path("file:///D:/ZZM/03.%20figures/table.png").unwrap();
        #[cfg(windows)]
        assert_eq!(_path, PathBuf::from(r"D:\ZZM\03. figures\table.png"));
        assert_eq!(
            percent_decode("figures/%E5%9B%BE%201.png"),
            "figures/图 1.png"
        );
    }

    #[test]
    #[cfg(windows)]
    fn normalizes_webview_windows_drive_paths() {
        assert_eq!(
            reference_path("/D:/ZZM/03.%20figures/table.png").unwrap(),
            PathBuf::from(r"D:\ZZM\03. figures\table.png")
        );
        assert_eq!(
            reference_path("/abs/path/D:/ZZM/03.%20figures/table.png").unwrap(),
            PathBuf::from(r"D:\ZZM\03. figures\table.png")
        );
    }

    #[tokio::test]
    async fn new_message_bindings_snapshot_and_reuse_immutable_versions() {
        let root =
            std::env::temp_dir().join(format!("wisp_resource_refs_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("figures")).unwrap();
        std::fs::write(root.join("figures/plot.png"), b"png-v1").unwrap();
        let store = Store::open(&root.join("store.sqlite")).await.unwrap();
        store
            .create_project("project", "Project", &root.to_string_lossy())
            .await
            .unwrap();
        store
            .create_frame("frame", "project", "OPERON", "model")
            .await
            .unwrap();

        let first = bind_new_message_resources(
            &store,
            &root,
            "project",
            "frame",
            2,
            "![plot](figures/plot.png)",
        )
        .await;
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].status, "ready");
        let artifact_id = first[0].artifact_id.clone().unwrap();
        let first_version = first[0].artifact_version_id.clone().unwrap();
        let version = store
            .latest_artifact_version(&artifact_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(version.id, first_version);
        assert!(root.join(version.storage_path).is_file());

        let repeated = bind_new_message_resources(
            &store,
            &root,
            "project",
            "frame",
            3,
            "![same](<figures/plot.png>)",
        )
        .await;
        assert_eq!(
            repeated[0].artifact_version_id.as_deref(),
            Some(first_version.as_str())
        );

        std::fs::write(root.join("figures/plot.png"), b"png-v2").unwrap();
        let changed = bind_new_message_resources(
            &store,
            &root,
            "project",
            "frame",
            4,
            "[updated](figures/plot.png)",
        )
        .await;
        assert_ne!(
            changed[0].artifact_version_id,
            repeated[0].artifact_version_id
        );

        let missing = bind_new_message_resources(
            &store,
            &root,
            "project",
            "frame",
            5,
            "[missing](figures/missing.md)",
        )
        .await;
        assert_eq!(missing[0].status, "unresolved");
        assert!(missing[0].error.is_some());

        std::fs::write(root.join("manuscript.docx"), b"PK\x03\x04docx-fixture").unwrap();
        std::fs::write(
            root.join("references.bib"),
            b"@article{wisp, title={Wisp Science}}\n",
        )
        .unwrap();
        let documents = bind_new_message_resources(
            &store,
            &root,
            "project",
            "frame",
            6,
            "[manuscript](manuscript.docx) [references](references.bib)",
        )
        .await;
        assert_eq!(documents.len(), 2);
        assert_eq!(documents[0].status, "ready");
        assert_eq!(documents[0].resource_kind, "docx");
        assert_eq!(documents[1].status, "ready");
        assert_eq!(documents[1].resource_kind, "text");
        for document in documents {
            let version = store
                .get_artifact_version(document.artifact_version_id.as_deref().unwrap())
                .await
                .unwrap()
                .unwrap();
            assert!(root.join(version.storage_path).is_file());
        }
    }
}
