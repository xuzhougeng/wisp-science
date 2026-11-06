//! In-process coordination for project resources used by parallel conversations.
//!
//! This is intentionally an advisory Wisp lease, not an OS file lock: child
//! interpreters and external editors do not consistently participate in
//! platform file-lock APIs. The host wraps complete tool calls instead, which
//! protects Wisp-vs-Wisp access and leaves checksum/file-watcher protection as
//! the separate boundary for external changes.

use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

pub(crate) const CONFIRM_PREFIX: &str = "WISP_RESOURCE_CONFLICT\n";
pub(crate) const CONFIRM_TOOL: &str = "resource_conflict";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceAccess {
    Read,
    Write,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResourceScope {
    Path(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResourceRequest {
    pub(crate) access: ResourceAccess,
    pub(crate) scope: ResourceScope,
}

impl ResourceRequest {
    pub(crate) fn description(&self) -> String {
        match &self.scope {
            ResourceScope::Path(path) => format!("`{path}`"),
        }
    }
}

#[derive(Clone)]
struct ActiveClaim {
    id: u64,
    project_id: String,
    frame_id: String,
    tool: String,
    preview: String,
    request: ResourceRequest,
    started: Instant,
    released: tokio::sync::watch::Sender<bool>,
}

#[derive(Default)]
struct CoordinatorState {
    next_id: u64,
    claims: Vec<ActiveClaim>,
}

#[derive(Default)]
struct CoordinatorInner {
    state: Mutex<CoordinatorState>,
}

impl CoordinatorInner {
    fn release(&self, id: u64) {
        let released = {
            let mut state = self.state.lock().unwrap();
            state
                .claims
                .iter()
                .position(|claim| claim.id == id)
                .map(|index| state.claims.remove(index).released)
        };
        if let Some(released) = released {
            let _ = released.send(true);
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct ProjectResourceCoordinator {
    inner: Arc<CoordinatorInner>,
}

pub(crate) enum AcquireResult {
    Acquired(wisp_tools::ToolResourceLease),
    Conflict(ResourceConflict),
}

pub(crate) struct ResourceConflict {
    pub(crate) frame_id: String,
    pub(crate) tool: String,
    pub(crate) preview: String,
    pub(crate) request: ResourceRequest,
    pub(crate) elapsed_secs: u64,
    released: tokio::sync::watch::Receiver<bool>,
}

impl ResourceConflict {
    pub(crate) async fn wait_until_released(&mut self) {
        if *self.released.borrow() {
            return;
        }
        let _ = self.released.changed().await;
    }
}

impl ProjectResourceCoordinator {
    pub(crate) fn try_acquire(
        &self,
        project_id: &str,
        frame_id: &str,
        tool: &str,
        preview: &str,
        request: ResourceRequest,
    ) -> AcquireResult {
        let mut state = self.inner.state.lock().unwrap();
        if let Some(claim) = state.claims.iter().find(|claim| {
            claim.project_id == project_id
                && claim.frame_id != frame_id
                && requests_conflict(&claim.request, &request)
        }) {
            return AcquireResult::Conflict(ResourceConflict {
                frame_id: claim.frame_id.clone(),
                tool: claim.tool.clone(),
                preview: claim.preview.clone(),
                request: claim.request.clone(),
                elapsed_secs: claim.started.elapsed().as_secs(),
                released: claim.released.subscribe(),
            });
        }

        state.next_id = state.next_id.wrapping_add(1).max(1);
        let id = state.next_id;
        let (released, _) = tokio::sync::watch::channel(false);
        state.claims.push(ActiveClaim {
            id,
            project_id: project_id.to_string(),
            frame_id: frame_id.to_string(),
            tool: tool.to_string(),
            preview: preview.to_string(),
            request,
            started: Instant::now(),
            released,
        });
        let inner = self.inner.clone();
        AcquireResult::Acquired(wisp_tools::ToolResourceLease::new(move || {
            inner.release(id)
        }))
    }
}

fn requests_conflict(left: &ResourceRequest, right: &ResourceRequest) -> bool {
    if left.access == ResourceAccess::Read && right.access == ResourceAccess::Read {
        return false;
    }
    scopes_overlap(&left.scope, &right.scope)
}

fn scopes_overlap(left: &ResourceScope, right: &ResourceScope) -> bool {
    match (left, right) {
        (ResourceScope::Path(left), ResourceScope::Path(right)) => {
            is_path_prefix(left, right) || is_path_prefix(right, left)
        }
    }
}

fn is_path_prefix(prefix: &str, path: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn normalized_project_path(root: &Path, raw: &str) -> Option<String> {
    let real = wisp_tools::safety::validate_file_path(root, raw).ok()?;
    let root = dunce::canonicalize(root).ok()?;
    let relative = real.strip_prefix(root).ok()?;
    let value = relative.to_string_lossy().replace('\\', "/");
    if value.is_empty() {
        return None;
    }
    #[cfg(windows)]
    let value = value.to_ascii_lowercase();
    Some(value)
}

/// Map current built-in tools onto the smallest resource scope they can state
/// truthfully. Unknown/plugin tools remain uncoordinated until they expose
/// structured resource metadata.
pub(crate) fn request_for_call(
    root: &Path,
    tool: &str,
    args: &serde_json::Value,
) -> Option<ResourceRequest> {
    let path_request = |key: &str, access| {
        args.get(key)
            .and_then(serde_json::Value::as_str)
            .and_then(|path| normalized_project_path(root, path))
            .map(|path| ResourceRequest {
                access,
                scope: ResourceScope::Path(path),
            })
    };
    match tool {
        "read" | "read_docx" | "view_image" => path_request("path", ResourceAccess::Read),
        "write" | "edit" | "generate_image" => path_request("path", ResourceAccess::Write),
        // The source is snapshot-checked again before publication; coordinate
        // the new output against other project writers.
        "suggest_docx_edit" => path_request("output_path", ResourceAccess::Write),
        _ => None,
    }
}

pub(crate) fn preview_for_call(tool: &str, args: &serde_json::Value) -> String {
    let key = match tool {
        "read" | "read_docx" | "view_image" | "write" | "edit" | "generate_image" => "path",
        "suggest_docx_edit" => "output_path",
        "python" | "r" => "code",
        _ => "cmd",
    };
    let value = args
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let mut preview: String = value.chars().take(240).collect();
    if value.chars().count() > 240 {
        preview.push('…');
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(access: ResourceAccess, path: &str) -> ResourceRequest {
        ResourceRequest {
            access,
            scope: ResourceScope::Path(path.into()),
        }
    }

    #[test]
    fn reads_share_a_path_but_writes_conflict() {
        assert!(!requests_conflict(
            &path(ResourceAccess::Read, "plot.R"),
            &path(ResourceAccess::Read, "plot.R")
        ));
        assert!(requests_conflict(
            &path(ResourceAccess::Read, "plot.R"),
            &path(ResourceAccess::Write, "plot.R")
        ));
        assert!(!requests_conflict(
            &path(ResourceAccess::Write, "plot.R"),
            &path(ResourceAccess::Write, "data.csv")
        ));
    }

    #[test]
    fn unstructured_execution_tools_do_not_request_leases() {
        let root = Path::new(".");
        for tool in ["shell", "python", "r"] {
            assert_eq!(request_for_call(root, tool, &serde_json::json!({})), None);
        }
    }

    #[tokio::test]
    async fn dropping_a_lease_wakes_a_conflicting_conversation() {
        let coordinator = ProjectResourceCoordinator::default();
        let first = coordinator.try_acquire(
            "project",
            "frame-a",
            "edit",
            "plot.R",
            path(ResourceAccess::Write, "plot.R"),
        );
        let AcquireResult::Acquired(first) = first else {
            panic!("first claim should acquire")
        };
        let second = coordinator.try_acquire(
            "project",
            "frame-b",
            "read",
            "plot.R",
            path(ResourceAccess::Read, "plot.R"),
        );
        let AcquireResult::Conflict(mut conflict) = second else {
            panic!("second claim should conflict")
        };
        drop(first);
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            conflict.wait_until_released(),
        )
        .await
        .expect("release notification");
    }

    #[test]
    fn tool_requests_normalize_project_relative_paths() {
        let root =
            std::env::temp_dir().join(format!("wisp-resource-request-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("plot.R"), "plot(1)\n").unwrap();
        let request =
            request_for_call(&root, "edit", &serde_json::json!({"path": "plot.R"})).unwrap();
        assert_eq!(
            request,
            path(
                ResourceAccess::Write,
                if cfg!(windows) { "plot.r" } else { "plot.R" }
            )
        );
        assert_eq!(
            request_for_call(
                &root,
                "read_docx",
                &serde_json::json!({"path":"source.docx"})
            ),
            Some(path(ResourceAccess::Read, "source.docx"))
        );
        let docx = serde_json::json!({"path":"source.docx", "output_path":"review.docx"});
        assert_eq!(
            request_for_call(&root, "suggest_docx_edit", &docx),
            Some(path(ResourceAccess::Write, "review.docx"))
        );
        assert_eq!(preview_for_call("suggest_docx_edit", &docx), "review.docx");
        std::fs::remove_dir_all(root).ok();
    }
}
