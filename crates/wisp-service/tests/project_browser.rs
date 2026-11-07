use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};
use wisp_dto::project_browser::{Response, SCHEMA};
use wisp_store::Store;

fn request(database: &Path, input: &str) -> Output {
    request_mode(database, input, false)
}

fn request_mode(database: &Path, input: &str, writable: bool) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wisp-service"))
        .arg("--database")
        .arg(database)
        .args(if writable {
            vec!["--allow-project-writes"]
        } else {
            vec![]
        })
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn shared_native_fixture_roundtrips_through_the_rust_contract() {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../contracts/project-browser/v1/projects.json"
    ))
    .unwrap();
    let decoded: Response = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);

    let mut legacy = expected.clone();
    legacy.as_object_mut().unwrap().remove("privacy");
    let decoded: Response = serde_json::from_value(legacy).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
}

#[tokio::test]
async fn stdio_supports_queries_capabilities_and_recovers_after_invalid_requests() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("wisp.sqlite");
    let store = Store::open(&database).await.unwrap();
    store
        .create_project("p", "Research", "workspace with spaces")
        .await
        .unwrap();
    let input = format!(
        "not json\n{}\n{}\n{}\n",
        json!({"schema":"old", "id":"old", "type":"list_projects"}),
        json!({"schema":SCHEMA, "id":"cap", "type":"capabilities"}),
        json!({"schema":SCHEMA, "id":"list", "type":"list_projects"}),
    );
    let output = request(&database, &input);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0]["code"], "invalid_request");
    assert_eq!(replies[1]["id"], "old");
    assert_eq!(replies[1]["code"], "unsupported_schema");
    assert_eq!(replies[2]["read_only"], true);
    assert_eq!(
        replies[2]["commands"],
        json!([
            "list_projects",
            "list_sessions",
            "get_transcript",
            "search_project_chats",
            "locate_project_chat_match",
            "capabilities"
        ])
    );
    assert_eq!(replies[3]["schema"], SCHEMA);
    assert_eq!(replies[3]["id"], "list");
    assert_eq!(replies[3]["activity_source"], "persisted_only");
    assert_eq!(
        replies[3]["privacy"],
        json!({"active":false, "project_ids":[]})
    );
    assert_eq!(replies[3]["projects"][0]["name"], "Research");
    assert_eq!(store.list_projects().await.unwrap().len(), 1);
}

#[tokio::test]
async fn project_chat_search_and_location_are_read_only_and_scoped() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("wisp.sqlite");
    let store = Store::open(&database).await.unwrap();
    store
        .create_project("p", "Research", "workspace")
        .await
        .unwrap();
    let search = json!({"schema":SCHEMA, "id":"search", "type":"search_project_chats",
        "project_id":"p", "query":"100%_", "cursor":null});
    let locate = json!({"schema":SCHEMA, "id":"location", "type":"locate_project_chat_match",
        "project_id":"p", "session_id":"not-in-this-project", "message_seq":1});
    let output = request(&database, &format!("{search}\n{locate}\n"));
    assert!(output.status.success());
    let replies: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(
        replies[0],
        json!({"schema":SCHEMA, "id":"search", "type":"project_chat_matches",
        "project_id":"p", "query":"100%_", "page":{"items":[], "next_cursor":null}})
    );
    assert_eq!(replies[1]["id"], "location");
    assert_eq!(replies[1]["code"], "query_failed");
}

#[tokio::test]
async fn read_only_open_rejects_writes_and_never_creates_a_missing_database() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("wisp.sqlite");
    let writable = Store::open(&database).await.unwrap();
    writable
        .create_project("p", "Existing", "workspace")
        .await
        .unwrap();
    let read_only = Store::open_read_only(&database).await.unwrap();
    assert!(read_only
        .create_project("new", "Forbidden", "workspace")
        .await
        .is_err());
    assert_eq!(read_only.list_projects().await.unwrap().len(), 1);
    let missing = directory.path().join("missing.sqlite");
    let output = request(&missing, "");
    assert!(!output.status.success());
    assert!(!missing.exists());
    assert!(output.stdout.is_empty());
}

#[tokio::test]
async fn oversized_requests_fail_without_polluting_stdout() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("wisp.sqlite");
    let _store = Store::open(&database).await.unwrap();
    let output = request(&database, &"x".repeat(64 * 1024 + 1));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("64 KiB"));
}

#[test]
fn shared_session_and_transcript_contracts_roundtrip() {
    for source in [
        include_str!("../../../contracts/project-browser/v1/chat-matches.json"),
        include_str!("../../../contracts/project-browser/v1/sessions.json"),
        include_str!("../../../contracts/project-browser/v1/transcript.json"),
    ] {
        let expected: Value = serde_json::from_str(source).unwrap();
        let decoded: Response = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }
}

#[tokio::test]
async fn project_stars_require_explicit_write_mode_and_preserve_activity() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("wisp.sqlite");
    let store = Store::open(&database).await.unwrap();
    store
        .create_project("research-1", "Research", "same workspace")
        .await
        .unwrap();
    store
        .create_project("other", "Other identity", "same workspace")
        .await
        .unwrap();
    store
        .create_project("assistant:research", "Assistant", "assistant")
        .await
        .unwrap();
    let before = store.list_projects().await.unwrap();
    let command = include_str!("../../../contracts/project-browser/v1/set-project-starred.json");
    let decoded: wisp_dto::project_browser::Request = serde_json::from_str(command).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::from_str::<Value>(command).unwrap()
    );
    let denied = request(&database, command);
    let reply: Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert_eq!(reply["code"], "write_disabled");
    assert!(store.starred_project_ids().await.unwrap().is_empty());
    for _ in 0..2 {
        let output = request_mode(&database, command, true);
        assert!(output.status.success(), "{:?}", output);
        let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(reply["type"], "projects");
        assert_eq!(reply["projects"][0]["id"], "research-1");
        assert_eq!(reply["projects"][0]["starred"], true);
        assert_eq!(reply["projects"][1]["starred"], false);
    }
    let after = store.list_projects().await.unwrap();
    for row in &after {
        assert_eq!(row.4, before.iter().find(|old| old.0 == row.0).unwrap().4);
    }
    for id in ["missing", "assistant:research"] {
        let input = format!(
            "{}\n",
            json!({"schema":SCHEMA,"id":"bad","type":"set_project_starred","project_id":id,"starred":true})
        );
        let output = request_mode(&database, &input, true);
        let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(reply["code"], "command_failed");
    }
    let output = request_mode(&database, &command.replace("true", "false"), true);
    assert!(output.status.success());
    assert!(store.starred_project_ids().await.unwrap().is_empty());
    assert_eq!(store.list_projects().await.unwrap(), before);
    let caps = format!(
        "{}\n",
        json!({"schema":SCHEMA,"id":"cap","type":"capabilities"})
    );
    let output = request_mode(&database, &caps, true);
    let reply: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(reply["read_only"], false);
    assert!(reply["commands"]
        .as_array()
        .unwrap()
        .contains(&json!("set_project_starred")));
    let missing = directory.path().join("missing.sqlite");
    assert!(!request_mode(&missing, command, true).status.success());
    assert!(!missing.exists());
}
