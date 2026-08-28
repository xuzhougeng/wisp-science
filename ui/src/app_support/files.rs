use super::*;

pub(crate) fn native_drop_remote_target_value(payload: JsValue) -> Option<(String, String)> {
    let value = native_drop_remote_target(payload);
    let parsed = serde_wasm_bindgen::from_value::<serde_json::Value>(value).ok()?;
    let context_id = parsed.get("contextId")?.as_str()?.to_string();
    let destination_dir = parsed.get("destinationDir")?.as_str()?.to_string();
    if context_id.is_empty() || context_id == "local" {
        None
    } else {
        Some((context_id, destination_dir))
    }
}

pub(crate) fn upload_to_remote_context(
    context_id: String,
    destination_dir: String,
    source_paths: Option<Vec<String>>,
    uploading: RwSignal<bool>,
    refresh_tick: RwSignal<u32>,
) {
    if uploading.get_untracked() {
        return;
    }
    uploading.set(true);
    spawn_local(async move {
        let mut payload = serde_json::json!({
            "contextId": context_id,
            "destinationDir": destination_dir,
        });
        if let Some(paths) = source_paths {
            payload["sourcePaths"] = serde_json::json!(paths);
        }
        let result = invoke_checked("upload_to_context", to_value(&payload).unwrap()).await;
        uploading.set(false);
        match result {
            Ok(value) => match serde_wasm_bindgen::from_value::<Vec<UploadToContextItem>>(value) {
                Ok(items) if items.is_empty() => {}
                Ok(items) => {
                    let loc = use_locale().get_untracked();
                    show_toast(&tf(
                        loc,
                        "files.upload_started",
                        &[("n", &items.len().to_string())],
                    ));
                    refresh_tick.update(|tick| *tick = tick.saturating_add(1));
                }
                Err(error) => show_toast(&error.to_string()),
            },
            Err(error) => {
                let loc = use_locale().get_untracked();
                show_toast(&localize_backend(loc, &js_error_text(error)));
            }
        }
    });
}

pub(crate) fn refresh_dir(cwd: RwSignal<String>, entries: RwSignal<Vec<DirEntry>>) {
    spawn_local(async move {
        let path = cwd.get();
        let v = invoke(
            "list_dir",
            to_value(&serde_json::json!({ "path": path })).unwrap(),
        )
        .await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<DirEntry>>(v) {
            entries.set(list);
        }
    });
}

pub(crate) fn refresh_remote_dir(
    context_id: String,
    cwd: RwSignal<String>,
    entries: RwSignal<Vec<DirEntry>>,
    loading: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    active_source: RwSignal<String>,
) {
    let requested_path = cwd.get_untracked();
    entries.set(vec![]);
    error.set(None);
    loading.set(true);
    spawn_local(async move {
        let result = invoke_checked(
            "list_remote_dir",
            to_value(&serde_json::json!({
                "contextId": context_id.clone(),
                "path": requested_path.clone(),
            }))
            .unwrap(),
        )
        .await;
        if active_source.get_untracked() != context_id || cwd.get_untracked() != requested_path {
            return;
        }
        loading.set(false);
        match result {
            Ok(value) => match serde_wasm_bindgen::from_value::<DirectoryListing>(value) {
                Ok(listing) => {
                    cwd.set(listing.path);
                    entries.set(listing.entries);
                }
                Err(parse_error) => error.set(Some(parse_error.to_string())),
            },
            Err(invoke_error) => error.set(Some(js_error_text(invoke_error))),
        }
    });
}

pub(crate) fn refresh_active_file_dir(
    source: RwSignal<String>,
    local_cwd: RwSignal<String>,
    local_entries: RwSignal<Vec<DirEntry>>,
    remote_cwd: RwSignal<String>,
    remote_entries: RwSignal<Vec<DirEntry>>,
    remote_loading: RwSignal<bool>,
    remote_error: RwSignal<Option<String>>,
) {
    let context_id = source.get_untracked();
    if context_id == "local" {
        refresh_dir(local_cwd, local_entries);
    } else {
        refresh_remote_dir(
            context_id,
            remote_cwd,
            remote_entries,
            remote_loading,
            remote_error,
            source,
        );
    }
}

pub(crate) fn refresh_file_search(query: RwSignal<String>, hits: RwSignal<Vec<FileSearchHit>>) {
    spawn_local(async move {
        let q = query.get().trim().to_string();
        if q.is_empty() {
            hits.set(vec![]);
            return;
        }
        let v = invoke(
            "search_files",
            to_value(&serde_json::json!({ "query": q, "limit": 200 })).unwrap(),
        )
        .await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<FileSearchHit>>(v) {
            hits.set(list);
        }
    });
}

pub(crate) fn refresh_artifact_search(query: RwSignal<String>, hits: RwSignal<Vec<ArtifactInfo>>) {
    spawn_local(async move {
        let q = query.get().trim().to_string();
        let v = invoke(
            "search_artifacts",
            to_value(&serde_json::json!({ "query": q, "limit": 12 })).unwrap(),
        )
        .await;
        if let Ok(list) = serde_wasm_bindgen::from_value::<Vec<ArtifactInfo>>(v) {
            hits.set(list);
        }
    });
}

pub(crate) fn artifact_badge(kind: &str, name: &str) -> String {
    let raw = name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .filter(|ext| !ext.is_empty() && ext.len() <= 10)
        .or_else(|| kind.rsplit('/').next())
        .unwrap_or("file");
    raw.to_uppercase()
}

pub(crate) fn stored_artifact_path(path: &str) -> String {
    path.strip_prefix("file://").unwrap_or(path).to_string()
}

pub(crate) fn contains_search(q: &str, parts: &[&str]) -> bool {
    q.is_empty() || parts.iter().any(|s| s.to_lowercase().contains(q))
}

pub(crate) type ModalArtifact = (String, String, String);

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CenterFileTab {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) kind: String,
}

impl CenterFileTab {
    pub(crate) fn new(path: String, name: String, kind: String) -> Self {
        Self { path, name, kind }
    }

    pub(crate) fn from_path(path: String) -> Self {
        let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
        let kind = file_kind(&path).unwrap_or("text").to_string();
        Self { path, name, kind }
    }
}

/// Keys a live file-change event can use to address an open center tab. Tools
/// may report either the relative argument the model supplied or the resolved
/// absolute path; normalize both POSIX and Windows separators for matching.
pub(crate) fn file_change_refresh_keys(path: &str, project_root: Option<&str>) -> Vec<String> {
    let mut keys = Vec::new();
    let mut push = |value: String| {
        if !value.is_empty() && !keys.contains(&value) {
            keys.push(value);
        }
    };
    push(path.to_string());
    let normalized = path.replace('\\', "/");
    push(normalized.clone());
    if let Some(relative) = normalized.strip_prefix("./") {
        push(relative.to_string());
        push(relative.replace('/', "\\"));
    }
    if let Some(root) = project_root {
        let normalized_root = root.replace('\\', "/");
        let normalized_root = normalized_root.trim_end_matches('/');
        if let Some(relative) = normalized.strip_prefix(normalized_root).and_then(|tail| {
            tail.strip_prefix('/')
                .filter(|relative| !relative.is_empty())
        }) {
            push(relative.to_string());
            push(relative.replace('/', "\\"));
        }
    }
    keys
}

#[cfg(test)]
mod file_change_refresh_keys_tests {
    use super::file_change_refresh_keys;

    #[test]
    fn matches_relative_and_absolute_workspace_paths() {
        assert_eq!(
            file_change_refresh_keys("analysis.R", Some("/work/project")),
            ["analysis.R"]
        );
        assert!(
            file_change_refresh_keys("./analysis.R", Some("/work/project"))
                .contains(&"analysis.R".to_string())
        );
        let unix = file_change_refresh_keys("/work/project/src/analysis.R", Some("/work/project"));
        assert!(unix.contains(&"src/analysis.R".to_string()));

        let windows =
            file_change_refresh_keys(r"C:\work\project\src\analysis.R", Some(r"C:\work\project"));
        assert!(windows.contains(&"src/analysis.R".to_string()));
        assert!(windows.contains(&r"src\analysis.R".to_string()));
    }
}

pub(crate) fn open_workspace_file(path: String, modal_artifact: RwSignal<Option<ModalArtifact>>) {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();
    let kind = file_kind(&path).unwrap_or("text").to_string();
    modal_artifact.set(Some((path, name, kind)));
}

pub(crate) fn modal_image_nav_targets(
    artifacts: &[Artifact],
    current_path: &str,
    current_kind: &str,
) -> (Option<ModalArtifact>, Option<ModalArtifact>) {
    if current_kind != "image" {
        return (None, None);
    }
    let images = artifacts
        .iter()
        .filter_map(|artifact| match &artifact.data {
            PreviewData::File { path, kind } if kind == "image" => {
                Some((path.clone(), artifact.name.clone(), kind.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let Some(index) = images.iter().position(|(path, _, _)| path == current_path) else {
        return (None, None);
    };
    let prev = index
        .checked_sub(1)
        .and_then(|idx| images.get(idx).cloned());
    let next = images.get(index + 1).cloned();
    (prev, next)
}

pub(crate) const ALL_RIGHT_TABS: [RightTab; 8] = [
    RightTab::Artifacts,
    RightTab::Agents,
    RightTab::Notebook,
    RightTab::Highlights,
    RightTab::File,
    RightTab::Provenance,
    RightTab::Hosts,
    RightTab::SideChat,
];

pub(crate) const DEFAULT_RIGHT_TABS: [RightTab; 4] = [
    RightTab::Artifacts,
    RightTab::Agents,
    RightTab::File,
    RightTab::Hosts,
];

pub(crate) fn scroll_active_right_tab_into_view() {
    request_animation_frame(|| {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let Ok(Some(scroller)) = document.query_selector(".rightpane .rp-tab-scroll") else {
            return;
        };
        let Ok(Some(active)) = document.query_selector(".rightpane .rp-tab.active") else {
            return;
        };
        let Some(active_wrap) = active.parent_element() else {
            return;
        };
        let Ok(scroller) = scroller.dyn_into::<web_sys::HtmlElement>() else {
            return;
        };
        let Ok(active_wrap) = active_wrap.dyn_into::<web_sys::HtmlElement>() else {
            return;
        };

        let tab_left = active_wrap.offset_left();
        let tab_right = tab_left + active_wrap.offset_width();
        let viewport_left = scroller.scroll_left();
        let viewport_width = scroller.client_width();
        let viewport_right = viewport_left + viewport_width;
        if tab_left < viewport_left {
            scroller.set_scroll_left((tab_left - 4).max(0));
        } else if tab_right > viewport_right {
            scroller.set_scroll_left(tab_right - viewport_width + 4);
        }
    });
}

pub(crate) fn ensure_right_tab(
    tab: RightTab,
    show_right: RwSignal<bool>,
    open_right_tabs: RwSignal<Vec<RightTab>>,
    right_tab: RwSignal<RightTab>,
) {
    show_right.set(true);
    open_right_tabs.update(|tabs| {
        if !tabs.iter().any(|t| *t == tab) {
            tabs.push(tab);
        }
    });
    right_tab.set(tab);
}

pub(crate) fn close_right_tab(
    tab: RightTab,
    show_right: RwSignal<bool>,
    open_right_tabs: RwSignal<Vec<RightTab>>,
    right_tab: RwSignal<RightTab>,
) {
    let was_active = right_tab.get_untracked() == tab;
    let prev_idx = open_right_tabs
        .get_untracked()
        .iter()
        .position(|t| *t == tab);
    open_right_tabs.update(|tabs| tabs.retain(|t| *t != tab));
    let remaining = open_right_tabs.get_untracked();
    if remaining.is_empty() {
        show_right.set(false);
        return;
    }
    if was_active {
        let pick = prev_idx
            .map(|i| if i > 0 { i - 1 } else { 0 })
            .unwrap_or(0)
            .min(remaining.len() - 1);
        right_tab.set(remaining[pick]);
    }
}

pub(crate) fn reveal_in_files(
    path: &str,
    file_source: RwSignal<String>,
    file_cwd: RwSignal<String>,
    file_query: RwSignal<String>,
    file_entries: RwSignal<Vec<DirEntry>>,
    show_right: RwSignal<bool>,
    open_right_tabs: RwSignal<Vec<RightTab>>,
    right_tab: RwSignal<RightTab>,
) {
    file_source.set("local".into());
    file_query.set(String::new());
    file_cwd.set(parent_path(path));
    refresh_dir(file_cwd, file_entries);
    ensure_right_tab(RightTab::File, show_right, open_right_tabs, right_tab);
}

pub(crate) fn file_dir_label(path: &str) -> String {
    let p = parent_path(path);
    if p == "." {
        String::new()
    } else {
        format!("{p}/")
    }
}

pub(crate) fn toggle_workspace_path(selected_paths: RwSignal<HashSet<String>>, path: &str) {
    selected_paths.update(|selected| {
        if !selected.insert(path.to_string()) {
            selected.remove(path);
        }
    });
}

fn is_absolute_workspace_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.starts_with('/')
        || matches!(
            normalized.as_bytes(),
            [drive, b':', b'/', ..] if drive.is_ascii_alphabetic()
        )
}

fn strip_workspace_root<'a>(root: &str, path: &'a str) -> Option<&'a str> {
    let windows = root.contains('\\')
        || root.starts_with("//")
        || matches!(root.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic());
    if path.len() == root.len()
        && if windows {
            path.eq_ignore_ascii_case(root)
        } else {
            path == root
        }
    {
        return Some("");
    }
    let prefix = if root == "/" {
        "/".to_string()
    } else {
        format!("{root}/")
    };
    let head = path.get(..prefix.len())?;
    let matches = if windows {
        head.eq_ignore_ascii_case(&prefix)
    } else {
        head == prefix
    };
    matches.then(|| &path[prefix.len()..])
}

pub(crate) fn workspace_relative_path(root: &str, path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let normalized = path.replace('\\', "/");
    if !is_absolute_workspace_path(path) {
        return Some(normalized.trim_start_matches("./").to_string());
    }
    let normalized_root = root.replace('\\', "/");
    let normalized_root = if normalized_root == "/" {
        normalized_root
    } else {
        normalized_root.trim_end_matches('/').to_string()
    };
    if normalized_root.is_empty() {
        return None;
    }
    strip_workspace_root(&normalized_root, &normalized)
        .map(|relative| relative.trim_start_matches('/').to_string())
}

pub(crate) const FILE_SORT_NAME: &str = "name";
pub(crate) const FILE_SORT_SIZE: &str = "size";
pub(crate) const FILE_SORT_MODIFIED: &str = "modified";
pub(crate) const FILE_SORT_PREF: &str = "wisp-file-sort";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileRowMeta {
    Size(u64),
    Modified(u64),
}

pub(crate) fn sort_dir_entries(entries: &mut [DirEntry], sort: &str) {
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir).then_with(|| match sort {
            FILE_SORT_SIZE => b
                .size
                .cmp(&a.size)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            FILE_SORT_MODIFIED => b
                .modified_unix_millis
                .unwrap_or(0)
                .cmp(&a.modified_unix_millis.unwrap_or(0))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        })
    });
}

pub(crate) fn file_row_meta(entry: &DirEntry, sort: &str) -> Option<FileRowMeta> {
    if sort == FILE_SORT_MODIFIED {
        return entry
            .modified_unix_millis
            .filter(|ms| *ms > 0)
            .map(FileRowMeta::Modified);
    }
    (!entry.is_dir).then_some(FileRowMeta::Size(entry.size))
}

pub(crate) fn file_row_meta_view(entry: &DirEntry, sort: &str) -> impl IntoView {
    match file_row_meta(entry, sort) {
        Some(FileRowMeta::Size(n)) => {
            view! { <span class="fb-size">{format_bytes(n)}</span> }.into_view()
        }
        Some(FileRowMeta::Modified(ms)) => {
            view! { <span class="fb-size">{format_message_time(ms as i64)}</span> }.into_view()
        }
        None => ().into_view(),
    }
}

pub(crate) fn FileSortControl(
    sort_by: RwSignal<String>,
    menu_open: RwSignal<bool>,
) -> impl IntoView {
    let locale = use_locale();
    window_capture_escape(move || {
        if !menu_open.get_untracked() {
            return false;
        }
        menu_open.set(false);
        true
    });
    view! {
        <div class="fb-sort">
            <button type="button"
                data-testid="files-sort"
                class:active=move || menu_open.get()
                aria-expanded=move || menu_open.get().to_string()
                aria-haspopup="menu"
                title=move || t(locale.get(), "files.sort")
                aria-label=move || t(locale.get(), "files.sort")
                on:click=move |ev: web_sys::MouseEvent| {
                    ev.stop_propagation();
                    menu_open.update(|open| *open = !*open);
                }>
                {compose_icon("sort")}
                <span>{move || t(locale.get(), "files.sort")}</span>
            </button>
            {move || menu_open.get().then(|| {
                let loc = locale.get();
                let opts = [
                    (FILE_SORT_NAME, "files.sort_name"),
                    (FILE_SORT_SIZE, "files.sort_size"),
                    (FILE_SORT_MODIFIED, "files.sort_modified"),
                ];
                view! {
                    <div class="fb-sort-backdrop" on:click=move |_| menu_open.set(false)></div>
                    <div class="fb-sort-menu" role="menu" on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="fb-sort-label">{t(loc, "files.sort_by")}</div>
                        <div class="fb-sort-opts">
                            {opts.into_iter().map(|(val, key)| view! {
                                <button type="button" class="fb-sort-opt" role="menuitem"
                                    class:active=move || sort_by.get().as_str() == val
                                    on:click=move |_| {
                                        sort_by.set(val.into());
                                        save_view_pref(FILE_SORT_PREF, val);
                                        menu_open.set(false);
                                    }>
                                    <span>{t(loc, key)}</span>
                                    <span class="fb-sort-check" class:on=move || sort_by.get().as_str() == val>
                                        {compose_icon("check")}
                                    </span>
                                </button>
                            }).collect_view()}
                        </div>
                    </div>
                }
            })}
        </div>
    }
}

pub(crate) fn workspace_absolute_path(root: &str, path: &str) -> Option<String> {
    if root.is_empty() {
        return None;
    }
    let relative = workspace_relative_path(root, path)?;
    if relative.is_empty() {
        return Some(root.to_string());
    }
    let windows = root.contains('\\')
        || matches!(root.as_bytes(), [drive, b':', ..] if drive.is_ascii_alphabetic());
    let separator = if windows { '\\' } else { '/' };
    let relative = if windows {
        relative.replace('/', "\\")
    } else {
        relative.replace('\\', "/")
    };
    let root = root.trim_end_matches(['/', '\\']);
    if root.is_empty() {
        Some(format!("{separator}{relative}"))
    } else {
        Some(format!("{root}{separator}{relative}"))
    }
}

#[cfg(test)]
mod workspace_copy_path_tests {
    use super::{workspace_absolute_path, workspace_relative_path};

    #[test]
    fn copies_relative_and_absolute_paths_on_posix() {
        assert_eq!(
            workspace_absolute_path("/work/project", "results/table.csv"),
            Some("/work/project/results/table.csv".into())
        );
        assert_eq!(
            workspace_relative_path("/work/project", "/work/project/results/table.csv"),
            Some("results/table.csv".into())
        );
        assert_eq!(
            workspace_relative_path("/work/project", "/other/table.csv"),
            None
        );
        assert_eq!(
            workspace_absolute_path("/work/project ", "results/table .csv"),
            Some("/work/project /results/table .csv".into())
        );
    }

    #[test]
    fn copies_native_absolute_and_portable_relative_paths_on_windows() {
        assert_eq!(
            workspace_absolute_path(r"C:\work\project", "results/table.csv"),
            Some(r"C:\work\project\results\table.csv".into())
        );
        assert_eq!(
            workspace_relative_path(r"C:\work\project", r"c:\WORK\project\results\table.csv"),
            Some("results/table.csv".into())
        );
        assert_eq!(
            workspace_relative_path(
                r"\\Server\Share\Project",
                r"\\server\share\project\results\table.csv"
            ),
            Some("results/table.csv".into())
        );
    }
}

#[cfg(test)]
mod file_sort_tests {
    use super::{
        file_row_meta, sort_dir_entries, DirEntry, FileRowMeta, FILE_SORT_MODIFIED, FILE_SORT_NAME,
        FILE_SORT_SIZE,
    };

    fn entry(name: &str, is_dir: bool, size: u64, modified: Option<u64>) -> DirEntry {
        DirEntry {
            name: name.into(),
            is_dir,
            size,
            modified_unix_millis: modified,
        }
    }

    fn names(entries: &[DirEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.name.as_str()).collect()
    }

    #[test]
    fn sorts_directories_first_then_by_size_or_modified_time() {
        let mut entries = vec![
            entry("notes.md", false, 12, Some(10)),
            entry("data", true, 0, Some(5)),
            entry("big.bin", false, 100, Some(1)),
            entry("results", true, 0, Some(20)),
        ];

        sort_dir_entries(&mut entries, FILE_SORT_NAME);
        assert_eq!(names(&entries), ["data", "results", "big.bin", "notes.md"]);

        sort_dir_entries(&mut entries, FILE_SORT_SIZE);
        assert_eq!(names(&entries), ["data", "results", "big.bin", "notes.md"]);

        sort_dir_entries(&mut entries, FILE_SORT_MODIFIED);
        assert_eq!(names(&entries), ["results", "data", "notes.md", "big.bin"]);
    }

    #[test]
    fn right_column_follows_the_active_sort() {
        let file = entry("notes.md", false, 12, Some(1_700_000_000_000));
        let dir = entry("data", true, 4096, Some(1_700_000_000_000));
        assert_eq!(
            file_row_meta(&file, FILE_SORT_NAME),
            Some(FileRowMeta::Size(12))
        );
        assert_eq!(
            file_row_meta(&file, FILE_SORT_SIZE),
            Some(FileRowMeta::Size(12))
        );
        assert_eq!(
            file_row_meta(&file, FILE_SORT_MODIFIED),
            Some(FileRowMeta::Modified(1_700_000_000_000))
        );
        assert_eq!(file_row_meta(&dir, FILE_SORT_NAME), None);
        assert_eq!(file_row_meta(&dir, FILE_SORT_SIZE), None);
        assert_eq!(
            file_row_meta(&dir, FILE_SORT_MODIFIED),
            Some(FileRowMeta::Modified(1_700_000_000_000))
        );
    }
}
