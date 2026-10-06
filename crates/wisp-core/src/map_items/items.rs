//! Where a run's items come from, and how one item becomes text for the model.
//! Sources are resolved once, up front, into the frozen list in the manifest.

use super::decide::Verdict;
use super::store::{valid_run_id, Item, RowStatus, RunDir, MAX_ITEMS};
use serde_json::{json, Map, Value};
use std::path::Path;
use wisp_tools::ToolEnv;

const MAX_SOURCE_BYTES: u64 = 50 * 1024 * 1024;

/// Resolve the `items` argument. Exactly one source key is allowed, and a
/// source larger than [`MAX_ITEMS`] is an error, never a silent truncation.
/// Returns the items plus a one-line description for the approval prompt.
pub fn resolve(spec: &Value, env: &dyn ToolEnv) -> Result<(Vec<Item>, String), String> {
    let obj = spec
        .as_object()
        .ok_or("'items' must be an object with one of: glob, paths, jsonl, run")?;
    let present: Vec<&str> = ["glob", "paths", "jsonl", "run"]
        .into_iter()
        .filter(|key| obj.contains_key(*key))
        .collect();
    let [kind] = present.as_slice() else {
        return Err("'items' needs exactly one of: glob, paths, jsonl, run".into());
    };
    let (items, what) = match *kind {
        "glob" => glob_items(str_of(obj, "glob")?, env)?,
        "paths" => path_items(obj.get("paths"), env)?,
        "jsonl" => jsonl_items(obj, env)?,
        _ => run_items(obj, env)?,
    };
    let (items, dropped) = dedupe(items);
    if items.is_empty() {
        return Err(format!("{what} matched no items"));
    }
    if items.len() > MAX_ITEMS {
        return Err(format!(
            "{what} has {} items; the limit is {MAX_ITEMS} per run. Narrow it or split it into several runs.",
            items.len()
        ));
    }
    let note = if dropped > 0 {
        format!(", {dropped} duplicate(s) dropped")
    } else {
        String::new()
    };
    Ok((
        items.clone(),
        format!("{} item(s) from {what}{note}", items.len()),
    ))
}

fn str_of<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    obj.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("'items.{key}' must be a non-empty string"))
}

fn dedupe(items: Vec<Item>) -> (Vec<Item>, usize) {
    let mut seen = std::collections::HashSet::new();
    let before = items.len();
    let items: Vec<Item> = items
        .into_iter()
        .filter(|item| seen.insert(item.id.clone()))
        .collect();
    let dropped = before - items.len();
    (items, dropped)
}

/// Project-relative id with `/` separators when the file is inside the root.
fn file_id(root: &Path, path: &Path) -> String {
    let shown = path.strip_prefix(root).unwrap_or(path);
    shown.to_string_lossy().replace('\\', "/")
}

fn glob_items(pattern: &str, env: &dyn ToolEnv) -> Result<(Vec<Item>, String), String> {
    let root = env.project_root();
    let full = if Path::new(pattern).is_absolute() {
        pattern.to_string()
    } else {
        format!(
            "{}/{}",
            glob::Pattern::escape(&root.to_string_lossy()),
            pattern
        )
    };
    let walker = glob::glob(&full).map_err(|e| format!("bad glob '{pattern}': {e}"))?;
    let mut paths: Vec<_> = walker
        .filter_map(Result::ok)
        .filter(|p| p.is_file())
        .collect();
    paths.sort();
    let mut items = Vec::new();
    for path in paths {
        // A restricted conversation may only read inside the project root.
        let resolved = env.resolve_read_path(&path.to_string_lossy(), false)?;
        items.push(Item {
            id: file_id(root, &resolved),
            payload: None,
        });
    }
    Ok((items, format!("glob '{pattern}'")))
}

fn path_items(paths: Option<&Value>, env: &dyn ToolEnv) -> Result<(Vec<Item>, String), String> {
    let list = paths
        .and_then(Value::as_array)
        .ok_or("'items.paths' must be an array of file paths")?;
    let mut items = Vec::new();
    for entry in list {
        let path = entry
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .ok_or("'items.paths' entries must be non-empty strings")?;
        let resolved = env.resolve_read_path(path, false)?;
        if !resolved.is_file() {
            return Err(format!("'{path}' is not a file"));
        }
        items.push(Item {
            id: file_id(env.project_root(), &resolved),
            payload: None,
        });
    }
    Ok((items, "an explicit path list".into()))
}

fn jsonl_items(obj: &Map<String, Value>, env: &dyn ToolEnv) -> Result<(Vec<Item>, String), String> {
    let name = str_of(obj, "jsonl")?;
    let path = env.resolve_read_path(name, false)?;
    if path.metadata().map_err(|e| format!("{name}: {e}"))?.len() > MAX_SOURCE_BYTES {
        return Err(format!("{name} is larger than 50 MiB"));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{name}: {e}"))?;
    let id_field = obj.get("id_field").and_then(Value::as_str);
    let filter = obj.get("where").and_then(Value::as_object);
    let mut items = Vec::new();
    for (index, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        let row: Value = serde_json::from_str(line)
            .map_err(|e| format!("{name} line {}: not JSON ({e})", index + 1))?;
        let Some(fields) = row.as_object() else {
            return Err(format!("{name} line {}: expected a JSON object", index + 1));
        };
        if filter.is_some_and(|f| f.iter().any(|(k, v)| fields.get(k) != Some(v))) {
            continue;
        }
        let id = id_field
            .into_iter()
            .chain(["id", "item"])
            .find_map(|key| match fields.get(key) {
                Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
                Some(Value::Number(n)) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_else(|| format!("line-{}", index + 1));
        items.push(Item {
            id,
            payload: Some(row),
        });
    }
    Ok((items, format!("JSONL file '{name}'")))
}

/// Rows of an earlier run as new items, so a second pass (for example a
/// tool-using worker over just the uncertain papers) never touches the others.
fn run_items(obj: &Map<String, Value>, env: &dyn ToolEnv) -> Result<(Vec<Item>, String), String> {
    let id = str_of(obj, "run")?;
    if !valid_run_id(id) {
        return Err(format!("invalid run id '{id}'"));
    }
    let (dir, manifest) = RunDir::open(env.project_root(), id)?;
    let wanted: Vec<Verdict> = match obj.get("verdict") {
        None => Vec::new(),
        Some(Value::Array(list)) => list
            .iter()
            .map(|v| {
                v.as_str()
                    .and_then(Verdict::parse)
                    .ok_or("'items.verdict' entries are pass, reject or uncertain")
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err("'items.verdict' must be an array".into()),
    };
    let rows = dir.load_rows();
    let mut items = Vec::new();
    for item in &manifest.items {
        let Some(row) = rows.get(&item.id).filter(|r| r.status == RowStatus::Ok) else {
            continue;
        };
        let verdict = row.verdict(&manifest.spec);
        if !wanted.is_empty() && !verdict.is_some_and(|v| wanted.contains(&v)) {
            continue;
        }
        items.push(Item {
            id: item.id.clone(),
            payload: Some(json!({
                "item": row.item,
                "verdict": verdict.map(Verdict::as_str),
                "data": row.data,
                "scores": row.scores,
                "answers": row.answers,
            })),
        });
    }
    let filter = if wanted.is_empty() {
        String::new()
    } else {
        let names: Vec<_> = wanted.iter().map(|v| v.as_str()).collect();
        format!(" ({})", names.join("/"))
    };
    Ok((items, format!("rows of run {id}{filter}")))
}

/// Keep `head` characters and the tail of `text` when it exceeds `max` characters.
pub fn clip_chars(text: &str, max: usize) -> (String, bool) {
    let total = text.chars().count();
    if total <= max {
        return (text.to_string(), false);
    }
    let head = max * 3 / 4;
    let tail = max - head;
    let head_end = text.char_indices().nth(head).map_or(text.len(), |(i, _)| i);
    let tail_start = text
        .char_indices()
        .nth(total - tail)
        .map_or(text.len(), |(i, _)| i);
    (
        format!(
            "{}\n[... {} characters omitted ...]\n{}",
            &text[..head_end],
            total - max,
            &text[tail_start..]
        ),
        true,
    )
}

/// The text of one item and whether it was clipped. Files go through the same
/// local converters as `read` (text PDFs, Office, EPUB, ...); structured items
/// are their JSON. An item with nothing to read is an item failure.
pub async fn load_text(
    item: &Item,
    max_chars: usize,
    env: &dyn ToolEnv,
) -> Result<(String, usize, bool), String> {
    let text = match &item.payload {
        Some(payload) => serde_json::to_string_pretty(payload).unwrap_or_default(),
        None => {
            let path = env.resolve_read_path(&item.id, false)?;
            tokio::task::spawn_blocking(move || read_document(&path))
                .await
                .map_err(|e| format!("reader crashed: {e}"))??
        }
    };
    let chars = text.chars().count();
    let (clipped, truncated) = clip_chars(&text, max_chars);
    Ok((clipped, chars, truncated))
}

fn read_document(path: &Path) -> Result<String, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot read file: {e}"))?;
    if meta.len() > MAX_SOURCE_BYTES {
        return Err("file is larger than 50 MiB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read file: {e}"))?;
    let text = match wisp_tools::read::document_markdown(path, &bytes) {
        Some(Ok(markdown)) => markdown,
        Some(Err(e)) => return Err(format!("cannot convert document: {e}")),
        None if bytes.starts_with(b"%PDF-") || bytes[..bytes.len().min(8192)].contains(&0) => {
            return Err("binary file with no text converter".into())
        }
        None => String::from_utf8_lossy(&bytes).into_owned(),
    };
    if text.trim().is_empty() {
        return Err("no extractable text (a scanned PDF? OCR is not supported)".into());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_keeps_head_and_tail_on_char_boundaries() {
        let text = "头".repeat(100) + &"尾".repeat(100);
        let (clipped, truncated) = clip_chars(&text, 40);
        assert!(truncated);
        assert!(clipped.starts_with(&"头".repeat(30)));
        assert!(clipped.ends_with(&"尾".repeat(10)));
        assert!(clipped.contains("160 characters omitted"));
        assert_eq!(clip_chars("short", 40), ("short".into(), false));
    }
}
