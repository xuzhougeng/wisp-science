use super::session_export::artifact_provenance_for_path;
use super::AppState;
use base64::Engine;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::State;
use wisp_store::{LibraryItem, NewLibraryItem};

const MAX_FIGURE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_CODE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize)]
pub(super) struct LibraryItemDetail {
    #[serde(flatten)]
    item: LibraryItem,
    base64: Option<String>,
}

#[tauri::command]
pub(super) async fn list_library_items(
    state: State<'_, AppState>,
) -> Result<Vec<LibraryItem>, String> {
    state.library.list().await.map_err(|e| format!("{e}"))
}

#[tauri::command]
pub(super) async fn star_library_code(
    state: State<'_, AppState>,
    session_id: String,
    language: String,
    code: String,
) -> Result<LibraryItem, String> {
    if code.trim().is_empty() {
        return Err("Code is empty".into());
    }
    if code.len() > MAX_CODE_BYTES {
        return Err("Code is too large to add to the library".into());
    }
    let source = source_session(&state, &session_id).await?;
    state
        .library
        .insert(NewLibraryItem {
            kind: "code".into(),
            title: code_title(&code),
            language: Some(language.trim().to_string()).filter(|v| !v.is_empty()),
            code,
            content_type: None,
            content: None,
            source_project_id: source.project_id,
            source_project_name: source.project_name,
            source_session_id: source.session_id,
            source_session_title: source.session_title,
            source_path: None,
        })
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub(super) async fn star_library_text(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<LibraryItem, String> {
    if text.trim().is_empty() {
        return Err("Text is empty".into());
    }
    if text.len() > MAX_CODE_BYTES {
        return Err("Text is too large to add to the library".into());
    }
    let source = source_session(&state, &session_id).await?;
    state
        .library
        .insert(NewLibraryItem {
            kind: "text".into(),
            title: code_title(&text),
            language: None,
            code: text,
            content_type: None,
            content: None,
            source_project_id: source.project_id,
            source_project_name: source.project_name,
            source_session_id: source.session_id,
            source_session_title: source.session_title,
            source_path: None,
        })
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub(super) async fn star_library_figure(
    state: State<'_, AppState>,
    session_id: String,
    path: String,
    name: String,
) -> Result<LibraryItem, String> {
    let source = source_session(&state, &session_id).await?;
    let (_, project_root) = state
        .store
        .get_project(&source.project_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Source project no longer exists".to_string())?;
    let root = PathBuf::from(project_root);
    let real = wisp_tools::safety::validate_file_path(&root, &path)?;
    let content_type = image_content_type(&real)
        .ok_or_else(|| "Only image artifacts can be added to the library".to_string())?;
    let metadata = tokio::fs::metadata(&real)
        .await
        .map_err(|e| format!("Cannot read figure: {e}"))?;
    if !metadata.is_file() {
        return Err("Figure path is not a file".into());
    }
    if metadata.len() > MAX_FIGURE_BYTES {
        return Err("Figure is larger than the 32 MB library limit".into());
    }
    let content = tokio::fs::read(&real)
        .await
        .map_err(|e| format!("Cannot read figure: {e}"))?;
    let (code, language) =
        match artifact_provenance_for_path(&state.store, &session_id, &root, &path).await? {
            Some(provenance) => provenance.into_source(),
            None => (String::new(), String::new()),
        };
    let title = if name.trim().is_empty() {
        real.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Figure")
            .to_string()
    } else {
        name.trim().to_string()
    };
    state
        .library
        .insert(NewLibraryItem {
            kind: "figure".into(),
            title,
            language: Some(language).filter(|v| !v.is_empty()),
            code,
            content_type: Some(content_type.into()),
            content: Some(content),
            source_project_id: source.project_id,
            source_project_name: source.project_name,
            source_session_id: source.session_id,
            source_session_title: source.session_title,
            source_path: Some(normalize_source_path(&path)),
        })
        .await
        .map_err(|e| format!("{e}"))
}

#[tauri::command]
pub(super) async fn get_library_item(
    state: State<'_, AppState>,
    id: String,
) -> Result<LibraryItemDetail, String> {
    let detail = state
        .library
        .get(&id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Library item not found".to_string())?;
    Ok(LibraryItemDetail {
        item: detail.item,
        base64: detail
            .content
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes)),
    })
}

#[tauri::command]
pub(super) async fn delete_library_item(
    state: State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    state.library.delete(&id).await.map_err(|e| format!("{e}"))
}

struct SourceSession {
    project_id: String,
    project_name: String,
    session_id: String,
    session_title: String,
}

async fn source_session(state: &AppState, session_id: &str) -> Result<SourceSession, String> {
    let session = state
        .store
        .get_session_reference(session_id)
        .await
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| "Source session no longer exists".to_string())?;
    Ok(SourceSession {
        project_id: session.project_id,
        project_name: session.project_name,
        session_id: session.id,
        session_title: session.title,
    })
}

fn code_title(code: &str) -> String {
    let first = code
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("Code")
        .trim();
    let mut title: String = first.chars().take(80).collect();
    if first.chars().count() > 80 {
        title.push('…');
    }
    title
}

fn normalize_source_path(path: &str) -> String {
    path.strip_prefix("./")
        .or_else(|| path.strip_prefix(".\\"))
        .unwrap_or(path)
        .replace('\\', "/")
}

fn image_content_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "svg" => Some("image/svg+xml"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_image_types_are_stable() {
        assert_eq!(code_title("\n  print(1)\n"), "print(1)");
        assert_eq!(
            image_content_type(Path::new("plot.SVG")),
            Some("image/svg+xml")
        );
        assert_eq!(
            normalize_source_path(".\\figures\\plot.png"),
            "figures/plot.png"
        );
    }
}
