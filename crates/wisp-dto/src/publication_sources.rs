//! Choosing a passage of a research conversation as paper evidence: the
//! offsets, teaser and exact locator shared by the WebView source picker and
//! the GPUI desktop; the exact anchors of the advanced dialog; and the order
//! the manuscript outline lists nested items in.
use crate::{PublicationItemInfo, PublicationSourceChoice};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicationEvidenceSource {
    pub kind: &'static str,
    pub id: String,
    pub label: String,
}

pub fn manuscript_rows(items: &[PublicationItemInfo]) -> Vec<(PublicationItemInfo, usize)> {
    fn visit(
        parent: Option<&str>,
        children: &HashMap<Option<String>, Vec<PublicationItemInfo>>,
        seen: &mut HashSet<String>,
        depth: usize,
        rows: &mut Vec<(PublicationItemInfo, usize)>,
    ) {
        let key = parent.map(str::to_string);
        let Some(items) = children.get(&key) else {
            return;
        };
        for item in items {
            if !seen.insert(item.id.clone()) {
                continue;
            }
            rows.push((item.clone(), depth));
            visit(Some(&item.id), children, seen, depth + 1, rows);
        }
    }

    let mut children = HashMap::<Option<String>, Vec<PublicationItemInfo>>::new();
    for item in items {
        children
            .entry(item.parent_item_id.clone())
            .or_default()
            .push(item.clone());
    }
    for siblings in children.values_mut() {
        siblings.sort_by(|left, right| {
            left.ordinal
                .cmp(&right.ordinal)
                .then_with(|| left.title.cmp(&right.title))
        });
    }
    let mut rows = Vec::with_capacity(items.len());
    let mut seen = HashSet::new();
    visit(None, &children, &mut seen, 0, &mut rows);
    for item in items {
        if seen.insert(item.id.clone()) {
            rows.push((item.clone(), 0));
            visit(Some(&item.id), &children, &mut seen, 1, &mut rows);
        }
    }
    rows
}

pub fn precise_evidence_source(
    kind: &str,
    frame_id: &str,
    message_seq: &str,
    byte_start: &str,
    byte_end: &str,
    tool_call_id: &str,
    exact_id: &str,
) -> Result<PublicationEvidenceSource, String> {
    let exact_id = exact_id.trim();
    match kind {
        "execution_log" | "code_cell" | "external_resource" => {
            if exact_id.is_empty() {
                return Err("An exact source ID is required.".into());
            }
            let (kind, label) = match kind {
                "execution_log" => ("execution_log", "Execution log"),
                "code_cell" => ("code_cell", "Code cell"),
                _ => ("external_resource", "External resource"),
            };
            Ok(PublicationEvidenceSource {
                kind,
                id: exact_id.into(),
                label: format!("{label}: {exact_id}"),
            })
        }
        "message_span" => {
            let seq = message_seq
                .trim()
                .parse::<i64>()
                .map_err(|_| "Message sequence must be a positive integer.")?;
            let start = byte_start
                .trim()
                .parse::<usize>()
                .map_err(|_| "Span start must be a byte offset.")?;
            let end = byte_end
                .trim()
                .parse::<usize>()
                .map_err(|_| "Span end must be a byte offset.")?;
            if frame_id.trim().is_empty() || seq < 1 || start >= end {
                return Err(
                    "MessageSpan requires a frame, message sequence, and valid range.".into(),
                );
            }
            Ok(PublicationEvidenceSource {
                kind: "message_span",
                id: serde_json::to_string(&serde_json::json!({
                    "byte_end": end,
                    "byte_start": start,
                    "frame_id": frame_id.trim(),
                    "message_seq": seq,
                }))
                .map_err(|error| error.to_string())?,
                label: format!("Message {seq} bytes {start}–{end}"),
            })
        }
        "tool_call" => {
            let seq = message_seq
                .trim()
                .parse::<i64>()
                .map_err(|_| "Message sequence must be a positive integer.")?;
            if frame_id.trim().is_empty() || seq < 1 || tool_call_id.trim().is_empty() {
                return Err("ToolCall requires a frame, message sequence, and call ID.".into());
            }
            Ok(PublicationEvidenceSource {
                kind: "tool_call",
                id: serde_json::to_string(&serde_json::json!({
                    "frame_id": frame_id.trim(),
                    "message_seq": seq,
                    "tool_call_id": tool_call_id.trim(),
                }))
                .map_err(|error| error.to_string())?,
                label: format!("Tool result: {}", tool_call_id.trim()),
            })
        }
        _ => Err("Unsupported precise evidence kind.".into()),
    }
}

/// Selection offsets count UTF-16 code units (the DOM's and GPUI's text
/// input both do); persisted anchors count UTF-8 bytes. Reject offsets inside
/// a surrogate pair rather than moving the range.
pub fn utf16_byte_offset(text: &str, offset: u32) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += ch.len_utf16() as u32;
    }
    (units == offset).then_some(text.len())
}

/// A selection of `text` given in UTF-16 units, as a non-empty UTF-8 byte
/// range; `None` for an empty selection or one that splits a character.
pub fn utf16_byte_range(text: &str, start: u32, end: u32) -> Option<(usize, usize)> {
    let start = utf16_byte_offset(text, start)?;
    let end = utf16_byte_offset(text, end)?;
    (start < end).then_some((start, end))
}

/// Card teaser for a transcript source: drops Markdown emphasis, code and
/// heading markers and collapses whitespace. The preview keeps the exact text.
pub fn source_snippet(text: &str) -> String {
    let cleaned: String = text.chars().filter(|ch| !matches!(ch, '*' | '`')).collect();
    let mut snippet = String::new();
    for word in cleaned
        .split_whitespace()
        .filter(|word| !word.chars().all(|ch| ch == '#'))
    {
        if !snippet.is_empty() {
            snippet.push(' ');
        }
        snippet.push_str(word);
        if snippet.chars().count() >= 160 {
            break;
        }
    }
    snippet.chars().take(160).collect()
}

/// The exact `message_span` locator for a passage of a conversation message
/// offered by the source picker: `range` in UTF-8 bytes of its preview text,
/// or the whole preview when nothing is selected. Canonical JSON (sorted keys),
/// as the store requires. `None` for a choice that is not a message.
pub fn message_span_locator(
    source: &PublicationSourceChoice,
    range: Option<(usize, usize)>,
) -> Option<String> {
    let text = source.text.as_deref()?;
    let (start, end) = range.unwrap_or((0, text.len()));
    let mut locator = BTreeMap::new();
    locator.insert("byte_end", serde_json::Value::from(end));
    locator.insert("byte_start", serde_json::Value::from(start));
    locator.insert("frame_id", source.frame_id.clone()?.into());
    locator.insert("message_seq", source.message_seq?.into());
    if let Some(hash) = &source.text_sha256 {
        locator.insert("message_content_sha256", hash.clone().into());
    }
    serde_json::to_string(&locator).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, parent: Option<&str>, ordinal: i64) -> PublicationItemInfo {
        PublicationItemInfo {
            id: id.into(),
            revision_id: "revision".into(),
            parent_item_id: parent.map(str::to_string),
            kind: "section".into(),
            title: id.into(),
            content: String::new(),
            ordinal,
        }
    }

    #[test]
    fn manuscript_rows_follow_parent_order_and_keep_orphans() {
        let rows = manuscript_rows(&[
            item("child", Some("root"), 0),
            item("later", None, 2),
            item("root", None, 1),
            item("orphan", Some("missing"), 0),
        ]);
        assert_eq!(
            rows.iter()
                .map(|(item, depth)| (item.id.as_str(), *depth))
                .collect::<Vec<_>>(),
            [("root", 0), ("child", 1), ("later", 0), ("orphan", 0),]
        );
    }

    #[test]
    fn message_span_source_uses_canonical_exact_locator() {
        let source =
            precise_evidence_source("message_span", "frame", "8", "2", "11", "", "").unwrap();
        assert_eq!(source.kind, "message_span");
        assert_eq!(
            source.id,
            r#"{"byte_end":11,"byte_start":2,"frame_id":"frame","message_seq":8}"#
        );
    }

    #[test]
    fn transcript_selection_preserves_utf8_boundaries() {
        let text = "A水稻🌱结果";
        assert_eq!(utf16_byte_offset(text, 0), Some(0));
        assert_eq!(utf16_byte_offset(text, 3), Some(7));
        assert_eq!(utf16_byte_offset(text, 4), None);
        assert_eq!(utf16_byte_offset(text, 5), Some(11));
        assert_eq!(utf16_byte_offset(text, 7), Some(text.len()));
        assert_eq!(utf16_byte_offset(text, 8), None);
        assert_eq!(utf16_byte_range(text, 1, 5), Some((1, 11)));
        assert_eq!(utf16_byte_range(text, 3, 3), None);
        assert_eq!(utf16_byte_range(text, 1, 4), None);
    }

    #[test]
    fn source_snippet_drops_markdown_markers_and_bounds_length() {
        assert_eq!(
            source_snippet("## 确认了。**`celltype_go/`** 就是\n\n这张图 `run_in_context`"),
            "确认了。celltype_go/ 就是 这张图 run_in_context"
        );
        assert_eq!(source_snippet("  \n "), "");
        assert_eq!(source_snippet(&"水稻 ".repeat(200)).chars().count(), 160);
    }

    #[test]
    fn a_message_locator_is_canonical_and_defaults_to_the_whole_preview() {
        let mut choice = PublicationSourceChoice {
            kind: "message_span".into(),
            id: "m1".into(),
            title: "Rice".into(),
            detail: "assistant".into(),
            text: Some("采用 🌱".into()),
            text_sha256: Some("ab".repeat(32)),
            frame_id: Some("frame".into()),
            message_seq: Some(4),
        };
        assert_eq!(
            message_span_locator(&choice, Some((0, 6))).unwrap(),
            format!(
                r#"{{"byte_end":6,"byte_start":0,"frame_id":"frame","message_content_sha256":"{}","message_seq":4}}"#,
                "ab".repeat(32)
            )
        );
        choice.text_sha256 = None;
        assert_eq!(
            message_span_locator(&choice, None).unwrap(),
            r#"{"byte_end":11,"byte_start":0,"frame_id":"frame","message_seq":4}"#
        );
        choice.text = None;
        assert_eq!(message_span_locator(&choice, None), None);
    }
}
