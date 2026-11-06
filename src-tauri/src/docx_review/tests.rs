use super::*;
use serde_json::json;
use wisp_tools::ToolEnv;
struct Env(std::path::PathBuf);
#[async_trait::async_trait]
impl ToolEnv for Env {
    fn project_root(&self) -> &Path {
        &self.0
    }
    async fn confirm(&self, _: &str) -> bool {
        true
    }
    async fn emit(&self, _: wisp_tools::ToolEvent) {}
}

async fn fixture(root: &Path) {
    std::fs::write(
        root.join("source.docx"),
        include_bytes!("../../../ui-tests/fixtures/office-preview.docx"),
    )
    .unwrap();
    let env = Env(root.into());
    let tools = wisp_tools::Registry::builtins();
    let read = tools
        .run("read_docx", &json!({"path":"source.docx"}), &env)
        .await;
    assert!(read.success, "{}", read.content);
    let view: serde_json::Value = serde_json::from_str(&read.content).unwrap();
    let result = tools
        .run(
            "suggest_docx_edit",
            &json!({"path":"source.docx","output_path":"review.docx",
        "source_sha256":view["source_sha256"],"paragraph_id":"p2","old":"local","new":"reviewed"}),
            &env,
        )
        .await;
    assert!(result.success, "{}", result.content);
}

#[tokio::test]
async fn docx_review_saves_atomically_keeps_preimage_and_checks_snapshot() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path()).await;
    let source = std::fs::read(root.path().join("source.docx")).unwrap();
    let before = std::fs::read(root.path().join("review.docx")).unwrap();
    let snapshot = read_at(root.path(), "review.docx", true).unwrap();
    assert!(snapshot.writable);
    let selection = DocxReviewSelection::Change(snapshot.changes[0].id.clone());
    let after = resolve_at(
        root.path(),
        "review.docx",
        &snapshot.source_sha256,
        &selection,
        DocxReviewAction::Accept,
    )
    .unwrap();
    assert!(after.changes.is_empty());
    assert_eq!(
        std::fs::read(
            root.path()
                .join(".wisp/docx-review")
                .join(format!("{}.docx", snapshot.source_sha256))
        )
        .unwrap(),
        before
    );
    assert_eq!(
        std::fs::read(root.path().join("source.docx")).unwrap(),
        source
    );
    assert!(resolve_at(
        root.path(),
        "review.docx",
        &snapshot.source_sha256,
        &selection,
        DocxReviewAction::Reject
    )
    .is_err());
    assert!(!read_at(root.path(), "review.docx", false).unwrap().writable);
    assert!(read_at(root.path(), "../outside.docx", true).is_err());
    // The actual backend payload round-trips through the shared UI contract.
    assert_eq!(
        serde_json::from_value::<DocxReviewSnapshot>(serde_json::to_value(&after).unwrap())
            .unwrap(),
        after
    );
}

#[cfg(unix)]
#[tokio::test]
async fn docx_review_rejects_redirected_backup_directories() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fixture(root.path()).await;
    std::os::unix::fs::symlink(outside.path(), root.path().join(".wisp")).unwrap();
    let before = std::fs::read(root.path().join("review.docx")).unwrap();
    let snapshot = read_at(root.path(), "review.docx", true).unwrap();
    assert!(resolve_at(
        root.path(),
        "review.docx",
        &snapshot.source_sha256,
        &DocxReviewSelection::Kind(wisp_dto::DocxChangeKind::Rewrite),
        DocxReviewAction::Accept
    )
    .is_err());
    assert_eq!(
        std::fs::read(root.path().join("review.docx")).unwrap(),
        before
    );
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn docx_review_native_payload_matches_shared_fixture_and_current_bytes() {
    let root = tempfile::tempdir().unwrap();
    let bytes = include_bytes!("../../../ui-tests/fixtures/docx-review.docx");
    std::fs::write(root.path().join("review.docx"), bytes).unwrap();
    let (review, mut content) = native_read_at(root.path(), "review.docx", true).unwrap();
    content.path = "/project/review.docx".into();
    let envelope = wisp_dto::NativeDocxReview {
        schema: wisp_dto::native_files::SCHEMA.into(),
        project_id: "project-1".into(),
        session_id: Some("session-1".into()),
        path: content.path.clone(),
        review,
        content,
    };
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/native-files/v1/docx-review.json"
    ))
    .unwrap();
    assert_eq!(serde_json::to_value(&envelope).unwrap(), fixture);
    let args: wisp_dto::NativeDocxReviewRequest =
        serde_json::from_value(json!({"session_id":"session-1", "path":"review.docx"})).unwrap();
    assert!(validate_native_request(&args, Some("session-1"), false).is_ok());
    assert!(validate_native_request(&args, Some("other"), false).is_err());
    assert!(validate_native_request(&args, None, false).is_err());
    assert!(validate_native_request(&args, Some("session-1"), true).is_err());
    assert!(serde_json::from_value::<wisp_dto::NativeDocxReviewRequest>(
        json!({"path":"review.docx", "context_id":"ssh:host"})
    )
    .is_err());
    for path in [
        "artifact:one",
        "artifact-version:one",
        "remote:ssh:host:file.docx",
        "ssh://host/file.docx",
    ] {
        let args: wisp_dto::NativeDocxReviewRequest =
            serde_json::from_value(json!({"path":path})).unwrap();
        assert!(validate_native_request(&args, None, false).is_err());
    }
    let next = resolve_at(
        root.path(),
        "review.docx",
        &envelope.review.source_sha256,
        &DocxReviewSelection::Kind(wisp_dto::DocxChangeKind::Rewrite),
        DocxReviewAction::Accept,
    )
    .unwrap();
    let (refreshed, content) = native_read_at(root.path(), "review.docx", false).unwrap();
    assert!(!refreshed.writable);
    assert_eq!(refreshed.source_sha256, next.source_sha256);
    assert_eq!(refreshed.changes.len(), 2);
    use base64::Engine;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(content.base64.unwrap())
            .unwrap(),
        std::fs::read(root.path().join("review.docx")).unwrap()
    );
}
