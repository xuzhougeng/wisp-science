use super::*;
use serde_json::json;
use std::{path::Path, sync::atomic::AtomicUsize};
use wisp_llm::{Completion, Message, Provider, StreamSink, ToolCall, ToolSchema};

struct FakeModel {
    calls: Arc<AtomicUsize>,
    write: bool,
}
#[async_trait]
impl Provider for FakeModel {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "Saved result".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.write
            && !messages
                .iter()
                .any(|message| message.role == wisp_llm::Role::Tool)
        {
            return Ok(Completion {
                tool_calls: vec![ToolCall {
                    id: "write-1".into(),
                    kind: "function".into(),
                    function: wisp_llm::FunctionCall {
                        name: "write".into(),
                        arguments: json!({"path":"answer.txt", "content":"offline result"})
                            .to_string(),
                    },
                }],
                finish_reason: Some("tool_calls".into()),
                ..Default::default()
            });
        }
        sink.on_text("Saved result");
        self.complete(messages, tools).await
    }
}

fn config(root: &Path) -> Config {
    Config {
        data: root.join("data"),
        workspace: root.join("workspace"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_url: "https://test.invalid/".into(),
        code: wisp_sync::RemoteCode::generate().unwrap(),
        model: Some(crate::config::ModelConfig {
            provider: wisp_llm::ProviderConfig::openai(
                "http://127.0.0.1:9",
                "synthetic",
                "offline",
            ),
            max_context: 128_000,
            supports_vision: false,
        }),
        max_iterations: 5,
        skills: vec![],
    }
}

fn models(calls: Arc<AtomicUsize>, write: bool) -> ModelFactory {
    Arc::new(move |_| {
        Box::new(FakeModel {
            calls: calls.clone(),
            write,
        })
    })
}

#[tokio::test]
async fn local_project_registration_preserves_identity_and_is_not_remotely_exposed() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let project = temp.path().join("研究 project");
    std::fs::create_dir(&project).unwrap();
    let first = server.register_local_project(&project).await.unwrap();
    assert_eq!(
        server
            .register_local_project(&project.join("."))
            .await
            .unwrap(),
        first
    );
    assert!(server.register_local_project(temp.path()).await.is_err());
    assert!(server
        .register_local_project(&temp.path().join("data"))
        .await
        .is_err());
    assert!(server
        .register_local_project(&temp.path().join("missing"))
        .await
        .is_err());
    assert!(!server.commands().contains(&"register_local_project"));
    let session = call(&server, "native_conversation_create", &first, json!({}))
        .await
        .unwrap();
    let result = call(
        &server,
        "native_conversation_snapshot",
        &first,
        json!({"session_id": session}),
    )
    .await
    .unwrap();
    assert_eq!(result["project_id"], first);
    server.shutdown().await;
}

async fn call(server: &Server, command: &str, project: &str, args: Value) -> Result<Value, String> {
    server
        .dispatch(RemoteRequest {
            nonce: "test".into(),
            seq: 1,
            id: uuid::Uuid::new_v4().to_string(),
            command: command.into(),
            project_id: Some(project.into()),
            args,
        })
        .await
}

async fn snapshot(server: &Server, id: &str) -> dto::Snapshot {
    serde_json::from_value(
        call(
            server,
            "native_conversation_snapshot",
            PROJECT,
            json!({"session_id":id}),
        )
        .await
        .unwrap(),
    )
    .unwrap()
}

async fn wait_for(
    server: &Server,
    id: &str,
    ready: impl Fn(&dto::Snapshot) -> bool,
) -> dto::Snapshot {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let value = snapshot(server, id).await;
            if ready(&value) {
                return value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("offline turn must make progress")
}

#[tokio::test]
async fn mobile_project_list_omits_a_missing_registered_database() {
    let root = tempfile::tempdir().unwrap();
    let cfg = config(root.path());
    let store = Store::open_application(&cfg.data.join("wisp.sqlite"))
        .await
        .unwrap();
    store
        .set_setting("decentralized_project_storage", "true")
        .await
        .unwrap();
    let workspace = root.path().join("offline");
    store
        .create_project("offline", "Offline", workspace.to_str().unwrap())
        .await
        .unwrap();
    let server = Server::open_with_models(cfg, models(Arc::new(AtomicUsize::new(0)), false))
        .await
        .unwrap();
    // Re-register after releasing the startup reader's handles, including on Windows.
    server.0.store.delete_project("offline").await.unwrap();
    server
        .0
        .store
        .register_project_folder(&workspace)
        .await
        .unwrap();
    let database = workspace.join(wisp_store::PROJECT_DATABASE);
    let saved = root.path().join("saved.sqlite");
    std::fs::rename(&database, &saved).unwrap();

    let projects = call(&server, "list_projects", PROJECT, json!({}))
        .await
        .unwrap();
    let rows = projects.as_array().unwrap();
    assert!(rows.iter().any(|row| row["id"] == PROJECT));
    assert!(rows.iter().all(|row| row["id"] != "offline"));
    assert!(server
        .0
        .store
        .list_projects()
        .await
        .unwrap()
        .iter()
        .any(|row| row.0 == "offline"));
    assert!(!database.exists());

    std::fs::rename(&saved, &database).unwrap();
    let projects = call(&server, "list_projects", PROJECT, json!({}))
        .await
        .unwrap();
    assert!(projects
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["id"] == "offline"));
    server.shutdown().await;
}

#[tokio::test]
async fn mobile_turn_approval_persistence_and_restart_reuse_the_same_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let server = Server::open_with_models(cfg.clone(), models(calls.clone(), true))
        .await
        .unwrap();
    let id = call(&server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let send = json!({"session_id":id,"request_id":"request-1","message":"Write the result"});
    // Drop the submitter's reply. The accepted task lives in the server.
    drop(
        call(&server, "native_conversation_send", PROJECT, send.clone())
            .await
            .unwrap(),
    );
    let pending = wait_for(&server, &id, |s| !s.approvals.is_empty()).await;
    let wire = serde_json::to_value(&pending).unwrap();
    assert!(wire.get("exploration").is_none());
    assert!(wire.get("branches").is_none());
    assert!(!cfg.workspace.join("answer.txt").exists());
    assert_eq!(pending.approvals[0].tool, "write");
    assert!(pending.approvals[0].preview.contains("offline result"));
    let approval =
        json!({"session_id":id,"approval_id":pending.approvals[0].approval_id,"approved":true});
    let mut broad = approval.clone();
    broad["scope"] = "global".into();
    assert!(call(&server, "native_conversation_approve", PROJECT, broad)
        .await
        .is_err());
    assert!(call(
        &server,
        "native_conversation_snapshot",
        "other",
        json!({"session_id":id})
    )
    .await
    .is_err());
    call(
        &server,
        "native_conversation_approve",
        PROJECT,
        approval.clone(),
    )
    .await
    .unwrap();
    let finished = wait_for(&server, &id, |s| !s.running).await;
    assert!(finished.error.is_none(), "{:?}", finished.error);
    assert!(finished
        .items
        .iter()
        .any(|item| item.tool_name.as_deref() == Some("write") && item.ok == Some(true)));
    assert_eq!(
        std::fs::read_to_string(cfg.workspace.join("answer.txt")).unwrap(),
        "offline result"
    );
    assert!(finished
        .items
        .iter()
        .any(|item| item.text == "Saved result"));
    assert!(
        call(&server, "native_conversation_approve", PROJECT, approval)
            .await
            .is_err()
    );
    let count = calls.load(Ordering::Relaxed);
    call(&server, "native_conversation_send", PROJECT, send.clone())
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), count);
    server.shutdown().await;
    drop(server);
    let reopened = Server::open_with_models(cfg, models(calls.clone(), false))
        .await
        .unwrap();
    let restored = snapshot(&reopened, &id).await;
    assert!(restored
        .items
        .iter()
        .any(|item| item.tool_name.as_deref() == Some("write") && item.ok == Some(true)));
    assert!(restored
        .items
        .iter()
        .any(|item| item.text == "Saved result"));
    call(&reopened, "native_conversation_send", PROJECT, send.clone())
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), count);
    let mut changed = send;
    changed["message"] = "different".into();
    assert!(
        call(&reopened, "native_conversation_send", PROJECT, changed)
            .await
            .is_err()
    );
    reopened.shutdown().await;
}

#[tokio::test]
async fn snapshot_during_turn_completion_includes_the_final_reply() {
    let tmp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(
        config(tmp.path()),
        models(Arc::new(AtomicUsize::new(0)), true),
    )
    .await
    .unwrap();
    let id = call(&server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    call(
        &server,
        "native_conversation_send",
        PROJECT,
        json!({"session_id":id,"request_id":"snapshot-race","message":"write"}),
    )
    .await
    .unwrap();
    let pending = wait_for(&server, &id, |s| !s.approvals.is_empty()).await;

    // Hold a snapshot after its transcript read, then finish and persist the turn.
    // A completed snapshot must not reuse the earlier, incomplete transcript.
    let (loaded_tx, loaded_rx) = tokio::sync::oneshot::channel();
    let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
    *server.session(&id).snapshot_after_read.lock().unwrap() = Some((loaded_tx, resume_rx));
    let reading = {
        let server = server.clone();
        let id = id.clone();
        tokio::spawn(async move { snapshot(&server, &id).await })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), loaded_rx)
        .await
        .unwrap()
        .unwrap();
    call(
        &server,
        "native_conversation_approve",
        PROJECT,
        json!({"session_id":id,"approval_id":pending.approvals[0].approval_id,"approved":true}),
    )
    .await
    .unwrap();
    wait_for(&server, &id, |s| !s.running).await;
    resume_tx.send(()).unwrap();
    let finished = tokio::time::timeout(std::time::Duration::from_secs(5), reading)
        .await
        .unwrap()
        .unwrap();
    assert!(!finished.running);
    assert!(finished.error.is_none(), "{:?}", finished.error);
    assert!(finished
        .items
        .iter()
        .any(|item| item.text == "Saved result"));
    assert!(finished
        .items
        .iter()
        .any(|item| item.tool_name.as_deref() == Some("write") && item.ok == Some(true)));
    server.shutdown().await;
}

#[tokio::test]
async fn stop_during_approval_never_executes_the_write() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::new(AtomicUsize::new(0)), true))
        .await
        .unwrap();
    let id = call(&server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    call(
        &server,
        "native_conversation_send",
        PROJECT,
        json!({"session_id":id,"request_id":"r","message":"write"}),
    )
    .await
    .unwrap();
    wait_for(&server, &id, |s| !s.approvals.is_empty()).await;
    call(
        &server,
        "native_conversation_stop",
        PROJECT,
        json!({"session_id":id}),
    )
    .await
    .unwrap();
    let finished = wait_for(&server, &id, |s| !s.running).await;
    assert!(finished.approvals.is_empty());
    assert!(!cfg.workspace.join("answer.txt").exists());
    server.shutdown().await;
}

#[tokio::test]
async fn one_server_owns_each_database_and_shutdown_rejects_new_work() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = config(tmp.path());
    let server =
        Server::open_with_models(cfg.clone(), models(Arc::new(AtomicUsize::new(0)), false))
            .await
            .unwrap();
    assert!(Server::open(cfg).await.is_err());
    let id = call(&server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    server.shutdown().await;
    assert!(call(
        &server,
        "native_conversation_send",
        PROJECT,
        json!({"session_id":id,"request_id":"r","message":"hello"})
    )
    .await
    .is_err());
    assert!(call(&server, "arbitrary_command", PROJECT, json!({}))
        .await
        .is_err());
}

async fn send(server: &Server, session: &str, message: &str) -> Result<Value, String> {
    call(
        server,
        "native_conversation_send",
        PROJECT,
        json!({"session_id": session, "request_id": uuid::Uuid::new_v4().to_string(), "message": message}),
    )
    .await
}

async fn create(server: &Server) -> String {
    call(server, "native_conversation_create", PROJECT, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn desktop_manages_conversations_without_exposing_it_remotely() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let alpha = create(&server).await;
    let beta = create(&server).await;
    send(&server, &alpha, "sequence alignment plan")
        .await
        .unwrap();
    wait_for(&server, &alpha, |s| !s.running).await;

    server
        .rename_conversation(PROJECT, &beta, "  基因组 notes  ")
        .await
        .unwrap();
    assert!(server
        .rename_conversation(PROJECT, &beta, " ")
        .await
        .is_err());
    assert!(server
        .rename_conversation(PROJECT, &beta, &"x".repeat(201))
        .await
        .is_err());
    assert!(server
        .rename_conversation("other-project", &beta, "stolen")
        .await
        .is_err());
    server.pin_conversation(PROJECT, &beta, true).await.unwrap();

    let recent = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(recent.pinned, [beta.clone()]);
    let title = |list: &desktop::ConversationList, id: &str| {
        list.rows
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.title.clone())
    };
    assert_eq!(title(&recent, &beta).as_deref(), Some("基因组 notes"));
    assert!(title(&recent, &alpha).is_some());

    // Keyword search reads message text, not only titles.
    let found = server
        .conversations(PROJECT, " alignment ", false)
        .await
        .unwrap();
    assert!(title(&found, &alpha).is_some());
    assert!(title(&found, &beta).is_none());

    server
        .archive_conversation(PROJECT, &alpha, true)
        .await
        .unwrap();
    let recent = server.conversations(PROJECT, "", false).await.unwrap();
    assert!(title(&recent, &alpha).is_none());
    let archive = server.conversations(PROJECT, "", true).await.unwrap();
    assert_eq!(archive.rows.len(), 1);
    assert!(archive.rows[0].shelved && archive.rows[0].id == alpha);
    // An archived conversation is still found by keyword and can come back.
    let found = server
        .conversations(PROJECT, "alignment", false)
        .await
        .unwrap();
    assert!(found.rows.iter().any(|row| row.id == alpha && row.shelved));
    server
        .archive_conversation(PROJECT, &alpha, false)
        .await
        .unwrap();
    assert!(title(
        &server.conversations(PROJECT, "", false).await.unwrap(),
        &alpha
    )
    .is_some());

    assert!(server
        .delete_conversation("other-project", &alpha)
        .await
        .is_err());
    server.delete_conversation(PROJECT, &alpha).await.unwrap();
    assert!(server.delete_conversation(PROJECT, &alpha).await.is_err());
    assert!(call(
        &server,
        "native_conversation_snapshot",
        PROJECT,
        json!({"session_id": alpha}),
    )
    .await
    .is_err());
    let left = server.conversations(PROJECT, "", false).await.unwrap();
    assert!(title(&left, &alpha).is_none() && title(&left, &beta).is_some());

    // None of this is a remote command.
    for command in [
        "native_conversation_rename",
        "native_conversation_pin",
        "native_conversation_delete",
        "native_project_session_shelve",
    ] {
        assert!(!server.commands().contains(&command));
        assert!(call(&server, command, PROJECT, json!({"session_id": beta}))
            .await
            .is_err());
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_running_conversation_is_not_archived_or_deleted() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), true))
        .await
        .unwrap();
    let session = create(&server).await;
    send(&server, &session, "write the answer").await.unwrap();
    // The write tool waits for approval, which keeps the turn running.
    wait_for(&server, &session, |s| !s.approvals.is_empty()).await;
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    for result in [
        server.archive_conversation(PROJECT, &session, true).await,
        server.delete_conversation(PROJECT, &session).await,
        server
            .transfer_conversation(PROJECT, &session, &lab, false)
            .await
            .map(|_| ()),
        server
            .transfer_conversation(PROJECT, &session, &lab, true)
            .await
            .map(|_| ()),
        server
            .rewind_conversation(
                PROJECT,
                &session,
                &desktop::TurnRef {
                    from_end: 0,
                    text: "write the answer".into(),
                },
            )
            .await
            .map(|_| ()),
        server
            .branch_conversation(PROJECT, &session, None)
            .await
            .map(|_| ()),
    ] {
        assert!(result.unwrap_err().to_string().contains("Stop the running"));
    }
    // A batch reports the refusal instead of failing as a whole.
    let report = server
        .batch(PROJECT, &[session.clone()], desktop::BatchAction::Delete)
        .await
        .unwrap();
    assert!(report.done.is_empty());
    assert!(report.refused[0].1.contains("Stop the running"));
    server
        .pin_conversation(PROJECT, &session, true)
        .await
        .unwrap();
    call(
        &server,
        "native_conversation_stop",
        PROJECT,
        json!({"session_id": session}),
    )
    .await
    .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    server.delete_conversation(PROJECT, &session).await.unwrap();
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_groups_batches_and_moves_conversations_between_projects() {
    use desktop::{BatchAction, Folder};
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let alpha = create(&server).await;
    let beta = create(&server).await;
    let gamma = create(&server).await;
    send(&server, &alpha, "sequence alignment plan")
        .await
        .unwrap();
    wait_for(&server, &alpha, |s| !s.running).await;

    // Groups: named, unique per project, in the order the user gives them.
    let screens = server.create_folder(PROJECT, "  Screens ").await.unwrap();
    assert_eq!(screens.name, "Screens");
    let later = server.create_folder(PROJECT, "Later").await.unwrap();
    for name in ["", "  ", "screens", &"x".repeat(61)] {
        assert!(server.create_folder(PROJECT, name).await.is_err(), "{name}");
    }
    assert!(server.create_folder("missing", "Screens").await.is_err());
    server
        .reorder_folders(PROJECT, &[later.id.clone(), screens.id.clone()])
        .await
        .unwrap();
    server
        .rename_folder(PROJECT, &later.id, "Someday")
        .await
        .unwrap();
    // A group may keep its own name, but not take another's.
    server
        .rename_folder(PROJECT, &later.id, "someday")
        .await
        .unwrap();
    assert!(server
        .rename_folder(PROJECT, &later.id, "Screens")
        .await
        .is_err());
    assert!(server
        .rename_folder(PROJECT, "missing", "Other")
        .await
        .is_err());

    server
        .group_conversation(PROJECT, &alpha, Some(&screens.id))
        .await
        .unwrap();
    assert!(server
        .group_conversation(PROJECT, &alpha, Some("missing"))
        .await
        .is_err());
    assert!(server
        .group_conversation("other-project", &alpha, None)
        .await
        .is_err());
    let report = server
        .batch(
            PROJECT,
            &[beta.clone(), gamma.clone(), "missing".into(), beta.clone()],
            BatchAction::Group(Some(later.id.clone())),
        )
        .await
        .unwrap();
    assert_eq!(report.done, [beta.clone(), gamma.clone()]);
    assert_eq!(report.refused.len(), 1);
    assert_eq!(report.refused[0].0, "missing");
    assert!(server
        .batch(
            PROJECT,
            &[beta.clone()],
            BatchAction::Group(Some("missing".into()))
        )
        .await
        .is_err());

    let list = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(
        list.folders,
        [
            Folder {
                id: later.id.clone(),
                name: "someday".into()
            },
            screens.clone()
        ]
    );
    assert_eq!(list.grouped.get(&alpha), Some(&screens.id));
    assert_eq!(list.grouped.get(&beta), Some(&later.id));
    assert_eq!(list.grouped.get(&gamma), Some(&later.id));

    // Taking a conversation out of its group, and deleting a group, keep
    // the conversations.
    server
        .batch(PROJECT, &[gamma.clone()], BatchAction::Group(None))
        .await
        .unwrap();
    server.delete_folder(PROJECT, &later.id).await.unwrap();
    assert!(server.delete_folder(PROJECT, &later.id).await.is_err());
    let list = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(list.folders, [screens.clone()]);
    assert_eq!(list.rows.len(), 3);
    assert_eq!(list.grouped.len(), 1);

    // Archive, restore and delete several at once.
    let both = [beta.clone(), gamma.clone()];
    let report = server
        .batch(PROJECT, &both, BatchAction::Archive)
        .await
        .unwrap();
    assert_eq!(report.done, both);
    let archived = server.conversations(PROJECT, "", true).await.unwrap();
    assert_eq!(archived.rows.len(), 2);
    server
        .batch(PROJECT, &[beta.clone()], BatchAction::Restore)
        .await
        .unwrap();
    let report = server
        .batch(PROJECT, &[gamma.clone()], BatchAction::Delete)
        .await
        .unwrap();
    assert_eq!(report.done, [gamma.clone()]);
    let ids = |list: desktop::ConversationList| {
        let mut ids: Vec<_> = list.rows.into_iter().map(|row| row.id).collect();
        ids.sort();
        ids
    };
    let mut kept = vec![alpha.clone(), beta.clone()];
    kept.sort();
    assert_eq!(
        ids(server.conversations(PROJECT, "", false).await.unwrap()),
        kept
    );
    assert!(server
        .conversations(PROJECT, "", true)
        .await
        .unwrap()
        .rows
        .is_empty());

    // How the sidebar is arranged is remembered, in its own key space.
    assert_eq!(server.preference("sidebar").await.unwrap(), None);
    server.set_preference("sidebar", "date,name").await.unwrap();
    assert_eq!(
        server.preference("sidebar").await.unwrap().as_deref(),
        Some("date,name")
    );
    for name in ["", "desktop.models", "Side Bar", &"x".repeat(41)] {
        assert!(server.set_preference(name, "x").await.is_err(), "{name}");
        assert!(server.preference(name).await.is_err(), "{name}");
    }
    assert!(server
        .set_preference("sidebar", &"x".repeat(2_001))
        .await
        .is_err());

    // Copy and move to another project. The transcript goes; the choice to
    // stop asking for approvals does not.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    server
        .set_conversation_options(
            PROJECT,
            &alpha,
            desktop::ConversationOptions {
                full_access: true,
                plan_first: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(server
        .transfer_conversation(PROJECT, &alpha, PROJECT, true)
        .await
        .is_err());
    assert!(server
        .transfer_conversation(PROJECT, &alpha, "missing", true)
        .await
        .is_err());
    assert!(server
        .transfer_conversation(&lab, &alpha, PROJECT, true)
        .await
        .is_err());
    let transcript = |project: String, id: String| {
        let server = &server;
        async move {
            let value = call(
                server,
                "native_conversation_snapshot",
                &project,
                json!({"session_id": id}),
            )
            .await
            .unwrap();
            serde_json::from_value::<dto::Snapshot>(value)
                .unwrap()
                .items
                .into_iter()
                .map(|item| item.text)
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    let original = transcript(PROJECT.into(), alpha.clone()).await;
    assert!(original.contains("sequence alignment plan"));

    let copy = server
        .transfer_conversation(PROJECT, &alpha, &lab, true)
        .await
        .unwrap();
    assert_ne!(copy, alpha);
    assert_eq!(transcript(lab.clone(), copy.clone()).await, original);
    assert_eq!(transcript(PROJECT.into(), alpha.clone()).await, original);
    let options = server
        .conversation_status(&lab, &copy)
        .await
        .unwrap()
        .options;
    assert!(options.plan_first);
    assert!(!options.full_access);
    // The original still has an unread reply; the copy does not ask for
    // attention in its new project.
    let status = |list: desktop::ConversationList, id: &str| {
        list.rows
            .into_iter()
            .find(|row| row.id == id)
            .map(|row| row.status)
    };
    assert_eq!(
        status(
            server.conversations(PROJECT, "", false).await.unwrap(),
            &alpha
        )
        .as_deref(),
        Some("needs_you")
    );
    assert_eq!(
        status(server.conversations(&lab, "", false).await.unwrap(), &copy).as_deref(),
        Some("complete")
    );

    let moved = server
        .transfer_conversation(PROJECT, &alpha, &lab, false)
        .await
        .unwrap();
    assert_eq!(transcript(lab.clone(), moved.clone()).await, original);
    assert!(server.conversation_status(PROJECT, &alpha).await.is_err());
    assert_eq!(
        ids(server.conversations(PROJECT, "", false).await.unwrap()),
        [beta.clone()]
    );
    let mut arrived = vec![copy, moved];
    arrived.sort();
    assert_eq!(
        ids(server.conversations(&lab, "", false).await.unwrap()),
        arrived
    );
    server.shutdown().await;
}

#[tokio::test]
async fn rewind_and_branch_act_on_the_named_message_or_change_nothing() {
    use desktop::TurnRef;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let session = create(&server).await;
    for message in ["first question", "second question", "third question"] {
        send(&server, &session, message).await.unwrap();
        wait_for(&server, &session, |s| !s.running).await;
    }
    // The transcript as a client sees it: who said what, in order.
    let said = |id: String| {
        let server = &server;
        async move {
            snapshot(server, &id)
                .await
                .items
                .into_iter()
                .map(|item| format!("{}: {}", item.role, item.text))
                .collect::<Vec<_>>()
        }
    };
    let whole = said(session.clone()).await;
    assert_eq!(
        whole,
        [
            "user: first question",
            "assistant: Saved result",
            "user: second question",
            "assistant: Saved result",
            "user: third question",
            "assistant: Saved result",
        ]
    );
    let turn = |from_end: usize, text: &str| TurnRef {
        from_end,
        text: text.into(),
    };

    // A reference that does not match the saved conversation is refused.
    for wrong in [
        turn(0, "second question"),
        turn(1, "second"),
        turn(3, "first question"),
        turn(5_000, "first question"),
    ] {
        for result in [
            server
                .rewind_conversation(PROJECT, &session, &wrong)
                .await
                .map(|_| ()),
            server
                .branch_conversation(PROJECT, &session, Some(&wrong))
                .await
                .map(|_| ()),
        ] {
            assert_eq!(result.unwrap_err().to_string(), desktop::TURN_NOT_FOUND);
        }
    }
    assert!(server
        .rewind_conversation("other-project", &session, &turn(0, "third question"))
        .await
        .is_err());
    assert_eq!(said(session.clone()).await, whole);
    assert_eq!(
        server
            .conversations(PROJECT, "", false)
            .await
            .unwrap()
            .rows
            .len(),
        1
    );

    // A branch copies the conversation through the named message's reply
    // and takes the source's group and choices with it.
    let group = server.create_folder(PROJECT, "Screens").await.unwrap();
    server
        .group_conversation(PROJECT, &session, Some(&group.id))
        .await
        .unwrap();
    server
        .set_conversation_options(
            PROJECT,
            &session,
            desktop::ConversationOptions {
                full_access: true,
                plan_first: true,
                reasoning_effort: "high".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let branch = server
        .branch_conversation(PROJECT, &session, Some(&turn(1, "second question")))
        .await
        .unwrap();
    assert_eq!(said(branch.clone()).await, whole[..4]);
    let full = server
        .branch_conversation(PROJECT, &session, None)
        .await
        .unwrap();
    assert_eq!(said(full.clone()).await, whole);
    // Naming the last message means the whole conversation too.
    let last = server
        .branch_conversation(PROJECT, &session, Some(&turn(0, "third question")))
        .await
        .unwrap();
    assert_eq!(said(last.clone()).await, whole);

    let list = server.conversations(PROJECT, "", false).await.unwrap();
    assert_eq!(list.rows.len(), 4);
    for id in [&branch, &full, &last] {
        assert_eq!(list.branched.get(id), Some(&session));
        assert_eq!(list.grouped.get(id), Some(&group.id));
        let row = list.rows.iter().find(|row| &row.id == id).unwrap();
        assert!(row.title.ends_with(" (branch)"), "{}", row.title);
        assert_eq!(row.status, "complete");
    }
    assert_eq!(list.branched.get(&session), None);
    let options = server
        .conversation_status(PROJECT, &branch)
        .await
        .unwrap()
        .options;
    assert!(options.full_access && options.plan_first);
    assert_eq!(options.reasoning_effort, "high");

    // A conversation with branches is not deleted or moved from under them.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    for result in [
        server.delete_conversation(PROJECT, &session).await,
        server
            .transfer_conversation(PROJECT, &session, &lab, false)
            .await
            .map(|_| ()),
    ] {
        assert!(result.unwrap_err().to_string().contains("branches"));
    }
    server.delete_conversation(PROJECT, &last).await.unwrap();
    server.delete_conversation(PROJECT, &full).await.unwrap();

    // Rewinding hands back the message and removes it and what followed.
    // The branch made earlier keeps its copy.
    let text = server
        .rewind_conversation(PROJECT, &session, &turn(1, "second question"))
        .await
        .unwrap();
    assert_eq!(text, "second question");
    assert_eq!(said(session.clone()).await, whole[..2]);
    assert_eq!(said(branch.clone()).await, whole[..4]);
    // The same reference no longer names anything.
    assert!(server
        .rewind_conversation(PROJECT, &session, &turn(1, "second question"))
        .await
        .is_err());

    // The conversation carries on from where it was cut.
    send(&server, &session, "a better question").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(
        said(session.clone()).await,
        [
            "user: first question",
            "assistant: Saved result",
            "user: a better question",
            "assistant: Saved result",
        ]
    );

    // Back to the very first message leaves an empty conversation that
    // still works.
    server
        .rewind_conversation(PROJECT, &session, &turn(1, "first question"))
        .await
        .unwrap();
    assert!(said(session.clone()).await.is_empty());
    assert!(server
        .branch_conversation(PROJECT, &session, None)
        .await
        .is_err());
    send(&server, &session, "start over").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(
        said(session.clone()).await,
        ["user: start over", "assistant: Saved result"]
    );
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_model_profiles_are_validated_saved_and_chosen_per_conversation() {
    use desktop::{ConversationOptions, ModelFields, ModelInput, ModelSource};
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    cfg.model = None;
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let used = Arc::new(std::sync::Mutex::new(Vec::new()));
    let factory = |used: Arc<std::sync::Mutex<Vec<(String, String)>>>| -> ModelFactory {
        Arc::new(move |provider| {
            used.lock()
                .unwrap()
                .push((provider.model.clone(), provider.api_key.clone()));
            Box::new(FakeModel {
                calls: Arc::default(),
                write: false,
            })
        })
    };
    let last_used = |used: &Arc<std::sync::Mutex<Vec<(String, String)>>>| {
        used.lock().unwrap().last().cloned().unwrap()
    };
    let fields = |model: &str| ModelFields {
        provider: "openai".into(),
        api_url: "http://127.0.0.1:9/v1".into(),
        model: model.into(),
        max_context: "32000".into(),
        max_tokens: "4000".into(),
        reasoning_effort: String::new(),
    };
    let input = |id: Option<&str>, name: &str, fields: ModelFields, key: Option<&str>| ModelInput {
        id: id.map(Into::into),
        name: name.into(),
        fields,
        api_key: key.map(Into::into),
    };
    let server = Server::open_with(cfg.clone(), factory(used.clone()), secrets.clone())
        .await
        .unwrap();
    let initial = server.model_settings().await.unwrap();
    assert_eq!(initial.source, ModelSource::None);
    assert!(initial.profiles.is_empty());

    let session = create(&server).await;
    let refused = send(&server, &session, "hello").await.unwrap_err();
    assert!(refused.contains("Configure a model"), "{refused}");

    let empty = server
        .save_model_profile(input(None, "", ModelFields::default(), None))
        .await
        .unwrap_err();
    assert_eq!(empty.to_string(), "Enter a model ID");
    // A rejected form changes nothing: no key, an unknown model without
    // limits, a name that is too long, a profile that does not exist.
    for bad in [
        input(None, "", fields("lab-model"), None),
        input(
            None,
            "",
            ModelFields {
                max_context: String::new(),
                ..fields("lab-model")
            },
            Some("first-key"),
        ),
        input(
            None,
            &"n".repeat(61),
            fields("lab-model"),
            Some("first-key"),
        ),
        input(Some("ghost"), "", fields("lab-model"), Some("first-key")),
    ] {
        assert!(server.save_model_profile(bad).await.is_err());
    }
    // A connection test uses the form as typed and saves nothing.
    assert_eq!(
        server
            .test_model(input(None, "", fields("lab-model"), Some("probe-key")))
            .await
            .unwrap(),
        "Saved result"
    );
    assert_eq!(last_used(&used), ("lab-model".into(), "probe-key".into()));
    assert!(server
        .test_model(input(None, "", fields("lab-model"), None))
        .await
        .is_err());
    assert!(server.model_settings().await.unwrap().profiles.is_empty());

    // The first profile becomes the default and is named after its model.
    let saved = server
        .save_model_profile(input(None, " ", fields("lab-model"), Some(" first-key ")))
        .await
        .unwrap();
    assert_eq!(saved.source, ModelSource::Saved);
    let first = saved.profiles[0].clone();
    assert_eq!(first.name, "lab-model");
    assert!(first.is_default && first.has_api_key && first.problem.is_none());
    send(&server, &session, "hello").await.unwrap();
    let done = wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(done.model_id, "lab-model");
    assert_eq!(last_used(&used), ("lab-model".into(), "first-key".into()));

    // Editing a profile keeps its stored key when none is typed.
    server
        .save_model_profile(input(
            Some(&first.id),
            "Lab",
            fields("lab-model-2"),
            Some("  "),
        ))
        .await
        .unwrap();
    send(&server, &session, "again").await.unwrap();
    wait_for(&server, &session, |s| {
        !s.running && s.model_id == "lab-model-2"
    })
    .await;
    assert_eq!(last_used(&used), ("lab-model-2".into(), "first-key".into()));

    // A second profile has its own key and does not take over.
    let both = server
        .save_model_profile(input(
            None,
            "Deep",
            fields("deep-model"),
            Some("second-key"),
        ))
        .await
        .unwrap();
    assert_eq!(both.profiles.len(), 2);
    let deep = both
        .profiles
        .iter()
        .find(|profile| profile.name == "Deep")
        .unwrap()
        .clone();
    assert!(!deep.is_default);
    let other_session = create(&server).await;

    // One conversation chooses it; the other keeps following the default.
    let choose = |model: &str| ConversationOptions {
        model: model.into(),
        ..Default::default()
    };
    assert!(server
        .set_conversation_options(PROJECT, &session, choose("ghost"))
        .await
        .is_err());
    server
        .set_conversation_options(PROJECT, &session, choose(&deep.id))
        .await
        .unwrap();
    assert_eq!(snapshot(&server, &session).await.model_id, "deep-model");
    assert_eq!(
        snapshot(&server, &other_session).await.model_id,
        "lab-model-2"
    );
    send(&server, &session, "third").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(last_used(&used), ("deep-model".into(), "second-key".into()));

    let switched = server.set_default_model(&deep.id).await.unwrap();
    assert!(switched
        .profiles
        .iter()
        .all(|profile| profile.is_default == (profile.id == deep.id)));
    assert_eq!(
        snapshot(&server, &other_session).await.model_id,
        "deep-model"
    );
    assert!(server.set_default_model("ghost").await.is_err());

    // Deleting the profile sends both conversations to what is left.
    let left = server.delete_model_profile(&deep.id).await.unwrap();
    assert_eq!(left.profiles.len(), 1);
    assert!(left.profiles[0].is_default);
    for id in [&session, &other_session] {
        assert_eq!(snapshot(&server, id).await.model_id, "lab-model-2");
    }
    assert!(server.delete_model_profile(&deep.id).await.is_err());

    // Keys are in the secret store only: not in settings, not in the database.
    let raw = server
        .0
        .store
        .get_setting("desktop.models")
        .await
        .unwrap()
        .unwrap();
    assert!(!raw.contains("first-key") && !raw.contains("second-key"));
    // Another data directory has its own keys, even with the same secret store.
    let other_root = tempfile::tempdir().unwrap();
    let mut other_cfg = config(other_root.path());
    other_cfg.model = None;
    let other = Server::open_with(other_cfg, factory(used.clone()), secrets.clone())
        .await
        .unwrap();
    assert!(other.model_settings().await.unwrap().profiles.is_empty());
    other.shutdown().await;
    server.shutdown().await;
    drop(server);

    // Profiles survive a restart; a lost key is reported, not fatal.
    let reopened = Server::open_with(cfg.clone(), factory(used.clone()), secrets)
        .await
        .unwrap();
    let restored = reopened.model_settings().await.unwrap();
    assert_eq!(restored.source, ModelSource::Saved);
    assert_eq!(restored.profiles[0].name, "Lab");
    assert_eq!(restored.profiles[0].fields.model, "lab-model-2");
    reopened.shutdown().await;
    drop(reopened);
    let without_key = Server::open_with(
        cfg.clone(),
        factory(used.clone()),
        Arc::new(desktop::MemorySecrets::default()),
    )
    .await
    .unwrap();
    let broken = without_key.model_settings().await.unwrap();
    assert_eq!(broken.source, ModelSource::None);
    let profile = &broken.profiles[0];
    assert!(!profile.has_api_key);
    assert!(profile
        .problem
        .as_ref()
        .unwrap()
        .contains("API key is missing"));
    assert_eq!(profile.fields.model, "lab-model-2");
    without_key.shutdown().await;
    drop(without_key);

    // WISP_* variables win and are not editable from the desktop.
    let from_env = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let env = from_env.model_settings().await.unwrap();
    assert_eq!(env.source, ModelSource::Environment);
    assert_eq!(env.profiles.len(), 1);
    assert_eq!(env.profiles[0].fields.model, "offline");
    assert!(env.profiles[0].is_default);
    let id = env.profiles[0].id.clone();
    for refused in [
        from_env
            .save_model_profile(input(None, "", fields("lab-model"), Some("k")))
            .await,
        from_env.set_default_model(&id).await,
        from_env.delete_model_profile(&id).await,
    ] {
        assert!(refused.unwrap_err().to_string().contains("environment"));
    }
    from_env.shutdown().await;
}

#[tokio::test]
async fn the_single_model_saved_by_an_earlier_build_is_kept_as_a_profile() {
    use desktop::{ModelFields, ModelInput, SecretStore};
    let temp = tempfile::tempdir().unwrap();
    let mut cfg = config(temp.path());
    cfg.model = None;
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let fields = ModelFields {
        provider: "openai".into(),
        api_url: "http://127.0.0.1:9/v1".into(),
        model: "old-model".into(),
        max_context: "32000".into(),
        max_tokens: "4000".into(),
        reasoning_effort: String::new(),
    };
    // What the earlier build left behind: one settings row and one key.
    {
        let server = Server::open_with(cfg.clone(), models(Arc::default(), false), secrets.clone())
            .await
            .unwrap();
        let store = &server.0.store;
        store
            .set_setting("desktop.model", &serde_json::to_string(&fields).unwrap())
            .await
            .unwrap();
        store
            .set_setting("desktop.model.key_id", "kid")
            .await
            .unwrap();
        secrets.set("desktop.model.api_key.kid", "old-key").unwrap();
        server.shutdown().await;
    }
    let server = Server::open_with(cfg, models(Arc::default(), false), secrets.clone())
        .await
        .unwrap();
    let settings = server.model_settings().await.unwrap();
    assert_eq!(settings.profiles.len(), 1);
    let old = settings.profiles[0].clone();
    assert_eq!(old.name, "old-model");
    assert!(old.is_default && old.has_api_key && old.problem.is_none());
    let session = create(&server).await;
    send(&server, &session, "hello").await.unwrap();
    assert_eq!(
        wait_for(&server, &session, |s| !s.running).await.model_id,
        "old-model"
    );
    // Adding another profile keeps it, still usable with its old key.
    let both = server
        .save_model_profile(ModelInput {
            id: None,
            name: "New".into(),
            fields: ModelFields {
                model: "new-model".into(),
                ..fields
            },
            api_key: Some("new-key".into()),
        })
        .await
        .unwrap();
    assert_eq!(both.profiles.len(), 2);
    let kept = both.profiles.iter().find(|p| p.id == old.id).unwrap();
    assert!(kept.is_default && kept.problem.is_none());
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_home_lists_recent_work_and_manages_projects() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), true))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("data.csv"), "a,b\n1,2\n").unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let session = call(&server, "native_conversation_create", &lab, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let running = || async {
        let value = call(
            &server,
            "native_conversation_snapshot",
            &lab,
            json!({"session_id": session}),
        )
        .await
        .unwrap();
        serde_json::from_value::<dto::Snapshot>(value).unwrap()
    };
    call(
        &server,
        "native_conversation_send",
        &lab,
        json!({"session_id": session, "request_id": "home-1", "message": "write the answer"}),
    )
    .await
    .unwrap();
    // The write tool waits for approval, which keeps the turn running.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while running().await.approvals.is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    // Recent work spans projects and says where each conversation lives.
    let recent = server.recent_conversations(10).await.unwrap();
    let row = recent.iter().find(|row| row.id == session).unwrap();
    assert_eq!(row.project_id, lab);
    assert_eq!(row.project_name, "lab");
    assert_eq!(row.status, "needs_you");
    assert!(server.recent_conversations(0).await.unwrap().is_empty());

    let host = &server;
    let starred = |id: String| async move {
        let value = call(host, "list_projects", "", json!({})).await.unwrap();
        serde_json::from_value::<Vec<wisp_dto::ProjectSummary>>(value)
            .unwrap()
            .into_iter()
            .find(|project| project.id == id)
            .map(|project| project.starred)
    };
    server.star_project(&lab, true).await.unwrap();
    assert_eq!(starred(lab.clone()).await, Some(true));
    server.star_project(&lab, false).await.unwrap();
    assert_eq!(starred(lab.clone()).await, Some(false));
    assert!(server.star_project("missing", true).await.is_err());

    let refused = server.remove_project(&lab).await.unwrap_err().to_string();
    assert!(refused.contains("Stop the running"), "{refused}");
    assert!(server.remove_project(PROJECT).await.is_err());
    assert!(server.remove_project("missing").await.is_err());
    call(
        &server,
        "native_conversation_stop",
        &lab,
        json!({"session_id": session}),
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while running().await.running {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();

    server.remove_project(&lab).await.unwrap();
    assert_eq!(starred(lab.clone()).await, None);
    assert!(server
        .recent_conversations(10)
        .await
        .unwrap()
        .iter()
        .all(|row| row.project_id != lab));
    // Removing a project never touches its folder.
    assert_eq!(
        std::fs::read_to_string(folder.join("data.csv")).unwrap(),
        "a,b\n1,2\n"
    );
    // The same folder can be opened again as a fresh project.
    let again = server.register_local_project(&folder).await.unwrap();
    assert_ne!(again, lab);

    // None of this is a remote command.
    for command in [
        "set_project_starred",
        "delete_project",
        "list_recent_sessions",
    ] {
        assert!(!server.commands().contains(&command));
        assert!(call(&server, command, &again, json!({})).await.is_err());
    }
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_search_and_project_files_need_no_conversation() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("data")).unwrap();
    std::fs::create_dir_all(folder.join(".wisp")).unwrap();
    std::fs::write(folder.join("data/counts.csv"), "gene,count\nTP53,7\n").unwrap();
    std::fs::write(folder.join(".wisp/counts.sqlite"), "internal").unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();

    // The project has no conversation yet; its files are still reachable.
    let root = server.project_directory(&lab, "").await.unwrap();
    assert_eq!(root.project_id, lab);
    assert_eq!(root.session_id, None);
    let names: Vec<_> = root.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["data"]);
    let file = server.project_file(&lab, "data/counts.csv").await.unwrap();
    assert_eq!(file.requested_path, "data/counts.csv");
    assert_eq!(file.content.text.as_deref(), Some("gene,count\nTP53,7\n"));
    for path in [
        "../lab/data/counts.csv",
        ".wisp/counts.sqlite",
        "/etc/hosts",
    ] {
        assert!(server.project_file(&lab, path).await.is_err(), "{path}");
    }
    assert!(server.project_directory("missing", "").await.is_err());

    // Name search covers every project, or just the one asked for.
    let everywhere = server.find_files(None, "COUNTS", 5).await.unwrap();
    assert_eq!(
        everywhere,
        [desktop::FileMatch {
            project_id: lab.clone(),
            project_name: "lab".into(),
            path: "data/counts.csv".into(),
        }]
    );
    assert!(server
        .find_files(Some(PROJECT), "counts", 5)
        .await
        .unwrap()
        .is_empty());
    assert!(server.find_files(None, " ", 5).await.unwrap().is_empty());

    // Conversation search reads messages in any project, archive included.
    let session = create(&server).await;
    send(&server, &session, "sequence alignment plan")
        .await
        .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let found = server.find_conversations(" alignment ", 5).await.unwrap();
    assert_eq!(found.len(), 1);
    // The instructions every conversation starts with are not its content:
    // a word that occurs only there must not match every conversation.
    let instructions = server
        .0
        .store
        .load_system_messages(std::slice::from_ref(&session))
        .await
        .unwrap()
        .remove(&session)
        .unwrap();
    let word = instructions
        .split(|c: char| !c.is_ascii_alphabetic())
        .max_by_key(|word| word.len())
        .unwrap();
    assert!(word.len() >= 6, "{word}");
    assert!(server.find_conversations(word, 5).await.unwrap().is_empty());
    assert_eq!(
        (found[0].id.as_str(), found[0].shelved),
        (session.as_str(), false)
    );
    server
        .archive_conversation(PROJECT, &session, true)
        .await
        .unwrap();
    let found = server.find_conversations("alignment", 5).await.unwrap();
    assert_eq!(
        (found[0].id.as_str(), found[0].shelved),
        (session.as_str(), true)
    );
    // With nothing typed it is the recent list, which leaves the archive out.
    assert!(server.find_conversations("", 5).await.unwrap().is_empty());
    assert!(server
        .find_conversations("alignment", 0)
        .await
        .unwrap()
        .is_empty());

    // None of this is a remote command.
    for command in [
        "native_project_file_directory",
        "native_project_file_read",
        "native_project_file_search",
        "search_sessions",
    ] {
        assert!(!server.commands().contains(&command));
        assert!(call(&server, command, &lab, json!({"path": ""}))
            .await
            .is_err());
    }
    server.shutdown().await;
}

#[tokio::test]
async fn desktop_conversation_options_shape_the_next_turn() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let efforts: Arc<std::sync::Mutex<Vec<Option<String>>>> = Arc::default();
    let seen = efforts.clone();
    let factory: ModelFactory = Arc::new(move |provider| {
        seen.lock().unwrap().push(provider.reasoning_effort.clone());
        Box::new(FakeModel {
            calls: Arc::default(),
            write: true,
        })
    });
    let server = Server::open_with_models(cfg.clone(), factory)
        .await
        .unwrap();
    let options = |full_access, effort: &str| desktop::ConversationOptions {
        full_access,
        reasoning_effort: effort.into(),
        ..Default::default()
    };
    let asking = create(&server).await;
    assert_eq!(
        server.conversation_status(PROJECT, &asking).await.unwrap(),
        desktop::ConversationStatus::default()
    );

    // Effort is validated, stored, and reaches the model on the next turn.
    assert!(server
        .set_conversation_options(PROJECT, &asking, options(false, "extreme"))
        .await
        .is_err());
    assert!(server
        .set_conversation_options("other", &asking, options(false, "high"))
        .await
        .is_err());
    assert_eq!(
        server
            .set_conversation_options(PROJECT, &asking, options(false, " high "))
            .await
            .unwrap(),
        options(false, "high")
    );
    send(&server, &asking, "write the answer").await.unwrap();
    let pending = wait_for(&server, &asking, |s| !s.approvals.is_empty()).await;
    assert_eq!(
        efforts.lock().unwrap().last().unwrap().as_deref(),
        Some("high")
    );
    let approval = pending.approvals[0].approval_id.clone();
    // "Deny for the conversation" does not exist; the request stays open.
    assert!(server
        .answer_approval(
            PROJECT,
            &asking,
            &approval,
            false,
            desktop::Remember::Conversation
        )
        .await
        .is_err());
    assert!(server
        .answer_approval(PROJECT, &asking, "missing", true, desktop::Remember::No)
        .await
        .is_err());
    server
        .answer_approval(
            PROJECT,
            &asking,
            &approval,
            true,
            desktop::Remember::Conversation,
        )
        .await
        .unwrap();
    wait_for(&server, &asking, |s| !s.running).await;
    assert!(cfg.workspace.join("answer.txt").exists());
    let status = server.conversation_status(PROJECT, &asking).await.unwrap();
    assert_eq!(status.allowed_tools, ["write"]);
    assert_eq!(status.options, options(false, "high"));
    assert!(status.usage.is_some());
    server.revoke_allowed_tools(PROJECT, &asking).await.unwrap();
    assert!(server
        .conversation_status(PROJECT, &asking)
        .await
        .unwrap()
        .allowed_tools
        .is_empty());

    // Full access: the same write runs without a question.
    std::fs::remove_file(cfg.workspace.join("answer.txt")).unwrap();
    let trusted = create(&server).await;
    server
        .set_conversation_options(PROJECT, &trusted, options(true, ""))
        .await
        .unwrap();
    send(&server, &trusted, "write the answer").await.unwrap();
    let done = wait_for(&server, &trusted, |s| !s.running).await;
    assert!(done.approvals.is_empty() && done.error.is_none());
    assert!(cfg.workspace.join("answer.txt").exists());
    assert_eq!(efforts.lock().unwrap().last().unwrap(), &None);
    // It belongs to that conversation only, and goes away with it.
    assert!(
        !server
            .conversation_status(PROJECT, &asking)
            .await
            .unwrap()
            .options
            .full_access
    );
    server.delete_conversation(PROJECT, &trusted).await.unwrap();
    assert_eq!(
        server
            .0
            .store
            .get_setting(&format!("desktop.conversation.full_access.{trusted}"))
            .await
            .unwrap(),
        None
    );

    // None of this is a remote command, and a remote approval is still once.
    for command in [
        "set_session_full_permission",
        "set_session_reasoning_effort",
    ] {
        assert!(!server.commands().contains(&command));
        assert!(
            call(&server, command, PROJECT, json!({"session_id": asking}))
                .await
                .is_err()
        );
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_skill_that_is_turned_off_is_not_offered_to_the_model_or_the_composer() {
    let temp = tempfile::tempdir().unwrap();
    // The host is told about one directory of skills; the project has its own.
    let shared = temp.path().join("shared-skills");
    let skill = |dir: &Path, name: &str, about: &str| {
        std::fs::create_dir_all(dir.join(name)).unwrap();
        std::fs::write(
            dir.join(name).join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {about}\ntags: [lab]\n---\nDo it.\n"),
        )
        .unwrap();
    };
    skill(&shared, "zebra-count", "Count stripes the lab way");
    let mut settings = config(temp.path());
    settings.skills = vec![shared];
    let server = Server::open_with_models(settings, models(Arc::default(), false))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    skill(
        &folder.join("skills"),
        "Alpha-fold-check",
        "Check a predicted fold",
    );
    let lab = server.register_local_project(&folder).await.unwrap();

    // Listed by name whatever the case, each with where it came from.
    let listed = |project: Option<String>| {
        let server = &server;
        async move {
            server
                .skills(project.as_deref())
                .await
                .unwrap()
                .into_iter()
                .map(|row| format!("{} {} {}", row.name, row.source, row.enabled))
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        listed(Some(lab.clone())).await,
        ["Alpha-fold-check project true", "zebra-count extra true"]
    );
    // Without a project, only what the host has for every project.
    assert_eq!(listed(None).await, ["zebra-count extra true"]);
    assert!(server.skills(Some("missing")).await.is_err());
    assert_eq!(
        server.skills(Some(&lab)).await.unwrap()[1].tags,
        ["lab".to_string()]
    );

    let offered = |query: &'static str| {
        let (server, lab) = (&server, &lab);
        async move {
            server
                .skill_options(lab, query, 10)
                .await
                .unwrap()
                .into_iter()
                .map(|option| option.label)
                .collect::<Vec<_>>()
        }
    };
    // What a turn's agent is given: the index its skill tools are built on.
    let for_turns = || {
        let (server, folder) = (&server, &folder);
        async move {
            let index =
                crate::runtime::skill_index(&server.0.config, &server.0.store, folder).await;
            let mut names: Vec<_> = index.all().iter().map(|skill| skill.name.clone()).collect();
            names.sort();
            names
        }
    };
    assert_eq!(offered("").await, ["Alpha-fold-check", "zebra-count"]);
    assert_eq!(for_turns().await, ["Alpha-fold-check", "zebra-count"]);

    // Off: gone from the model's view and from the composer, still listed.
    server
        .set_skill_enabled("zebra-count", false)
        .await
        .unwrap();
    assert!(server.set_skill_enabled(" ", false).await.is_err());
    assert_eq!(offered("").await, ["Alpha-fold-check"]);
    assert_eq!(for_turns().await, ["Alpha-fold-check"]);
    assert_eq!(
        listed(Some(lab.clone())).await,
        ["Alpha-fold-check project true", "zebra-count extra false"]
    );
    // Turning off twice, or a skill that is not installed, is harmless.
    server
        .set_skill_enabled("zebra-count", false)
        .await
        .unwrap();
    server
        .set_skill_enabled("not-installed", false)
        .await
        .unwrap();

    server.set_skill_enabled("zebra-count", true).await.unwrap();
    assert_eq!(offered("zebra").await, ["zebra-count"]);
    assert_eq!(for_turns().await, ["Alpha-fold-check", "zebra-count"]);
    server.shutdown().await;
}

#[tokio::test]
async fn a_bundled_connector_that_is_turned_off_gives_the_model_no_tools() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();

    // One row per connector, by name, each with the tools it brings.
    let rows = server.connectors().await.unwrap();
    let domains: Vec<_> = rows.iter().map(|row| row.domain.as_str()).collect();
    let mut sorted = domains.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(domains, sorted);
    assert!(rows.len() > 10);
    let pubmed = rows.iter().find(|row| row.domain == "pubmed").unwrap();
    assert!(pubmed.enabled && !pubmed.tools.is_empty());
    assert!(!pubmed.description.is_empty() && !pubmed.links.is_empty());
    let total: usize = rows.iter().map(|row| row.tools.len()).sum();
    assert_eq!(total, wisp_bio::catalog().len());

    // What a turn's agent is given.
    let given = || {
        let server = &server;
        async move {
            crate::runtime::connector_tools(&server.0.store)
                .await
                .iter()
                .map(|tool| tool.name().to_owned())
                .collect::<std::collections::HashSet<_>>()
        }
    };
    let all = given().await;
    assert_eq!(all.len(), total);
    assert!(pubmed.tools.iter().all(|tool| all.contains(tool)));

    server.set_connector_enabled("pubmed", false).await.unwrap();
    assert!(server
        .set_connector_enabled("not-a-connector", false)
        .await
        .is_err());
    let without = given().await;
    assert!(pubmed.tools.iter().all(|tool| !without.contains(tool)));
    assert_eq!(without.len(), total - pubmed.tools.len());
    let rows = server.connectors().await.unwrap();
    assert!(rows
        .iter()
        .all(|row| row.enabled == (row.domain != "pubmed")));

    // A turn still runs with the rest in place.
    let session = create(&server).await;
    send(&server, &session, "hello").await.unwrap();
    let done = wait_for(&server, &session, |s| !s.running).await;
    assert!(done.error.is_none());

    server.set_connector_enabled("pubmed", true).await.unwrap();
    assert_eq!(given().await, all);
    server.shutdown().await;
}

#[tokio::test]
async fn turn_defaults_and_project_grants_apply_from_the_next_turn() {
    use desktop::Remember;
    let temp = tempfile::tempdir().unwrap();
    // Counts the calls the model was given no tools for: the agent's last
    // word when a turn runs out of steps.
    struct Counting(FakeModel, Arc<AtomicUsize>);
    #[async_trait]
    impl Provider for Counting {
        fn name(&self) -> &str {
            self.0.name()
        }
        fn model(&self) -> &str {
            self.0.model()
        }
        async fn complete(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            self.0.complete(messages, tools).await
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            if tools.is_empty() {
                self.1.fetch_add(1, Ordering::Relaxed);
            }
            self.0.stream(messages, tools, sink).await
        }
    }
    let out_of_steps = Arc::new(AtomicUsize::new(0));
    let counter = out_of_steps.clone();
    let factory: ModelFactory = Arc::new(move |_| {
        Box::new(Counting(
            FakeModel {
                calls: Arc::default(),
                write: true,
            },
            counter.clone(),
        ))
    });
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();

    // Until something is saved, turns run as the host was started.
    let start = server.turn_defaults().await.unwrap();
    assert_eq!(start.max_iterations, 5);
    assert!(start.auto_compact && !start.full_access);
    assert_eq!(start.auto_compact_percent, 80);
    assert_eq!(start.compact_range, (10, 95));
    for wrong in [
        desktop::TurnDefaults {
            max_iterations: 0,
            ..start
        },
        desktop::TurnDefaults {
            max_iterations: 1_001,
            ..start
        },
        desktop::TurnDefaults {
            auto_compact_percent: 9,
            ..start
        },
        desktop::TurnDefaults {
            auto_compact_percent: 96,
            ..start
        },
    ] {
        assert!(server.set_turn_defaults(wrong).await.is_err());
    }
    assert_eq!(server.turn_defaults().await.unwrap(), start);
    let saved = server
        .set_turn_defaults(desktop::TurnDefaults {
            max_iterations: 7,
            auto_compact: false,
            auto_compact_percent: 60,
            full_access: true,
            ..start
        })
        .await
        .unwrap();
    assert_eq!(saved.max_iterations, 7);
    assert!(!saved.auto_compact && saved.full_access);
    assert_eq!(saved.auto_compact_percent, 60);
    assert_eq!(server.turn_defaults().await.unwrap(), saved);

    // A conversation made by the desktop starts the way the default says;
    // one made before keeps what it had.
    let trusting = server.create_conversation(PROJECT).await.unwrap();
    let full = |id: String| {
        let server = &server;
        async move {
            server
                .conversation_status(PROJECT, &id)
                .await
                .unwrap()
                .options
                .full_access
        }
    };
    assert!(full(trusting.clone()).await);
    server
        .set_turn_defaults(desktop::TurnDefaults {
            full_access: false,
            ..saved
        })
        .await
        .unwrap();
    let asking = server.create_conversation(PROJECT).await.unwrap();
    assert!(!full(asking.clone()).await);
    assert!(full(trusting).await);
    assert!(server.create_conversation("missing").await.is_err());

    // "Always" cannot be said while denying, and the request stays open.
    send(&server, &asking, "write the answer").await.unwrap();
    let approval = wait_for(&server, &asking, |s| !s.approvals.is_empty())
        .await
        .approvals[0]
        .approval_id
        .clone();
    assert!(server
        .answer_approval(PROJECT, &asking, &approval, false, Remember::Project)
        .await
        .is_err());
    assert!(server.project_grants().await.unwrap().is_empty());
    server
        .answer_approval(PROJECT, &asking, &approval, true, Remember::Project)
        .await
        .unwrap();
    wait_for(&server, &asking, |s| !s.running).await;
    assert_eq!(
        server.project_grants().await.unwrap(),
        [desktop::ProjectGrants {
            project_id: PROJECT.into(),
            project_name: "Server".into(),
            tools: vec!["write".into()],
        }]
    );

    // Another conversation of the project is not asked at all.
    let second = server.create_conversation(PROJECT).await.unwrap();
    send(&server, &second, "write the answer").await.unwrap();
    let done = wait_for(&server, &second, |s| !s.running).await;
    assert!(done.approvals.is_empty());
    assert!(done.items.iter().any(|item| item.role == "tool"));

    // Taken back, the next conversation is asked again.
    server.revoke_project_grant(PROJECT, "write").await.unwrap();
    assert!(server.revoke_project_grant(PROJECT, "write").await.is_err());
    assert!(server.project_grants().await.unwrap().is_empty());
    let third = server.create_conversation(PROJECT).await.unwrap();
    send(&server, &third, "write the answer").await.unwrap();
    let approval = wait_for(&server, &third, |s| !s.approvals.is_empty())
        .await
        .approvals[0]
        .approval_id
        .clone();
    server
        .answer_approval(PROJECT, &third, &approval, true, Remember::No)
        .await
        .unwrap();
    wait_for(&server, &third, |s| !s.running).await;

    // The step limit is read at the start of each turn. Writing and then
    // answering takes two steps; with one, the turn runs out after the
    // write and the agent is asked for a last word without tools.
    assert_eq!(out_of_steps.load(Ordering::Relaxed), 0);
    server
        .set_turn_defaults(desktop::TurnDefaults {
            max_iterations: 1,
            full_access: true,
            ..saved
        })
        .await
        .unwrap();
    let limited = server.create_conversation(PROJECT).await.unwrap();
    send(&server, &limited, "write the answer").await.unwrap();
    wait_for(&server, &limited, |s| !s.running).await;
    assert_eq!(out_of_steps.load(Ordering::Relaxed), 1);
    server.shutdown().await;
}

/// Records every message of every request, whatever its role.
struct Everything(Arc<std::sync::Mutex<Vec<String>>>);
#[async_trait]
impl Provider for Everything {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "noted".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        {
            let mut seen = self.0.lock().unwrap();
            seen.extend(messages.iter().map(|message| message.content.as_text()));
            seen.extend(tools.iter().map(|tool| format!("{tool:?}")));
        }
        sink.on_text("noted");
        self.complete(messages, tools).await
    }
}

#[tokio::test]
async fn memos_are_the_researchers_own_and_never_shown_to_the_model() {
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();

    let draft = server
        .add_memo(
            PROJECT,
            "  Prompt draft ",
            " ask about the quokka-7 plasmid ",
        )
        .await
        .unwrap();
    assert_eq!(draft.title, "Prompt draft");
    assert_eq!(draft.body, "ask about the quokka-7 plasmid");
    assert!(!draft.pinned);
    // A title is optional; a body is not, and both have limits.
    let untitled = server.add_memo(PROJECT, "", "ssh lab-gpu").await.unwrap();
    for (title, body) in [
        ("empty", "  ".to_string()),
        ("long body", "x".repeat(20_001)),
        (&"t".repeat(201), "body".to_string()),
    ] {
        assert!(server.add_memo(PROJECT, title, &body).await.is_err());
    }
    assert!(server.add_memo("missing", "", "body").await.is_err());
    assert!(server.memos("missing").await.is_err());

    let titles = |memos: Vec<desktop::ProjectMemo>| {
        memos.into_iter().map(|memo| memo.body).collect::<Vec<_>>()
    };
    // Newest first, until one is pinned.
    assert_eq!(
        titles(server.memos(PROJECT).await.unwrap()),
        ["ssh lab-gpu", "ask about the quokka-7 plasmid"]
    );
    let patch = |id: &str| desktop::ProjectMemoPatch {
        id: id.into(),
        title: None,
        body: None,
        pinned: None,
    };
    let pinned = server
        .update_memo(
            PROJECT,
            desktop::ProjectMemoPatch {
                pinned: Some(true),
                ..patch(&draft.id)
            },
        )
        .await
        .unwrap();
    assert!(pinned.pinned);
    assert_eq!(pinned.body, draft.body);
    assert_eq!(
        titles(server.memos(PROJECT).await.unwrap()),
        ["ask about the quokka-7 plasmid", "ssh lab-gpu"]
    );
    let edited = server
        .update_memo(
            PROJECT,
            desktop::ProjectMemoPatch {
                title: Some("Server".into()),
                body: Some("ssh lab-gpu -p 2222".into()),
                ..patch(&untitled.id)
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (edited.title.as_str(), edited.body.as_str()),
        ("Server", "ssh lab-gpu -p 2222")
    );
    assert!(server
        .update_memo(
            PROJECT,
            desktop::ProjectMemoPatch {
                body: Some(" ".into()),
                ..patch(&untitled.id)
            }
        )
        .await
        .is_err());

    // A memo belongs to its project: another one neither lists nor changes it.
    assert!(server.memos(&lab).await.unwrap().is_empty());
    assert!(server.update_memo(&lab, patch(&draft.id)).await.is_err());
    assert!(server.delete_memo(&lab, &draft.id).await.is_err());

    // A whole turn runs without the memo reaching the model in any message
    // or tool description.
    let session = create(&server).await;
    send(&server, &session, "what should I ask next?")
        .await
        .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let seen = seen.lock().unwrap().join("\n");
    assert!(seen.contains("what should I ask next?"));
    assert!(!seen.contains("quokka-7"), "a memo reached the model");
    assert!(!seen.contains("lab-gpu"), "a memo reached the model");

    server.delete_memo(PROJECT, &draft.id).await.unwrap();
    assert!(server.delete_memo(PROJECT, &draft.id).await.is_err());
    assert_eq!(
        titles(server.memos(PROJECT).await.unwrap()),
        ["ssh lab-gpu -p 2222"]
    );
    server.shutdown().await;
}

#[tokio::test]
async fn memory_reaches_the_model_only_while_it_is_on_and_is_never_saved_with_a_turn() {
    let temp = tempfile::tempdir().unwrap();
    let seen: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let record = seen.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();

    // Memory starts on and empty.
    let view = server.memory(&lab).await.unwrap();
    assert!(view.enabled);
    assert!(view.notes.is_empty() && view.habits.is_empty());
    assert!(view.today.ends_with(".md"));
    assert!(server.memory("missing").await.is_err());

    // A remembered line lands in today's note of that project only.
    server
        .remember(&lab, None, "  the queue is called gpu-long  ", false)
        .await
        .unwrap();
    let view = server.memory(&lab).await.unwrap();
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.notes[0].name, view.today);
    assert_eq!(view.notes[0].preview.trim(), "the queue is called gpu-long");
    assert!(folder.join(".wisp/memory").join(&view.today).is_file());
    assert!(server.memory(PROJECT).await.unwrap().notes.is_empty());
    for (text, why) in [("  ", "empty"), (&"x".repeat(4_001), "too long")] {
        assert!(
            server.remember(&lab, None, text, false).await.is_err(),
            "{why}"
        );
        assert!(server.add_habit(text).await.is_err(), "{why}");
    }

    // A note is edited as a whole file, by a name that stays in the folder.
    server
        .save_memory_note(&lab, "2026-01-02.md", "older finding\n")
        .await
        .unwrap();
    assert_eq!(
        server.memory_note(&lab, "2026-01-02.md").await.unwrap(),
        "older finding\n"
    );
    assert_eq!(server.memory_note(&lab, "2020-01-01.md").await.unwrap(), "");
    for name in [
        "",
        "notes.txt",
        "../escape.md",
        "sub/note.md",
        "sub\\note.md",
        ".hidden.md",
    ] {
        assert!(
            server.save_memory_note(&lab, name, "x").await.is_err(),
            "{name}"
        );
        assert!(server.memory_note(&lab, name).await.is_err(), "{name}");
        assert!(
            server.delete_memory_note(&lab, name).await.is_err(),
            "{name}"
        );
    }
    assert!(server
        .save_memory_note(&lab, "2026-01-02.md", "  ")
        .await
        .is_err());
    let names: Vec<_> = server
        .memory(&lab)
        .await
        .unwrap()
        .notes
        .into_iter()
        .map(|note| note.name)
        .collect();
    assert_eq!(names, [view.today.clone(), "2026-01-02.md".into()]);
    server
        .delete_memory_note(&lab, "2026-01-02.md")
        .await
        .unwrap();
    assert!(server
        .delete_memory_note(&lab, "2026-01-02.md")
        .await
        .is_err());

    // Habits belong to the user, not to a project.
    let habit = server.add_habit(" always report medians ").await.unwrap();
    assert_eq!(habit.content, "always report medians");
    server
        .update_habit(&habit.id, "always report medians and IQR")
        .await
        .unwrap();
    assert!(server.update_habit("missing", "text").await.is_err());
    let session = call(&server, "native_conversation_create", &lab, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    server
        .remember(&lab, Some(&session), "prefers vector figures", true)
        .await
        .unwrap();
    // A conversation of another project cannot be named as the source.
    assert!(server
        .remember(PROJECT, Some(&session), "stolen", true)
        .await
        .is_err());
    let habits = |view: desktop::MemoryView| {
        view.habits
            .into_iter()
            .map(|habit| habit.content)
            .collect::<Vec<_>>()
    };
    // Most recently written or changed first; the edit above counts.
    let expected = ["always report medians and IQR", "prefers vector figures"];
    assert_eq!(habits(server.memory(&lab).await.unwrap()), expected);
    assert_eq!(habits(server.memory(PROJECT).await.unwrap()), expected);

    // On: the turn carries the habits and can search the notes.
    let turn = |message: &'static str| {
        let (server, lab, seen) = (&server, &lab, &seen);
        async move {
            seen.lock().unwrap().clear();
            let session = call(server, "native_conversation_create", lab, json!({}))
                .await
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned();
            call(
                server,
                "native_conversation_send",
                lab,
                json!({"session_id": session, "request_id": message, "message": message}),
            )
            .await
            .unwrap();
            let done = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let value = call(
                        server,
                        "native_conversation_snapshot",
                        lab,
                        json!({"session_id": session}),
                    )
                    .await
                    .unwrap();
                    let snapshot: dto::Snapshot = serde_json::from_value(value).unwrap();
                    if !snapshot.running {
                        return snapshot;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            (seen.lock().unwrap().join("\n"), done)
        }
    };
    // The system prompt always explains the block; this is the block itself.
    const BLOCK: &str = "<global_memory>\nHost-provided user context";
    let (shown, saved) = turn("plot the counts").await;
    assert!(shown.contains(BLOCK));
    assert!(shown.contains("always report medians and IQR"));
    assert!(shown.contains("search_memory"));
    // The habits were for that turn; the saved message is what was typed.
    let user: Vec<_> = saved
        .items
        .iter()
        .filter(|item| item.role == "user")
        .map(|item| item.text.as_str())
        .collect();
    assert_eq!(user, ["plot the counts"]);

    // Off: neither reaches the model, and nothing was deleted.
    server.set_memory_enabled(false).await.unwrap();
    let (shown, _) = turn("plot them again").await;
    assert!(shown.contains("plot them again"));
    assert!(!shown.contains(BLOCK));
    assert!(!shown.contains("always report medians"));
    assert!(!shown.contains("search_memory"));
    let view = server.memory(&lab).await.unwrap();
    assert!(!view.enabled);
    assert_eq!(view.notes.len(), 1);
    assert_eq!(view.habits.len(), 2);

    server.set_memory_enabled(true).await.unwrap();
    server.delete_habit(&habit.id).await.unwrap();
    let (shown, _) = turn("and once more").await;
    assert!(shown.contains("prefers vector figures"));
    assert!(!shown.contains("always report medians"));
    server.shutdown().await;
}

#[tokio::test]
async fn favorites_are_snapshots_that_outlive_the_conversation_they_came_from() {
    let temp = tempfile::tempdir().unwrap();
    let settings = config(temp.path());
    let library = settings.data.join("library.sqlite");
    let server = Server::open_with_models(settings, models(Arc::default(), false))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("figs")).unwrap();
    std::fs::create_dir_all(folder.join(".wisp")).unwrap();
    let picture = b"\x89PNG\r\n\x1a\nnot really a picture".to_vec();
    std::fs::write(folder.join("figs/plot.png"), &picture).unwrap();
    std::fs::write(folder.join(".wisp/secret.png"), &picture).unwrap();
    std::fs::write(folder.join("notes.txt"), "plain").unwrap();
    std::fs::write(temp.path().join("outside.png"), &picture).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let session = call(&server, "native_conversation_create", &lab, json!({}))
        .await
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    // A host nobody saved anything on has no library file.
    assert!(!library.exists());

    let finding = "  The knockout halves viability.\nIt holds in both lines.";
    let text = server
        .star_text(&lab, &session, "text", None, finding)
        .await
        .unwrap();
    assert_eq!(text.kind, "text");
    assert_eq!(text.title, "The knockout halves viability.");
    assert_eq!(text.code, finding);
    assert_eq!(text.source_project_name, "lab");
    assert_eq!(text.source_session_id, session);
    assert!(library.exists());
    // Saving the same passage again is the same item.
    let again = server
        .star_text(&lab, &session, "text", None, finding)
        .await
        .unwrap();
    assert_eq!(again.id, text.id);
    let code = server
        .star_text(
            &lab,
            &session,
            "code",
            Some(" python "),
            "import scanpy as sc\n",
        )
        .await
        .unwrap();
    assert_eq!(code.language.as_deref(), Some("python"));
    for (kind, body) in [
        ("figure", "x".to_string()),
        ("text", "  \n".to_string()),
        ("text", "x".repeat(2 * 1024 * 1024 + 1)),
    ] {
        assert!(server
            .star_text(&lab, &session, kind, None, &body)
            .await
            .is_err());
    }
    assert!(server
        .star_text(PROJECT, &session, "text", None, "stolen")
        .await
        .is_err());

    // A figure is a copy of an image inside the project.
    let figure = server
        .star_figure(&lab, &session, "figs/plot.png")
        .await
        .unwrap();
    assert_eq!(figure.kind, "figure");
    assert_eq!(figure.title, "plot.png");
    assert_eq!(figure.content_type.as_deref(), Some("image/png"));
    assert_eq!(figure.source_path.as_deref(), Some("figs/plot.png"));
    for path in [
        "notes.txt",
        "../outside.png",
        ".wisp/secret.png",
        "figs/missing.png",
    ] {
        assert!(
            server.star_figure(&lab, &session, path).await.is_err(),
            "{path}"
        );
    }
    std::fs::remove_file(folder.join("figs/plot.png")).unwrap();
    let kept = server.favorite(&figure.id).await.unwrap();
    assert_eq!(kept.image.as_deref(), Some(picture.as_slice()));
    assert_eq!(server.favorite(&text.id).await.unwrap().image, None);

    let ids = |items: Vec<desktop::LibraryItemSummary>| {
        items.into_iter().map(|item| item.id).collect::<Vec<_>>()
    };
    let newest_first = vec![figure.id.clone(), code.id.clone(), text.id.clone()];
    assert_eq!(ids(server.favorites("", None).await.unwrap()), newest_first);
    assert_eq!(
        ids(server.favorites(" KNOCKOUT ", None).await.unwrap()),
        [text.id.clone()]
    );
    assert_eq!(
        ids(server.favorites("", Some("figure")).await.unwrap()),
        [figure.id.clone()]
    );
    // The project's name finds what came from it.
    assert_eq!(
        ids(server.favorites("lab", Some("code")).await.unwrap()),
        [code.id.clone()]
    );
    assert!(server
        .favorites("nothing like it", None)
        .await
        .unwrap()
        .is_empty());
    let here: Vec<_> = server
        .conversation_favorites(&lab, &session)
        .await
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(here, newest_first);
    assert!(server
        .conversation_favorites(PROJECT, &session)
        .await
        .is_err());

    // The conversation goes; what was saved from it stays.
    server.delete_conversation(&lab, &session).await.unwrap();
    assert_eq!(ids(server.favorites("", None).await.unwrap()), newest_first);
    assert_eq!(server.favorite(&text.id).await.unwrap().item.code, finding);

    server.unstar(&text.id).await.unwrap();
    assert!(server.unstar(&text.id).await.is_err());
    assert!(server.favorite(&text.id).await.is_err());
    assert_eq!(
        ids(server.favorites("", None).await.unwrap()),
        [figure.id, code.id]
    );
    server.shutdown().await;
}

/// Records what the model was shown, turn by turn.
struct Recorder(Arc<std::sync::Mutex<Vec<Vec<String>>>>);
#[async_trait]
impl Provider for Recorder {
    fn name(&self) -> &str {
        "offline"
    }
    fn model(&self) -> &str {
        "offline"
    }
    async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
        Ok(Completion {
            content: "noted".into(),
            finish_reason: Some("stop".into()),
            ..Default::default()
        })
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> wisp_llm::Result<Completion> {
        self.0.lock().unwrap().push(
            messages
                .iter()
                .filter(|message| message.role == wisp_llm::Role::User)
                .map(|message| message.content.as_text())
                .collect(),
        );
        sink.on_text("noted");
        self.complete(messages, tools).await
    }
}

#[tokio::test]
async fn desktop_messages_carry_project_files_and_turn_scoped_references() {
    use wisp_dto::ComposerReferenceArg as Reference;
    let temp = tempfile::tempdir().unwrap();
    let shown: Arc<std::sync::Mutex<Vec<Vec<String>>>> = Arc::default();
    let record = shown.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Recorder(record.clone())));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("data")).unwrap();
    std::fs::create_dir_all(folder.join("skills/demo")).unwrap();
    std::fs::write(folder.join("data/counts.csv"), "gene,count\n").unwrap();
    std::fs::write(
        folder.join("skills/demo/SKILL.md"),
        "---\nname: demo\ndescription: Count reads the lab way\n---\nAlways report medians.\n",
    )
    .unwrap();
    let outside = temp.path().join("notes.txt");
    std::fs::write(&outside, "field notes").unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let new_conversation = || async {
        call(&server, "native_conversation_create", &lab, json!({}))
            .await
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    };
    let request = |session: &str, id: &str, message: &str| dto::SendRequest {
        session_id: session.into(),
        request_id: id.into(),
        message: message.into(),
        resume: false,
        resume_after: None,
        attachments: vec![],
        references: vec![],
        acp_agent_id: None,
    };
    let finished = |session: String| {
        let (server, lab) = (&server, &lab);
        async move {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let value = call(
                        server,
                        "native_conversation_snapshot",
                        lab,
                        json!({"session_id": session}),
                    )
                    .await
                    .unwrap();
                    let snapshot: dto::Snapshot = serde_json::from_value(value).unwrap();
                    if !snapshot.running {
                        return snapshot;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap()
        }
    };

    // Picking files: one outside the project is copied in, one inside is not.
    let files = server
        .import_attachments(&lab, &[folder.join("data/counts.csv"), outside])
        .await
        .unwrap();
    assert_eq!(files, ["data/counts.csv", "uploads/notes.txt"]);
    let skills = server.skill_options(&lab, "lab COUNT", 5).await.unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].label, "demo");
    assert!(server
        .skill_options(&lab, "absent", 5)
        .await
        .unwrap()
        .is_empty());

    let earlier = new_conversation().await;
    server
        .send(&lab, request(&earlier, "r1", "alpha finding about TP53"))
        .await
        .unwrap();
    finished(earlier.clone()).await;

    let session = new_conversation().await;
    // Nothing that cannot be honoured is sent, and nothing is saved for it.
    for (attachments, references) in [
        (vec!["../notes.txt".to_string()], vec![]),
        (vec!["data".to_string()], vec![]),
        (
            vec![],
            vec![Reference::Skill {
                name: "ghost".into(),
            }],
        ),
        (vec![], vec![Reference::Session { id: "ghost".into() }]),
        (vec![], vec![Reference::Artifact { id: "a".into() }]),
    ] {
        let mut bad = request(&session, "bad", "use these");
        bad.attachments = attachments;
        bad.references = references;
        assert!(server.send(&lab, bad).await.is_err());
    }
    assert!(finished(session.clone()).await.items.is_empty());
    assert!(server
        .send("other", request(&session, "bad", "hello"))
        .await
        .is_err());

    let mut full = request(&session, "r2", "use these");
    full.attachments = files.clone();
    full.references = vec![
        Reference::Skill {
            name: "demo".into(),
        },
        Reference::Session {
            id: earlier.clone(),
        },
        Reference::Skill {
            name: "demo".into(),
        },
    ];
    server.send(&lab, full).await.unwrap();
    let done = finished(session.clone()).await;
    // The files are part of the saved message and listed on it.
    let asked = &done.items[0];
    assert_eq!(
        asked.text,
        "use these\n\nUploaded files: data/counts.csv, uploads/notes.txt"
    );
    assert_eq!(asked.attachments, files);
    // The model saw the references ahead of the request, each once.
    let turn = shown.lock().unwrap().last().unwrap().clone();
    let position = |needle: &str| turn.iter().position(|text| text.contains(needle));
    let request_at = position("use these").unwrap();
    assert!(position("alpha finding about TP53").unwrap() < request_at);
    assert!(position("Always report medians.").unwrap() < request_at);
    assert_eq!(
        turn.iter()
            .filter(|text| text.contains("# Skill: demo"))
            .count(),
        1
    );
    // They belong to that turn only: not saved, and not shown to the next.
    let saved = server.0.store.load_messages(&session).await.unwrap();
    assert!(saved
        .iter()
        .all(|message| !message.content.as_text().contains("Always report medians.")));
    server
        .send(&lab, request(&session, "r3", "and now?"))
        .await
        .unwrap();
    finished(session.clone()).await;
    let next = shown.lock().unwrap().last().unwrap().clone();
    assert!(next.iter().any(|text| text.contains("and now?")));
    assert!(next
        .iter()
        .all(|text| !text.contains("Always report medians.")));

    // A message can be files alone; it is named after the first one.
    let files_only = new_conversation().await;
    let mut bare = request(&files_only, "r4", "  ");
    bare.attachments = vec!["data/counts.csv".into()];
    server.send(&lab, bare).await.unwrap();
    let bare = finished(files_only.clone()).await;
    assert_eq!(bare.items[0].text, "Uploaded files: data/counts.csv");
    assert_eq!(
        server
            .conversations(&lab, "", false)
            .await
            .unwrap()
            .rows
            .iter()
            .find(|row| row.id == files_only)
            .unwrap()
            .title,
        "data/counts.csv"
    );

    // The remote send command still takes text only.
    let refused = call(
        &server,
        "native_conversation_send",
        &lab,
        json!({"session_id": session, "request_id": "r5", "message": "x",
               "attachments": ["data/counts.csv"]}),
    )
    .await
    .unwrap_err();
    assert!(refused.contains("text messages"), "{refused}");
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_in_plan_mode_is_told_so_and_cannot_change_anything() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::default(), true))
        .await
        .unwrap();
    let session = create(&server).await;
    let instructions = || async {
        server
            .0
            .store
            .load_system_messages(std::slice::from_ref(&session))
            .await
            .unwrap()
            .remove(&session)
            .unwrap()
    };
    let plan = |plan_first| desktop::ConversationOptions {
        plan_first,
        ..Default::default()
    };
    assert_eq!(snapshot(&server, &session).await.plan_mode, Some(false));
    assert!(
        server
            .set_conversation_options(PROJECT, &session, plan(true))
            .await
            .unwrap()
            .plan_first
    );
    assert_eq!(snapshot(&server, &session).await.plan_mode, Some(true));

    // The model tries to write. Plan mode refuses before any question is
    // asked, and nothing is written.
    send(&server, &session, "write the answer").await.unwrap();
    let done = wait_for(&server, &session, |s| !s.running).await;
    assert!(done.approvals.is_empty() && done.error.is_none());
    assert!(!cfg.workspace.join("answer.txt").exists());
    let refused = done
        .items
        .iter()
        .find(|item| item.tool_name.as_deref() == Some("write"))
        .unwrap();
    assert_eq!(refused.ok, Some(false));
    assert!(refused.text.contains("plan"), "{}", refused.text);
    assert!(instructions().await.contains("<plan_mode>"));

    // Leaving plan mode takes the section out again from the next turn, and
    // the switch goes away with the conversation.
    server
        .set_conversation_options(PROJECT, &session, plan(false))
        .await
        .unwrap();
    send(&server, &session, "carry on").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(!instructions().await.contains("<plan_mode>"));
    server
        .set_conversation_options(PROJECT, &session, plan(true))
        .await
        .unwrap();
    server.delete_conversation(PROJECT, &session).await.unwrap();
    assert_eq!(
        server
            .0
            .store
            .get_setting(&format!("frame_plan_mode:{session}"))
            .await
            .unwrap(),
        None
    );
    server.shutdown().await;
}

#[tokio::test]
async fn files_a_turn_writes_become_artifacts_of_its_conversation() {
    let temp = tempfile::tempdir().unwrap();
    let cfg = config(temp.path());
    let server = Server::open_with_models(cfg.clone(), models(Arc::default(), true))
        .await
        .unwrap();
    let writer = create(&server).await;
    let idle = create(&server).await;
    assert!(server
        .artifacts(PROJECT, None, "", 10)
        .await
        .unwrap()
        .is_empty());

    // The turn writes answer.txt through the write tool.
    server
        .set_conversation_options(
            PROJECT,
            &writer,
            desktop::ConversationOptions {
                full_access: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    send(&server, &writer, "write the answer").await.unwrap();
    wait_for(&server, &writer, |s| !s.running).await;
    assert!(cfg.workspace.join("answer.txt").exists());

    let all = server.artifacts(PROJECT, None, "", 10).await.unwrap();
    assert_eq!(all.len(), 1, "{all:?}");
    let artifact = &all[0];
    assert_eq!(artifact.name, "answer.txt");
    assert_eq!(artifact.path.as_deref(), Some("answer.txt"));
    assert_eq!(artifact.session_id, writer);
    assert_eq!(artifact.size_bytes, Some("offline result".len() as u64));
    assert!(!artifact.kind.is_empty());

    // Listed for its own conversation only, and found by name.
    assert_eq!(
        server
            .artifacts(PROJECT, Some(&writer), "", 10)
            .await
            .unwrap(),
        all
    );
    assert!(server
        .artifacts(PROJECT, Some(&idle), "", 10)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        server
            .artifacts(PROJECT, None, " ANSWER ", 10)
            .await
            .unwrap(),
        all
    );
    assert!(server
        .artifacts(PROJECT, None, "absent", 10)
        .await
        .unwrap()
        .is_empty());
    assert!(server
        .artifacts(PROJECT, None, "", 0)
        .await
        .unwrap()
        .is_empty());
    assert!(server.artifacts("missing", None, "", 10).await.is_err());

    // A run that does not exist, or is not this project's, is not cancelled.
    assert!(server.cancel_run(PROJECT, "ghost").await.is_err());
    assert!(server.cancel_run("missing", "ghost").await.is_err());

    for command in ["list_artifacts", "cancel_run"] {
        assert!(!server.commands().contains(&command));
        assert!(call(&server, command, PROJECT, json!({})).await.is_err());
    }
    server.shutdown().await;
}
