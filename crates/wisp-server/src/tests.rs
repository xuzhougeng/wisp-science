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

    // A review is made with the conversation's model until another is
    // chosen for reviews. (This model writes no report, so the review
    // fails; which model was asked is what is checked.)
    server.after_turn_settled(PROJECT, &session).await.unwrap();
    assert_eq!(server.review_model().await.unwrap(), "");
    assert!(server.set_review_model("ghost").await.is_err());
    assert!(server.review_conversation(PROJECT, &session).await.is_err());
    assert_eq!(last_used(&used), ("deep-model".into(), "second-key".into()));
    assert_eq!(server.set_review_model(&first.id).await.unwrap(), first.id);
    assert!(server.review_conversation(PROJECT, &session).await.is_err());
    assert_eq!(last_used(&used), ("lab-model-2".into(), "first-key".into()));
    assert_eq!(server.set_review_model("").await.unwrap(), "");
    assert!(server.review_conversation(PROJECT, &session).await.is_err());
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
            crate::runtime::connector_tools(&server.0.store, &[])
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
async fn usage_is_counted_from_each_model_call_and_storage_is_measured() {
    // A model that says what each call cost.
    struct Metered;
    #[async_trait]
    impl Provider for Metered {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "metered-1"
        }
        async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
            Ok(Completion {
                content: "counted".into(),
                finish_reason: Some("stop".into()),
                usage: wisp_llm::Usage {
                    input_tokens: 120,
                    output_tokens: 30,
                    reasoning_tokens: 5,
                    cached_input_tokens: 100,
                },
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            sink.on_text("counted");
            self.complete(messages, tools).await
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let settings = config(temp.path());
    let data = settings.data.clone();
    let factory: ModelFactory = Arc::new(|_| Box::new(Metered));
    let server = Server::open_with_models(settings, factory).await.unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("uploads")).unwrap();
    std::fs::write(folder.join("uploads/reads.fastq"), vec![b'A'; 4_096]).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let gone = temp.path().join("gone");
    std::fs::create_dir(&gone).unwrap();
    let lost = server.register_local_project(&gone).await.unwrap();
    std::fs::remove_dir(&gone).unwrap();

    // Nothing was used yet. The days run up to today.
    let empty = server.usage().await.unwrap();
    assert!(empty.projects.is_empty() && empty.models.is_empty());
    assert!(empty.days.len() > 300);
    assert!(empty.days.iter().all(|(_, tokens)| *tokens == 0));

    let turn = |project: String, session: String, message: &'static str| {
        let server = &server;
        async move {
            call(
                server,
                "native_conversation_send",
                &project,
                json!({"session_id": session, "request_id": message, "message": message}),
            )
            .await
            .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let value = call(
                        server,
                        "native_conversation_snapshot",
                        &project,
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
    let first = server.create_conversation(&lab).await.unwrap();
    let done = turn(lab.clone(), first.clone(), "one").await;
    // The saved usage rows do not show up as anything in the transcript.
    let roles: Vec<_> = done.items.iter().map(|item| item.role.as_str()).collect();
    assert_eq!(roles, ["user", "assistant"]);
    turn(lab.clone(), first.clone(), "two").await;
    let other = server.create_conversation(PROJECT).await.unwrap();
    turn(PROJECT.into(), other, "three").await;

    let used = server.usage().await.unwrap();
    let project = |id: &str| {
        used.projects
            .iter()
            .find(|row| row.project_id == id)
            .unwrap()
            .clone()
    };
    assert_eq!(
        project(&lab),
        desktop::ProjectUsage {
            project_id: lab.clone(),
            name: "lab".into(),
            conversations: 1,
            input: 240,
            output: 60,
            reasoning: 10,
            cached: 200,
        }
    );
    assert_eq!(project(PROJECT).input, 120);
    assert_eq!(used.projects.len(), 2);
    // By model, and on today's row of the calendar: input plus output.
    assert_eq!(used.models, [("metered-1".to_string(), 450)]);
    assert_eq!(used.days.last().unwrap().1, 450);
    assert_eq!(used.days.iter().map(|(_, tokens)| tokens).sum::<u64>(), 450);

    // Storage: the database, no favorites yet, and what is kept in each
    // project's folder.
    let storage = server.storage().await.unwrap();
    // The same folder, whatever links the temp path goes through.
    let same = |shown: &str, wanted: &Path| {
        std::fs::canonicalize(shown).unwrap() == std::fs::canonicalize(wanted).unwrap()
    };
    assert!(same(&storage.data_dir, &data));
    assert!(storage.database_bytes > 0);
    assert_eq!(storage.favorites_bytes, 0);
    let kept = |id: &str| {
        storage
            .projects
            .iter()
            .find(|row| row.project_id == id)
            .unwrap()
            .clone()
    };
    let lab_kept = kept(&lab);
    assert_eq!(lab_kept.conversations, 1);
    assert_eq!(lab_kept.uploads_bytes, 4_096);
    assert!(!lab_kept.missing);
    assert!(same(&lab_kept.workspace_dir, &folder));
    let lost_kept = kept(&lost);
    assert!(lost_kept.missing);
    assert_eq!((lost_kept.wisp_bytes, lost_kept.uploads_bytes), (0, 0));
    server
        .star_text(&lab, &first, "text", None, "kept")
        .await
        .unwrap();
    assert!(server.storage().await.unwrap().favorites_bytes > 0);
    server.shutdown().await;
}

#[tokio::test]
async fn service_keys_live_in_the_secret_store_and_are_never_read_back() {
    let temp = tempfile::tempdir().unwrap();
    let secrets = Arc::new(desktop::MemorySecrets::default());
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), false),
        secrets.clone(),
    )
    .await
    .unwrap();

    // Listed, none saved, and nothing is created just by looking.
    let rows = server.credentials().await.unwrap();
    let ids: Vec<_> = rows.iter().map(|row| row.id).collect();
    assert_eq!(ids, ["ncbi_api_key", "ncbi_email", "openalex_api_key"]);
    assert!(rows.iter().all(|row| !row.saved));
    assert!(server.service_keys().await.is_empty());
    assert_eq!(
        server
            .0
            .store
            .get_setting("desktop.model.key_id")
            .await
            .unwrap(),
        None
    );

    server
        .set_credential("ncbi_api_key", "  abc123  ")
        .await
        .unwrap();
    server
        .set_credential("ncbi_email", "lab@example.org")
        .await
        .unwrap();
    for (id, value) in [
        ("not-a-key", "x".to_string()),
        ("ncbi_api_key", "two words".to_string()),
        ("ncbi_api_key", "x".repeat(513)),
    ] {
        assert!(server.set_credential(id, &value).await.is_err(), "{id}");
    }
    let saved: Vec<_> = server
        .credentials()
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.saved)
        .collect();
    assert_eq!(saved, [true, true, false]);
    // A turn is given them under the names the services use.
    assert_eq!(
        server.service_keys().await,
        [
            ("NCBI_API_KEY".to_string(), "abc123".to_string()),
            ("NCBI_EMAIL".to_string(), "lab@example.org".to_string()),
        ]
    );

    // The value is in the secret store and nowhere in the database.
    let key_id = server
        .0
        .store
        .get_setting("desktop.model.key_id")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        desktop::SecretStore::get(
            secrets.as_ref(),
            &format!("desktop.service.{key_id}.ncbi_api_key")
        )
        .as_deref(),
        Some("abc123")
    );
    server.shutdown().await;
    drop(server);
    for entry in std::fs::read_dir(temp.path().join("data")).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap_or_default();
        assert!(!bytes.windows(6).any(|window| window == b"abc123"));
    }

    // Forgetting one leaves the other.
    let server = Server::open_with(
        config(temp.path()),
        models(Arc::default(), false),
        secrets.clone(),
    )
    .await
    .unwrap();
    server.set_credential("ncbi_api_key", "").await.unwrap();
    assert_eq!(
        server.service_keys().await,
        [("NCBI_EMAIL".to_string(), "lab@example.org".to_string())]
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_users_mcp_server_gives_turns_its_tools_and_each_asks_before_it_runs() {
    use axum::{http::HeaderMap, response::IntoResponse, routing::post, Json, Router};
    use desktop::{McpServerInput, McpTransport};

    // An MCP server on the loopback interface with one tool, `echo`.
    let tokens: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let heard = tokens.clone();
    let app = Router::new().route(
        "/mcp",
        post(move |headers: HeaderMap, Json(body): Json<Value>| {
            let heard = heard.clone();
            async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_owned();
                heard.lock().unwrap().push(auth);
                let Some(id) = body.get("id").cloned() else {
                    return axum::http::StatusCode::ACCEPTED.into_response();
                };
                let result = match body["method"].as_str().unwrap_or_default() {
                    "initialize" => json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "fake", "version": "1"}
                    }),
                    "tools/list" => json!({"tools": [{
                        "name": "echo",
                        "description": "Say it back",
                        "inputSchema": {
                            "type": "object",
                            "properties": {"text": {"type": "string"}}
                        }
                    }]}),
                    "tools/call" => json!({
                        "content": [{
                            "type": "text",
                            "text": format!("echo: {}", body["params"]["arguments"]["text"].as_str().unwrap_or_default())
                        }],
                        "isError": false
                    }),
                    _ => json!({}),
                };
                Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });

    // A model that calls the server's tool once, then says what came back.
    struct Caller;
    #[async_trait]
    impl Provider for Caller {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            let back = messages
                .iter()
                .rev()
                .find(|message| message.role == wisp_llm::Role::Tool)
                .map(|message| message.content.as_text());
            Ok(match back {
                Some(text) => Completion {
                    content: format!("It said: {text}"),
                    finish_reason: Some("stop".into()),
                    ..Default::default()
                },
                None => Completion {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
                        kind: "function".into(),
                        function: wisp_llm::FunctionCall {
                            name: "fake__echo".into(),
                            arguments: json!({"text": "hi"}).to_string(),
                        },
                    }],
                    finish_reason: Some("tool_calls".into()),
                    ..Default::default()
                },
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            _: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.complete(messages, tools).await
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let factory: ModelFactory = Arc::new(|_| Box::new(Caller));
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();

    // What is refused before anything is saved.
    let input = |name: &str, url: &str| McpServerInput {
        id: None,
        name: name.into(),
        transport: McpTransport::Http { url: url.into() },
        token: None,
    };
    assert!(server.save_mcp_server(input(" ", &url)).await.is_err());
    assert!(server
        .save_mcp_server(input("Fake", "fake.example"))
        .await
        .is_err());
    assert!(server.mcp_servers().await.unwrap().is_empty());

    let fake = server
        .save_mcp_server(McpServerInput {
            token: Some("tok-1".into()),
            ..input("Fake", &url)
        })
        .await
        .unwrap();
    assert!(fake.enabled);
    // Two servers may not answer to the same name.
    assert!(server.save_mcp_server(input("fake", &url)).await.is_err());
    let rows = server.mcp_servers().await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].has_token && rows[0].error.is_none());

    // Asked now, it says what it offers, and it was shown the token.
    assert_eq!(
        server.test_mcp_server(&fake.id).await.unwrap(),
        ["fake__echo"]
    );
    assert!(tokens
        .lock()
        .unwrap()
        .iter()
        .all(|auth| auth == "Bearer tok-1"));
    assert!(server.test_mcp_server("missing").await.is_err());

    // In a turn its tool asks first, then runs.
    let session = create(&server).await;
    send(&server, &session, "go").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty()).await;
    // The card marks it as coming from an MCP server.
    assert_eq!(asked.approvals[0].tool, "mcp:fake__echo");
    server
        .answer_approval(
            PROJECT,
            &session,
            &asked.approvals[0].approval_id,
            true,
            desktop::Remember::No,
        )
        .await
        .unwrap();
    let done = wait_for(&server, &session, |s| !s.running).await;
    let said = |snapshot: &dto::Snapshot| {
        snapshot
            .items
            .iter()
            .map(|item| item.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(said(&done).contains("It said: echo: hi"), "{}", said(&done));

    // Switched off, the model has no such tool, and nothing asks.
    server
        .set_mcp_server_enabled(&fake.id, false)
        .await
        .unwrap();
    let without = create(&server).await;
    send(&server, &without, "go").await.unwrap();
    let done = wait_for(&server, &without, |s| !s.running).await;
    assert!(done.approvals.is_empty());
    assert!(!said(&done).contains("echo: hi"), "{}", said(&done));

    // A server that cannot be reached says why, and turns still run.
    let dead = server
        .save_mcp_server(input("Dead", "http://127.0.0.1:1/mcp"))
        .await
        .unwrap();
    assert!(server.test_mcp_server(&dead.id).await.is_err());
    let rows = server.mcp_servers().await.unwrap();
    assert!(rows
        .iter()
        .find(|row| row.server.id == dead.id)
        .unwrap()
        .error
        .is_some());
    let beside = create(&server).await;
    send(&server, &beside, "go").await.unwrap();
    wait_for(&server, &beside, |s| !s.running).await;

    // Renaming keeps the switch as it was; forgetting the token is explicit.
    let renamed = server
        .save_mcp_server(McpServerInput {
            id: Some(fake.id.clone()),
            token: Some(String::new()),
            ..input("Echo box", &url)
        })
        .await
        .unwrap();
    assert!(!renamed.enabled);
    assert!(!server.mcp_servers().await.unwrap()[0].has_token);
    server.set_mcp_server_enabled(&fake.id, true).await.unwrap();
    assert_eq!(
        server.test_mcp_server(&fake.id).await.unwrap(),
        ["echo_box__echo"]
    );

    server.delete_mcp_server(&dead.id).await.unwrap();
    assert!(server.delete_mcp_server(&dead.id).await.is_err());
    assert_eq!(server.mcp_servers().await.unwrap().len(), 1);
    server.shutdown().await;
}

#[tokio::test]
async fn ssh_hosts_are_registered_as_execution_contexts_without_their_keys() {
    use desktop::SshHostRow;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let start = server.environments().await.unwrap();
    assert!(start.hosts.is_empty());
    assert_eq!(start.default, None);

    let gpu = SshHostRow {
        alias: " gpu ".into(),
        host_name: "10.0.0.8".into(),
        user: "lab".into(),
        port: Some(2222),
        identity_file: "~/.ssh/id_lab".into(),
        notes: "four A100".into(),
        ..Default::default()
    };
    server.save_ssh_host(gpu.clone()).await.unwrap();
    for wrong in [
        SshHostRow::default(),
        SshHostRow {
            alias: "bad alias; rm -rf".into(),
            ..Default::default()
        },
        SshHostRow {
            alias: "ok".into(),
            notes: "n".repeat(501),
            ..Default::default()
        },
    ] {
        assert!(server.save_ssh_host(wrong).await.is_err());
    }
    let view = server.environments().await.unwrap();
    assert_eq!(
        view.hosts,
        [SshHostRow {
            alias: "gpu".into(),
            ..gpu.clone()
        }]
    );

    // The host is an execution context a run can name, holding the path of
    // the key and not a key.
    let contexts = server.0.store.list_execution_contexts().await.unwrap();
    let context = contexts.iter().find(|ctx| ctx.id == "ssh:gpu").unwrap();
    assert!(context.config_json.contains("10.0.0.8"));
    assert!(context.config_json.contains("id_lab"));
    assert!(!context.config_json.contains("PRIVATE KEY"));

    // Saving the same alias changes the host; it does not add another.
    server
        .save_ssh_host(SshHostRow {
            alias: "gpu".into(),
            host_name: "gpu.lab.example".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let view = server.environments().await.unwrap();
    assert_eq!(view.hosts.len(), 1);
    assert_eq!(view.hosts[0].host_name, "gpu.lab.example");
    assert_eq!(view.hosts[0].user, "");

    // New work can be sent there by default, and back to this computer.
    server.set_default_environment(Some("gpu")).await.unwrap();
    assert_eq!(
        server.environments().await.unwrap().default.as_deref(),
        Some("gpu")
    );
    assert!(server
        .set_default_environment(Some("nowhere"))
        .await
        .is_err());
    server.set_default_environment(None).await.unwrap();
    assert_eq!(server.environments().await.unwrap().default, None);

    // Removing the host removes its context and a default that named it.
    server.set_default_environment(Some("gpu")).await.unwrap();
    server.remove_ssh_host("gpu").await.unwrap();
    assert!(server.remove_ssh_host("gpu").await.is_err());
    let view = server.environments().await.unwrap();
    assert!(view.hosts.is_empty());
    assert_eq!(view.default, None);
    assert!(server
        .0
        .store
        .get_execution_context("ssh:gpu")
        .await
        .unwrap()
        .is_none());
    // Only hosts that are in the SSH config can be imported from it.
    assert!(server
        .import_ssh_hosts(&["surely-not-a-configured-host-name".into()])
        .await
        .is_err());
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

#[tokio::test]
async fn network_routes_reach_the_model_and_mcp_servers_and_mirrors_reach_the_prompt() {
    use axum::{response::IntoResponse, Json, Router};
    use desktop::{McpServerInput, McpTransport};
    use wisp_dto::NetworkSettings;

    // A proxy on the loopback interface that answers MCP itself, whatever
    // host it was asked for.
    let app = Router::new().fallback(|Json(body): Json<Value>| async move {
        let Some(id) = body.get("id").cloned() else {
            return axum::http::StatusCode::ACCEPTED.into_response();
        };
        let result = match body["method"].as_str().unwrap_or_default() {
            "initialize" => json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "fake", "version": "1"}
            }),
            "tools/list" => json!({"tools": [{
                "name": "echo",
                "description": "Say it back",
                "inputSchema": {"type": "object"}
            }]}),
            _ => json!({}),
        };
        Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await });

    // A model that records the route it was built with and what it is shown.
    let routes: Arc<std::sync::Mutex<Vec<Option<String>>>> = Arc::default();
    let shown: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let factory = |routes: &Arc<std::sync::Mutex<Vec<Option<String>>>>,
                   shown: &Arc<std::sync::Mutex<Vec<String>>>|
     -> ModelFactory {
        let (routes, shown) = (routes.clone(), shown.clone());
        Arc::new(move |cfg| {
            routes.lock().unwrap().push(cfg.proxy.clone());
            Box::new(Everything(shown.clone()))
        })
    };
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), factory(&routes, &shown))
        .await
        .unwrap();
    assert_eq!(server.network().await.unwrap(), NetworkSettings::default());

    // A wrong field is named and nothing is saved, the good fields included.
    let refused = server
        .set_network(NetworkSettings {
            model_proxy_url: "http://127.0.0.1:7890".into(),
            command_proxy_url: "http://user:secret@proxy.test:1".into(),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(refused.to_string().starts_with("Code proxy"), "{refused}");
    assert_eq!(server.network().await.unwrap(), NetworkSettings::default());

    // An MCP server whose name does not resolve: it answers only when the
    // request goes through the proxy.
    let fake = server
        .save_mcp_server(McpServerInput {
            id: None,
            name: "Fake".into(),
            transport: McpTransport::Http {
                url: "http://mcp.invalid/mcp".into(),
            },
            token: None,
        })
        .await
        .unwrap();
    let saved = server
        .set_network(NetworkSettings {
            model_proxy_url: " http://127.0.0.1:7890 ".into(),
            mcp_proxy_url: proxy.clone(),
            pip_index_url: "https://pypi.mirror.test/simple".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(saved.model_proxy_url, "http://127.0.0.1:7890");
    assert_eq!(server.network().await.unwrap(), saved);
    assert_eq!(
        server.test_mcp_server(&fake.id).await.unwrap(),
        ["fake__echo"]
    );

    // The next turn's model is built with the route, and is told where
    // packages come from.
    let last_route = || routes.lock().unwrap().last().cloned().flatten();
    let prompt = || shown.lock().unwrap().join("\n");
    let session = create(&server).await;
    send(&server, &session, "go").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(last_route().as_deref(), Some("http://127.0.0.1:7890"));
    assert!(prompt().contains("<wisp-package-mirrors>"));
    assert!(prompt().contains("\"https://pypi.mirror.test/simple\""));

    // `none` is passed on as it is; with the mirror gone the same
    // conversation is no longer told about it.
    let direct = server
        .set_network(NetworkSettings {
            model_proxy_url: "none".into(),
            mcp_proxy_url: proxy.clone(),
            ..Default::default()
        })
        .await
        .unwrap();
    shown.lock().unwrap().clear();
    send(&server, &session, "again").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert_eq!(last_route().as_deref(), Some("none"));
    assert!(!prompt().contains("<wisp-package-mirrors>"), "{}", prompt());

    // The routes are kept and apply again when the host next opens.
    server.shutdown().await;
    drop(server);
    let server = Server::open_with_models(config(temp.path()), factory(&routes, &shown))
        .await
        .unwrap();
    assert_eq!(server.network().await.unwrap(), direct);
    assert_eq!(
        server.test_mcp_server(&fake.id).await.unwrap(),
        ["fake__echo"]
    );
    server.shutdown().await;
}

#[tokio::test]
async fn checking_an_ssh_host_records_what_it_is_and_tells_the_model() {
    use desktop::SshHostRow;
    use wisp_runs::context_probe::{ProbeCommand, ProbeCommandOutput, ProbeRunner};

    // Stands in for OpenSSH: answers the probe's one command with what the
    // host "is", or fails the way a refused login does.
    struct Host {
        answers: Result<Vec<(&'static str, &'static str)>, &'static str>,
        asked: Vec<ProbeCommand>,
    }
    impl ProbeRunner for Host {
        fn run(&mut self, command: &ProbeCommand) -> Result<ProbeCommandOutput, String> {
            self.asked.push(command.clone());
            Ok(match &self.answers {
                Ok(values) => ProbeCommandOutput {
                    status: 0,
                    stdout: values
                        .iter()
                        .map(|(key, value)| {
                            format!(
                                "__WISP_PROBE_VALUE_BEGIN__:{key}\n{value}\n\
                                 __WISP_PROBE_VALUE_END__:{key}\n"
                            )
                        })
                        .collect::<String>()
                        .into_bytes(),
                    stderr: String::new(),
                },
                Err(why) => ProbeCommandOutput {
                    status: 255,
                    stdout: vec![],
                    stderr: (*why).into(),
                },
            })
        }
    }

    let shown: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let seen = shown.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(seen.clone())));
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    server
        .save_ssh_host(SshHostRow {
            alias: "lab-probe".into(),
            host_name: "10.0.0.9".into(),
            user: "lab".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Never checked: there is nothing to show, and no host of that name
    // is checked by mistake.
    assert!(server.environments().await.unwrap().checks.is_empty());
    let mut nobody = Host {
        answers: Ok(vec![]),
        asked: vec![],
    };
    assert!(server
        .check_environment_with("missing", &mut nobody)
        .await
        .is_err());
    assert!(nobody.asked.is_empty());

    // A refused login is recorded, not raised, and says what to look at.
    let mut refusing = Host {
        answers: Err("lab@10.0.0.9: Permission denied (publickey)."),
        asked: vec![],
    };
    let failed = server
        .check_environment_with("lab-probe", &mut refusing)
        .await
        .unwrap();
    assert!(failed.found.is_none());
    let why = failed.error.clone().unwrap();
    assert!(why.contains("key authentication failed"), "{why}");
    // It asked the saved host, as the saved user, without a prompt.
    assert_eq!(refusing.asked.len(), 1);
    assert_eq!(refusing.asked[0].program, "ssh");
    let args = refusing.asked[0].args.join(" ");
    assert!(
        args.contains("BatchMode=yes") && args.contains("lab@10.0.0.9"),
        "{args}"
    );
    assert_eq!(server.environments().await.unwrap().checks, [failed]);

    // The model is told not to use a host whose check failed.
    let session = create(&server).await;
    send(&server, &session, "go").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let prompt = || shown.lock().unwrap().join("\n");
    assert!(prompt().contains("ssh:lab-probe"));
    assert!(prompt().contains("last probe failed"), "{}", prompt());

    // Checked again, it answers: what it is is kept and shown.
    let mut answering = Host {
        answers: Ok(vec![
            ("os", "Linux"),
            ("arch", "x86_64"),
            ("hostname", "lab-node-7"),
            ("cpu_count", "64"),
            ("gpu_summary", "NVIDIA A100 x4"),
            ("scheduler", "/usr/bin/sbatch"),
            ("python_executable", "/opt/conda/bin/python3"),
            ("python_version", "3.11.9"),
        ]),
        asked: vec![],
    };
    let passed = server
        .check_environment_with("lab-probe", &mut answering)
        .await
        .unwrap();
    assert_eq!(passed.error, None);
    let found = passed.found.clone().unwrap();
    assert_eq!(found.hostname.as_deref(), Some("lab-node-7"));
    assert_eq!(found.cpu_count, Some(64));
    assert_eq!(found.scheduler.as_deref(), Some("slurm"));
    assert_eq!(
        found.python_executable.as_deref(),
        Some("/opt/conda/bin/python3")
    );
    assert_eq!(server.environments().await.unwrap().checks, [passed]);
    shown.lock().unwrap().clear();
    send(&server, &session, "again").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(!prompt().contains("last probe failed"), "{}", prompt());

    // Changing the host's notes does not forget that it was checked.
    server
        .save_ssh_host(SshHostRow {
            alias: "lab-probe".into(),
            host_name: "10.0.0.9".into(),
            user: "lab".into(),
            notes: "four A100".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let after = server.environments().await.unwrap();
    assert_eq!(after.checks.len(), 1);
    assert!(after.checks[0].found.is_some());

    // A saved host whose execution context is missing can still be checked.
    server
        .0
        .store
        .delete_execution_context("ssh:lab-probe")
        .await
        .unwrap();
    assert!(server.environments().await.unwrap().checks.is_empty());
    let again = server
        .check_environment_with("lab-probe", &mut answering)
        .await
        .unwrap();
    assert!(again.found.is_some());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_chooses_where_its_work_runs() {
    use desktop::{ConversationOptions, SshHostRow, RUNS_LOCALLY};

    let shown: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let seen = shown.clone();
    let factory: ModelFactory = Arc::new(move |_| Box::new(Everything(seen.clone())));
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    for alias in ["run-a", "run-b"] {
        server
            .save_ssh_host(SshHostRow {
                alias: alias.into(),
                host_name: "10.0.0.8".into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    server.set_default_environment(Some("run-a")).await.unwrap();

    // What the model is told its work goes to, after one more turn.
    let told = |session: String, message: &'static str| {
        let (server, shown) = (server.clone(), shown.clone());
        async move {
            shown.lock().unwrap().clear();
            send(&server, &session, message).await.unwrap();
            wait_for(&server, &session, |s| !s.running).await;
            let prompt = shown.lock().unwrap().join("\n");
            ["run-a", "run-b"]
                .into_iter()
                .filter(|alias| {
                    prompt.contains(&format!("Default analysis environment: `ssh:{alias}`"))
                })
                .collect::<Vec<_>>()
        }
    };
    let runs_on = |session: String| {
        let server = server.clone();
        async move {
            server
                .conversation_status(PROJECT, &session)
                .await
                .unwrap()
                .options
                .runs_on
        }
    };
    let choose = |session: String, runs_on: &'static str| {
        let server = server.clone();
        async move {
            let options = server
                .conversation_status(PROJECT, &session)
                .await
                .unwrap()
                .options;
            server
                .set_conversation_options(
                    PROJECT,
                    &session,
                    ConversationOptions {
                        runs_on: runs_on.into(),
                        ..options
                    },
                )
                .await
        }
    };

    // With no choice of its own a conversation follows Settings.
    let session = create(&server).await;
    assert_eq!(runs_on(session.clone()).await, "");
    assert_eq!(told(session.clone(), "go").await, ["run-a"]);

    // Its own choice applies from its next turn, and only to it.
    let saved = choose(session.clone(), "run-b").await.unwrap();
    assert_eq!(saved.runs_on, "run-b");
    assert_eq!(told(session.clone(), "again").await, ["run-b"]);
    let other = create(&server).await;
    assert_eq!(told(other.clone(), "go").await, ["run-a"]);

    // This computer is a choice too: no host is named as where work goes.
    choose(session.clone(), RUNS_LOCALLY).await.unwrap();
    assert!(told(session.clone(), "once more").await.is_empty());

    // A host that is not saved is refused and nothing changes.
    assert!(choose(session.clone(), "nowhere").await.is_err());
    assert_eq!(runs_on(session.clone()).await, RUNS_LOCALLY);

    // Changing another option keeps the choice, and a branch carries it.
    let options = server
        .conversation_status(PROJECT, &session)
        .await
        .unwrap()
        .options;
    server
        .set_conversation_options(
            PROJECT,
            &session,
            ConversationOptions {
                plan_first: true,
                ..options
            },
        )
        .await
        .unwrap();
    assert_eq!(runs_on(session.clone()).await, RUNS_LOCALLY);
    let branch = server
        .branch_conversation(PROJECT, &session, None)
        .await
        .unwrap();
    assert_eq!(runs_on(branch).await, RUNS_LOCALLY);
    // So does a copy in another project: hosts belong to the app.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let copy = server
        .transfer_conversation(PROJECT, &session, &lab, true)
        .await
        .unwrap();
    let copied = server.conversation_status(&lab, &copy).await.unwrap();
    assert_eq!(copied.options.runs_on, RUNS_LOCALLY);

    // Back to following Settings; and a host that is removed is no longer
    // any conversation's choice.
    choose(session.clone(), "").await.unwrap();
    assert_eq!(runs_on(session.clone()).await, "");
    choose(other.clone(), "run-b").await.unwrap();
    server.remove_ssh_host("run-b").await.unwrap();
    assert_eq!(runs_on(other.clone()).await, "");
    assert_eq!(told(other, "after").await, ["run-a"]);
    server.shutdown().await;
}

#[tokio::test]
async fn project_settings_name_the_project_and_give_the_model_its_rules() {
    use desktop::{ProjectSettings, PROJECT_NAME_LIMIT, PROJECT_RULES_LIMIT};

    let shown: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
    let models = |shown: &Arc<std::sync::Mutex<Vec<String>>>| -> ModelFactory {
        let shown = shown.clone();
        Arc::new(move |_| Box::new(Everything(shown.clone())))
    };
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(&shown))
        .await
        .unwrap();
    let rules_file = temp.path().join("workspace/.wisp/WISP.md");
    let start = server.project_settings(PROJECT).await.unwrap();
    assert_eq!((start.name.as_str(), start.rules.as_str()), ("Server", ""));
    assert!(server.project_settings("missing").await.is_err());

    // What the model is shown after one more turn of `session`.
    let told = |session: String, message: &'static str| {
        let (server, shown) = (server.clone(), shown.clone());
        async move {
            shown.lock().unwrap().clear();
            send(&server, &session, message).await.unwrap();
            wait_for(&server, &session, |s| !s.running).await;
            shown.lock().unwrap().join("\n")
        }
    };
    // A conversation that starts before the project has rules.
    let session = create(&server).await;
    assert!(told(session.clone(), "go")
        .await
        .contains("No user-defined rules."));

    // What cannot be saved changes nothing, the rules file included.
    let with = |change: fn(&mut ProjectSettings)| {
        let mut settings = ProjectSettings {
            rules: "Never saved.".into(),
            ..start.clone()
        };
        change(&mut settings);
        settings
    };
    for refused in [
        with(|s| s.name = "  ".into()),
        with(|s| s.name = "two\nlines".into()),
        with(|s| s.name = "n".repeat(PROJECT_NAME_LIMIT + 1)),
        with(|s| s.rules = "r".repeat(PROJECT_RULES_LIMIT + 1)),
    ] {
        assert!(server.set_project_settings(PROJECT, refused).await.is_err());
    }
    assert_eq!(server.project_settings(PROJECT).await.unwrap(), start);
    assert!(!rules_file.exists());

    // Saved, they are trimmed; the rules are a file in the project.
    let saved = server
        .set_project_settings(
            PROJECT,
            ProjectSettings {
                name: " Screens 2026 ".into(),
                description: " CRISPR screens ".into(),
                rules: "Always report effect sizes.\n".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(saved.name, "Screens 2026");
    assert_eq!(saved.description, "CRISPR screens");
    assert_eq!(
        std::fs::read_to_string(&rules_file).unwrap(),
        "Always report effect sizes."
    );

    // The conversation that was already open is told from its next turn.
    let prompt = told(session.clone(), "again").await;
    assert!(prompt.contains("Always report effect sizes."), "{prompt}");
    assert!(!prompt.contains("No user-defined rules."));
    // The file is the truth: an edit made outside the app is read as well.
    std::fs::write(&rules_file, "Use SI units.").unwrap();
    assert_eq!(
        server.project_settings(PROJECT).await.unwrap().rules,
        "Use SI units."
    );
    let prompt = told(session.clone(), "third").await;
    assert!(prompt.contains("Use SI units.") && !prompt.contains("effect sizes"));

    // Emptying the rules removes the file, and the model is told there are none.
    server
        .set_project_settings(
            PROJECT,
            ProjectSettings {
                rules: "  ".into(),
                ..saved.clone()
            },
        )
        .await
        .unwrap();
    assert!(!rules_file.exists());
    assert!(told(session.clone(), "fourth")
        .await
        .contains("No user-defined rules."));

    // The name is the user's: opening the host again does not put it back.
    server.shutdown().await;
    drop(server);
    let server = Server::open_with_models(config(temp.path()), models(&shown))
        .await
        .unwrap();
    let after = server.project_settings(PROJECT).await.unwrap();
    assert_eq!(
        (after.name.as_str(), after.description.as_str()),
        ("Screens 2026", "CRISPR screens")
    );
    server.shutdown().await;
}

#[tokio::test]
async fn the_desktop_makes_renames_and_trashes_project_files() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let names = |path: &'static str| {
        let server = server.clone();
        async move {
            let mut names: Vec<String> = server
                .project_directory(PROJECT, path)
                .await
                .unwrap()
                .entries
                .into_iter()
                .map(|entry| entry.name)
                .collect();
            names.sort();
            names
        }
    };
    assert_eq!(
        server
            .create_project_entry(PROJECT, "", "figures", true)
            .await
            .unwrap(),
        "figures"
    );
    let note = server
        .create_project_entry(PROJECT, "figures", "notes.md", false)
        .await
        .unwrap();
    assert_eq!(note, "figures/notes.md");
    assert!(server
        .create_project_entry("missing", "", "x", true)
        .await
        .is_err());
    let plan = server
        .rename_project_entry(PROJECT, &note, "plan.md")
        .await
        .unwrap();
    assert_eq!(names("figures").await, ["plan.md"]);

    // Trashed, it is gone from the listing and kept on disk; the trash is
    // part of the project's own records, which the browser does not show.
    let gone = server.trash_project_entry(PROJECT, &plan).await.unwrap();
    assert!(gone.starts_with(".wisp/trash/") && gone.ends_with("/plan.md"));
    assert!(names("figures").await.is_empty());
    assert!(temp.path().join("workspace").join(&gone).is_file());
    assert_eq!(names("").await, ["figures"]);

    // A picture is handed over whole, with what kind it is; other files,
    // and anything outside the project, are not.
    let workspace = temp.path().join("workspace");
    std::fs::write(workspace.join("figures/plot.PNG"), b"\x89PNG fake").unwrap();
    std::fs::write(workspace.join("figures/notes.txt"), "words").unwrap();
    let (kind, bytes) = server
        .project_image(PROJECT, "figures/plot.PNG")
        .await
        .unwrap();
    assert_eq!(
        (kind.as_str(), bytes.as_slice()),
        ("image/png", &b"\x89PNG fake"[..])
    );
    for path in [
        "figures/notes.txt",
        "figures",
        "figures/missing.png",
        "../x.png",
    ] {
        assert!(server.project_image(PROJECT, path).await.is_err(), "{path}");
    }
    server.shutdown().await;
}

#[tokio::test]
async fn a_projects_journey_lists_what_happened_and_what_the_researcher_recorded() {
    use wisp_dto::ResearchJournalInput;
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let (from, until) = (now - 86_400, now + 86_400);
    assert!(server
        .journey(PROJECT, from, until)
        .await
        .unwrap()
        .entries
        .is_empty());

    // What was asked in a conversation is part of the history.
    let session = create(&server).await;
    send(&server, &session, "align the reads").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    // So is what the researcher writes down, dated when it happened.
    let record = |title: &str, category: &str, occurred_at: i64| ResearchJournalInput {
        title: title.into(),
        body: "Mapping rate 98%.".into(),
        category: category.into(),
        occurred_at,
    };
    let id = server
        .add_journal_entry(PROJECT, record(" Reads are clean ", "finding", now - 3_600))
        .await
        .unwrap();
    for refused in [
        record("", "finding", now),
        record("Title", "gossip", now),
        record("Title", "finding", -5),
    ] {
        assert!(server.add_journal_entry(PROJECT, refused).await.is_err());
    }
    assert!(server
        .add_journal_entry("missing", record("Title", "finding", now))
        .await
        .is_err());

    let journey = server.journey(PROJECT, from, until).await.unwrap();
    let kinds: Vec<_> = journey
        .entries
        .iter()
        .map(|entry| (entry.kind.as_str(), entry.title.as_str(), entry.manual))
        .collect();
    // Newest first: the question just asked, then the record dated an hour ago.
    assert_eq!(
        kinds,
        [
            ("session", "align the reads", false),
            ("finding", "Reads are clean", true)
        ]
    );
    assert_eq!(journey.entries[1].source_id, id);
    assert_eq!(journey.entries[1].summary, "Mapping rate 98%.");
    assert_eq!(
        journey.entries[0].frame_id.as_deref(),
        Some(session.as_str())
    );

    // Only what falls in the range, and never more than a month at a time.
    let earlier = server
        .journey(PROJECT, now - 10 * 86_400, now - 86_400)
        .await
        .unwrap();
    assert!(earlier.entries.is_empty());
    assert!(server
        .journey(PROJECT, now - 40 * 86_400, now)
        .await
        .is_err());
    assert!(server.journey("missing", from, until).await.is_err());

    // The calendar is the same history for every project at once.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    server
        .add_journal_entry(&lab, record("Ordered primers", "progress", now))
        .await
        .unwrap();
    let calendar = server.calendar(from, until).await.unwrap();
    let mut seen: Vec<_> = calendar
        .iter()
        .map(|project| {
            assert_eq!(project.error, None);
            (project.project_id.as_str(), project.history.entries.len())
        })
        .collect();
    seen.sort();
    let mut expected = [(PROJECT, 2), (lab.as_str(), 1)];
    expected.sort();
    assert_eq!(seen, expected);
    assert!(server.calendar(now - 40 * 86_400, now).await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_is_read_back_as_a_trajectory() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    // A model that runs one tool, then answers.
    let server = Server::open_with_models(config(temp.path()), models(calls, true))
        .await
        .unwrap();
    let session = create(&server).await;
    // Nothing asked yet: nothing to show, and nothing made up.
    let empty = server.trajectory(PROJECT, &session).await.unwrap();
    assert!(empty.turns.is_empty());
    assert_eq!(empty.stats, wisp_dto::TrajectoryStatsDto::default());

    send(&server, &session, "write the file").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty() || !s.running).await;
    for approval in &asked.approvals {
        server
            .answer_approval(
                PROJECT,
                &session,
                &approval.approval_id,
                true,
                desktop::Remember::No,
            )
            .await
            .unwrap();
    }
    wait_for(&server, &session, |s| !s.running).await;

    let trajectory = server.trajectory(PROJECT, &session).await.unwrap();
    assert_eq!(trajectory.frame_id, session);
    assert_eq!(trajectory.turns.len(), 1);
    let turn = &trajectory.turns[0];
    assert_eq!(turn.index, 1);
    assert!(turn.started_at.is_some());
    let cells = |kind: &str| -> Vec<_> { turn.cells.iter().filter(|c| c.kind == kind).collect() };
    assert_eq!(turn.cells[0].kind, "user");
    assert_eq!(turn.cells[0].summary, "write the file");
    // The tool is one step: what it was given, what it answered, how it went.
    let tools = cells("tool");
    assert_eq!(tools.len(), 1, "{turn:?}");
    assert!(tools[0].summary.starts_with("write {"), "{turn:?}");
    assert!(tools[0]
        .detail_input
        .as_deref()
        .unwrap()
        .contains("answer.txt"));
    assert!(tools[0].detail_output.is_some());
    assert_eq!((tools[0].ok, tools[0].is_error), (Some(true), false));
    assert_eq!(cells("assistant").len(), 1, "{turn:?}");
    // Two model calls: the one that asked for the tool and the one that answered.
    assert_eq!(cells("usage").len(), 2, "{turn:?}");
    assert_eq!((trajectory.stats.turns, trajectory.stats.steps), (1, 2));

    // Only the conversation's own project may read it.
    assert!(server.trajectory("missing", &session).await.is_err());
    assert!(server.trajectory(PROJECT, "missing").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn an_answer_is_followed_by_questions_to_ask_next() {
    // A model that answers a turn, and suggests when asked on the side.
    struct Advising {
        suggested: Arc<AtomicUsize>,
        hold: Arc<AtomicBool>,
    }
    #[async_trait]
    impl Provider for Advising {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(&self, _: &[Message], _: &[ToolSchema]) -> wisp_llm::Result<Completion> {
            self.suggested.fetch_add(1, Ordering::Relaxed);
            Ok(Completion {
                content: r#"["Why?", "How many?", "What next?"]"#.into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            _: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            while self.hold.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            sink.on_text("The answer.");
            Ok(Completion {
                content: "The answer.".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let suggested = Arc::new(AtomicUsize::new(0));
    let hold = Arc::new(AtomicBool::new(false));
    let factory: ModelFactory = {
        let (suggested, hold) = (suggested.clone(), hold.clone());
        Arc::new(move |_| {
            Box::new(Advising {
                suggested: suggested.clone(),
                hold: hold.clone(),
            })
        })
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // On until it is turned off.
    let defaults = server.turn_defaults().await.unwrap();
    assert!(defaults.follow_ups);

    let session = create(&server).await;
    assert!(snapshot(&server, &session).await.follow_ups.is_empty());
    // Nothing to wait for before anything was asked.
    server.after_turn_settled(PROJECT, &session).await.unwrap();
    assert!(server
        .after_turn_settled("missing", &session)
        .await
        .is_err());
    send(&server, &session, "one").await.unwrap();
    // Whoever sees the turn over can wait for the questions.
    wait_for(&server, &session, |s| !s.running).await;
    server.after_turn_settled(PROJECT, &session).await.unwrap();
    let after = snapshot(&server, &session).await;
    assert!(!after.running);
    assert_eq!(after.follow_ups, ["Why?", "How many?", "What next?"]);
    assert_eq!(suggested.load(Ordering::Relaxed), 1);

    // They are about that answer: the next turn starts without them.
    hold.store(true, Ordering::Relaxed);
    send(&server, &session, "two").await.unwrap();
    let during = wait_for(&server, &session, |s| s.running).await;
    assert!(during.follow_ups.is_empty());
    hold.store(false, Ordering::Relaxed);
    wait_for(&server, &session, |s| !s.follow_ups.is_empty()).await;

    // Put away, they stay away; only the conversation's project may.
    assert!(server
        .dismiss_follow_ups("missing", &session)
        .await
        .is_err());
    server.dismiss_follow_ups(PROJECT, &session).await.unwrap();
    assert!(snapshot(&server, &session).await.follow_ups.is_empty());

    // Turned off, the model is not asked at all.
    let off = server
        .set_turn_defaults(desktop::TurnDefaults {
            follow_ups: false,
            ..defaults
        })
        .await
        .unwrap();
    assert!(!off.follow_ups);
    // The key and value are the WebView desktop's.
    assert_eq!(
        server
            .0
            .store
            .get_setting("follow_up_questions")
            .await
            .unwrap()
            .as_deref(),
        Some("false")
    );
    let before = suggested.load(Ordering::Relaxed);
    send(&server, &session, "three").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    server.after_turn_settled(PROJECT, &session).await.unwrap();
    assert!(snapshot(&server, &session).await.follow_ups.is_empty());
    assert_eq!(suggested.load(Ordering::Relaxed), before);
    server.shutdown().await;
}

#[tokio::test]
async fn a_question_asked_aside_is_answered_without_touching_the_conversation() {
    // A model that answers a turn, and records what it is asked aside.
    struct Recording {
        asked: Arc<Mutex<Vec<Vec<Message>>>>,
    }
    #[async_trait]
    impl Provider for Recording {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            assert!(tools.is_empty(), "a side question has no tools");
            self.asked.lock().unwrap().push(messages.to_vec());
            Ok(Completion {
                content: " Because the file has 400 lines. ".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            _: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            sink.on_text("There are 100 reads.");
            Ok(Completion {
                content: "There are 100 reads.".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let factory: ModelFactory = {
        let asked = asked.clone();
        Arc::new(move |_| {
            Box::new(Recording {
                asked: asked.clone(),
            })
        })
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // Asking aside is all this test's model is asked on the side.
    let defaults = server.turn_defaults().await.unwrap();
    server
        .set_turn_defaults(desktop::TurnDefaults {
            follow_ups: false,
            ..defaults
        })
        .await
        .unwrap();
    let session = create(&server).await;
    send(&server, &session, "count the reads").await.unwrap();
    let before = wait_for(&server, &session, |s| !s.running).await;

    let earlier = [desktop::AsideExchange {
        question: "which file?".into(),
        answer: "reads.fq".into(),
    }];
    let answer = server
        .ask_aside(PROJECT, &session, &earlier, "  why 100?  ")
        .await
        .unwrap();
    assert_eq!(answer, "Because the file has 400 lines.");
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        let sent: Vec<_> = asked[0]
            .iter()
            .map(|message| message.content.as_text())
            .collect();
        // The conversation as quoted data, the earlier exchange, the question.
        assert!(sent[1].contains("[USER]\ncount the reads"), "{sent:?}");
        assert!(
            sent[1].contains("[ASSISTANT]\nThere are 100 reads."),
            "{sent:?}"
        );
        assert_eq!(sent[3..], ["which file?", "reads.fq", "why 100?"]);
    }
    // Nothing was added to the conversation.
    let after = snapshot(&server, &session).await;
    assert_eq!(after.items.len(), before.items.len());
    assert!(!after.running);

    // A question is needed, of a sensible length, in the conversation's own project.
    for (project, question) in [
        (PROJECT, "   ".to_owned()),
        (PROJECT, "x".repeat(4_001)),
        ("missing", "why?".to_owned()),
    ] {
        assert!(server
            .ask_aside(project, &session, &[], &question)
            .await
            .is_err());
    }
    assert!(server
        .ask_aside(PROJECT, "missing", &[], "why?")
        .await
        .is_err());
    assert_eq!(asked.lock().unwrap().len(), 1);
    // It is the desktop's own: a paired device cannot ask it.
    assert!(!server.commands().iter().any(|name| name.contains("aside")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_project_and_its_folder_are_deleted_only_when_its_name_is_typed() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let folder = temp.path().join("lab");
    std::fs::create_dir_all(folder.join("results")).unwrap();
    std::fs::write(folder.join("results/counts.csv"), "gene,count\n").unwrap();
    let neighbour = temp.path().join("neighbour");
    std::fs::create_dir(&neighbour).unwrap();
    std::fs::write(neighbour.join("keep.txt"), "mine").unwrap();
    let lab = server.register_local_project(&folder).await.unwrap();
    let kept = server.register_local_project(&neighbour).await.unwrap();
    let session = server.create_conversation(&lab).await.unwrap();
    let listed = |id: String| {
        let server = &server;
        async move {
            let value = call(server, "list_projects", "", json!({})).await.unwrap();
            serde_json::from_value::<Vec<wisp_dto::ProjectSummary>>(value)
                .unwrap()
                .iter()
                .any(|project| project.id == id)
        }
    };

    // The question says what would go.
    let asked = server.project_folder(&lab).await.unwrap();
    assert_eq!(asked.name, "lab");
    assert!(asked.path.ends_with("lab"), "{asked:?}");
    assert_eq!(
        asked.contents,
        desktop::FolderContents::Files {
            files: 1,
            bytes: 11,
            more: false
        }
    );

    // Without the name, with another project's name, or for a project
    // that cannot leave, nothing is touched.
    for (project, typed) in [
        (lab.as_str(), ""),
        (lab.as_str(), "neighbour"),
        (lab.as_str(), "LAB"),
        (PROJECT, "Server"),
        ("missing", "lab"),
    ] {
        assert!(server
            .delete_project_and_folder(project, typed)
            .await
            .is_err());
    }
    assert!(server.project_folder(PROJECT).await.is_err());
    assert!(folder.join("results/counts.csv").exists());
    assert!(listed(lab.clone()).await);

    server
        .delete_project_and_folder(&lab, " lab ")
        .await
        .unwrap();
    assert!(!folder.exists());
    assert!(!listed(lab.clone()).await);
    // Its conversation went with it; the neighbour and its files did not.
    assert!(server.conversation_status(&lab, &session).await.is_err());
    assert!(listed(kept.clone()).await);
    assert_eq!(
        std::fs::read_to_string(neighbour.join("keep.txt")).unwrap(),
        "mine"
    );

    // A folder that is already gone leaves only records to remove.
    std::fs::remove_dir_all(&neighbour).unwrap();
    let asked = server.project_folder(&kept).await.unwrap();
    assert_eq!(asked.contents, desktop::FolderContents::Missing);
    server
        .delete_project_and_folder(&kept, "neighbour")
        .await
        .unwrap();
    assert!(!listed(kept).await);
    // It is the desktop's own: a paired device cannot ask for it.
    assert!(!server
        .commands()
        .iter()
        .any(|name| name.contains("delete_project")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_review_reports_what_the_tool_output_does_not_support() {
    // A model that runs a tool and answers, and reviews when asked to.
    struct Reviewing {
        read: Arc<Mutex<Vec<String>>>,
    }
    #[async_trait]
    impl Provider for Reviewing {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "reviewer-1"
        }
        async fn complete(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            assert!(tools.is_empty(), "a reviewer has no tools");
            assert!(messages[0]
                .content
                .as_text()
                .starts_with("You are a REVIEWER"));
            self.read
                .lock()
                .unwrap()
                .push(messages[1].content.as_text());
            Ok(Completion {
                content: r#"{"summary":"Checked the write.","findings":[{"message_index":3,
                    "claim":"Saved result","evidence":"the tool wrote answer.txt",
                    "fix":"Name the file","verdict":"warn","severity":"low"}]}"#
                    .into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            if !messages
                .iter()
                .any(|message| message.role == wisp_llm::Role::Tool)
            {
                return Ok(Completion {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
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
            Ok(Completion {
                content: "Saved result".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let read = Arc::new(Mutex::new(Vec::new()));
    let factory: ModelFactory = {
        let read = read.clone();
        Arc::new(move |_| Box::new(Reviewing { read: read.clone() }))
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    let defaults = server.turn_defaults().await.unwrap();
    server
        .set_turn_defaults(desktop::TurnDefaults {
            follow_ups: false,
            ..defaults
        })
        .await
        .unwrap();
    let session = create(&server).await;
    // Before anything was said there is nothing to review, and the model
    // is not asked.
    let empty = server
        .review_conversation(PROJECT, &session)
        .await
        .unwrap_err()
        .to_string();
    assert!(empty.contains("nothing to review"), "{empty}");
    assert!(read.lock().unwrap().is_empty());

    send(&server, &session, "write the file").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty() || !s.running).await;
    // Half a turn cannot be judged.
    if asked.running {
        let early = server
            .review_conversation(PROJECT, &session)
            .await
            .unwrap_err()
            .to_string();
        assert!(early.contains("Wait for the turn"), "{early}");
    }
    for approval in &asked.approvals {
        server
            .answer_approval(
                PROJECT,
                &session,
                &approval.approval_id,
                true,
                desktop::Remember::No,
            )
            .await
            .unwrap();
    }
    let before = wait_for(&server, &session, |s| !s.running).await;

    let review = server.review_conversation(PROJECT, &session).await.unwrap();
    assert_eq!(review.summary, "Checked the write.");
    assert_eq!(review.model, "reviewer-1");
    assert_eq!(review.findings.len(), 1);
    assert_eq!(review.findings[0].claim, "Saved result");
    assert_eq!(review.findings[0].verdict, "warn");
    assert!(review.correction().contains("1. Claim: Saved result"));
    // The reviewer read the question, what the tool was given and what it
    // answered, and the reply.
    let transcript = read.lock().unwrap()[0].clone();
    assert!(
        transcript.starts_with("[msg:0 USER]\nwrite the file"),
        "{transcript}"
    );
    assert!(transcript.contains("TOOL:write]\nargs:\n"), "{transcript}");
    assert!(transcript.contains("answer.txt"), "{transcript}");
    assert!(
        transcript.contains("ASSISTANT]\nSaved result"),
        "{transcript}"
    );
    // Nothing was added to the conversation.
    let after = snapshot(&server, &session).await;
    assert_eq!(after.items.len(), before.items.len());

    // One turn reviewed by itself: the reviewer reads up to its end and
    // nothing that came after.
    send(&server, &session, "and thanks").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let first = desktop::TurnRef {
        from_end: 1,
        text: "write the file".into(),
    };
    server.review_turn(PROJECT, &session, &first).await.unwrap();
    let earlier = read.lock().unwrap().last().unwrap().clone();
    assert!(earlier.contains("write the file"), "{earlier}");
    assert!(!earlier.contains("and thanks"), "{earlier}");
    server.review_conversation(PROJECT, &session).await.unwrap();
    assert!(read.lock().unwrap().last().unwrap().contains("and thanks"));
    // A turn that is not there is refused, and nobody is asked.
    let asked = read.lock().unwrap().len();
    let wrong = desktop::TurnRef {
        from_end: 1,
        text: "something else".into(),
    };
    assert!(server.review_turn(PROJECT, &session, &wrong).await.is_err());
    assert!(server
        .review_turn("missing", &session, &first)
        .await
        .is_err());
    assert_eq!(read.lock().unwrap().len(), asked);

    assert!(server
        .review_conversation("missing", &session)
        .await
        .is_err());
    assert!(server
        .review_conversation(PROJECT, "missing")
        .await
        .is_err());
    assert!(!server.commands().iter().any(|name| name.contains("review")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversations_interpreters_are_listed_and_can_be_stopped() {
    // An interpreter that starts at once and does nothing.
    struct Idle;
    #[async_trait]
    impl wisp_runtime::RuntimeKernel for Idle {
        async fn execute(
            &mut self,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: &[String],
            _: &wisp_runtime::RuntimeOutput,
        ) -> anyhow::Result<wisp_runtime::KernelResp> {
            Ok(Default::default())
        }
        async fn inspect(&mut self, _: &str) -> anyhow::Result<wisp_runtime::RuntimeObjectList> {
            Ok(Default::default())
        }
        async fn shutdown(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
    }
    struct Launcher;
    #[async_trait]
    impl wisp_runtime::RuntimeLauncher for Launcher {
        async fn launch(
            &self,
            key: &wisp_runtime::RuntimeKey,
            _: &Path,
        ) -> anyhow::Result<wisp_runtime::LaunchedRuntime> {
            let python = key.language == wisp_runtime::RuntimeLanguage::Python;
            Ok(wisp_runtime::LaunchedRuntime::new(
                Box::new(Idle),
                wisp_runtime::RuntimeMetadata {
                    interpreter: Some(
                        if python {
                            "/opt/py/bin/python3"
                        } else {
                            "/opt/r/bin/Rscript"
                        }
                        .into(),
                    ),
                    version: Some(if python { "3.12.4" } else { "4.4.1" }.into()),
                    process_id: None,
                },
            ))
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let session = create(&server).await;
    let runtimes = |id: String| {
        let server = &server;
        async move {
            server
                .conversation_status(PROJECT, &id)
                .await
                .unwrap()
                .runtimes
        }
    };
    // A conversation that ran no code keeps no interpreter.
    assert!(runtimes(session.clone()).await.is_empty());
    assert!(server
        .stop_runtime(PROJECT, &session, "none")
        .await
        .is_err());

    let manager = wisp_runtime::RuntimeManager::new(Arc::new(Launcher));
    *server.session(&session).runtime.lock().unwrap() = Some(manager.clone());
    for key in [
        wisp_runtime::RuntimeKey::local_python(PROJECT),
        wisp_runtime::RuntimeKey::r(PROJECT, wisp_runtime::LOCAL_CONTEXT_ID),
    ] {
        manager
            .start(key.with_session(session.clone()), temp.path().to_path_buf())
            .await
            .unwrap();
    }
    let listed = runtimes(session.clone()).await;
    let seen: Vec<_> = listed
        .iter()
        .map(|row| (row.language, row.state, row.version.as_deref()))
        .collect();
    assert_eq!(
        seen,
        [
            (
                desktop::RuntimeLanguage::Python,
                desktop::RuntimeState::Ready,
                Some("3.12.4")
            ),
            (
                desktop::RuntimeLanguage::R,
                desktop::RuntimeState::Ready,
                Some("4.4.1")
            ),
        ]
    );
    assert_eq!(
        listed[0].interpreter.as_deref(),
        Some("/opt/py/bin/python3")
    );

    // Stopped, it stays listed as stopped; the other is untouched.
    let python = listed[0].id.clone();
    server
        .stop_runtime(PROJECT, &session, &python)
        .await
        .unwrap();
    let listed = runtimes(session.clone()).await;
    assert_eq!(listed[0].state, desktop::RuntimeState::Stopped);
    assert_eq!(listed[1].state, desktop::RuntimeState::Ready);
    // Asked again, the stopped one is taken away.
    server
        .stop_runtime(PROJECT, &session, &python)
        .await
        .unwrap();
    let listed = runtimes(session.clone()).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].language, desktop::RuntimeLanguage::R);
    assert!(server
        .stop_runtime(PROJECT, &session, &python)
        .await
        .is_err());
    // Only through the conversation's own project.
    assert!(server
        .stop_runtime("missing", &session, &listed[0].id)
        .await
        .is_err());
    assert_eq!(runtimes(session.clone()).await.len(), 1);
    server.shutdown().await;
}

#[tokio::test]
async fn the_model_view_is_what_the_next_turn_would_be_sent() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let session = create(&server).await;
    // Nothing has been sent, so nothing is saved to send again.
    let empty = server.model_view(PROJECT, &session).await.unwrap();
    assert!(empty.rows.is_empty() && !empty.compacted);

    send(&server, &session, "count the reads").await.unwrap();
    let shown = wait_for(&server, &session, |s| !s.running).await;
    let view = server.model_view(PROJECT, &session).await.unwrap();
    let kinds: Vec<_> = view.rows.iter().map(|row| row.kind).collect();
    assert_eq!(
        kinds,
        [
            desktop::ModelRowKind::System,
            desktop::ModelRowKind::User,
            desktop::ModelRowKind::Assistant,
        ]
    );
    // The instructions are in it though the transcript never shows them.
    assert!(view.rows[0].chars > 200, "{:?}", view.rows[0].chars);
    assert!(shown.items.iter().all(|item| item.role != "system"));
    assert_eq!(view.rows[1].text, "count the reads");
    assert_eq!(view.rows[2].text, "Saved result");
    assert_eq!(
        view.chars,
        view.rows.iter().map(|row| row.chars).sum::<usize>()
    );

    assert!(server.model_view("missing", &session).await.is_err());
    assert!(server.model_view(PROJECT, "missing").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_that_asks_for_it_is_reviewed_after_each_turn_worth_it() {
    // A model that runs one tool per conversation and answers briefly, and
    // reports one finding whenever it is asked to review.
    struct Checked {
        reviews: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl Provider for Checked {
        fn name(&self) -> &str {
            "offline"
        }
        fn model(&self) -> &str {
            "offline"
        }
        async fn complete(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            let content = if messages[0]
                .content
                .as_text()
                .starts_with("You are a REVIEWER")
            {
                self.reviews.fetch_add(1, Ordering::Relaxed);
                r#"{"summary":"Checked.","findings":[{"claim":"Saved result",
                    "evidence":"the file is answer.txt","fix":"Name it","verdict":"fail"}]}"#
            } else {
                "[]"
            };
            Ok(Completion {
                content: content.into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            _: &[ToolSchema],
            sink: &mut dyn StreamSink,
        ) -> wisp_llm::Result<Completion> {
            if !messages
                .iter()
                .any(|message| message.role == wisp_llm::Role::Tool)
            {
                return Ok(Completion {
                    tool_calls: vec![ToolCall {
                        id: "call-1".into(),
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
            Ok(Completion {
                content: "Saved result".into(),
                finish_reason: Some("stop".into()),
                ..Default::default()
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let reviews = Arc::new(AtomicUsize::new(0));
    let factory: ModelFactory = {
        let reviews = reviews.clone();
        Arc::new(move |_| {
            Box::new(Checked {
                reviews: reviews.clone(),
            })
        })
    };
    let server = Server::open_with_models(config(temp.path()), factory)
        .await
        .unwrap();
    // One turn with a tool, approved, and everything after it settled.
    let turn = |session: String, message: &'static str| {
        let server = &server;
        async move {
            send(server, &session, message).await.unwrap();
            let asked = wait_for(server, &session, |s| !s.approvals.is_empty() || !s.running).await;
            for approval in &asked.approvals {
                server
                    .answer_approval(
                        PROJECT,
                        &session,
                        &approval.approval_id,
                        true,
                        desktop::Remember::No,
                    )
                    .await
                    .unwrap();
            }
            wait_for(server, &session, |s| !s.running).await;
            server.after_turn_settled(PROJECT, &session).await.unwrap();
            server.conversation_status(PROJECT, &session).await.unwrap()
        }
    };
    let choose = |session: String, auto_review: bool| {
        let server = &server;
        async move {
            let options = server
                .conversation_status(PROJECT, &session)
                .await
                .unwrap()
                .options;
            server
                .set_conversation_options(
                    PROJECT,
                    &session,
                    desktop::ConversationOptions {
                        auto_review,
                        ..options
                    },
                )
                .await
                .unwrap()
                .auto_review
        }
    };

    // Off until it is asked for: a turn with a tool is not reviewed.
    let defaults = server.turn_defaults().await.unwrap();
    assert!(!defaults.auto_review);
    let quiet = create(&server).await;
    let status = turn(quiet.clone(), "write the file").await;
    assert!(!status.options.auto_review);
    assert_eq!(status.review, None);
    assert_eq!(reviews.load(Ordering::Relaxed), 0);

    // Asked for in one conversation: its tool turn is reviewed.
    let checked = create(&server).await;
    assert!(choose(checked.clone(), true).await);
    let status = turn(checked.clone(), "write the file").await;
    let Some(desktop::ReviewOutcome::Done(review)) = status.review else {
        panic!("no review: {:?}", status.review);
    };
    assert_eq!(review.findings.len(), 1);
    assert_eq!(review.findings[0].verdict, "fail");
    assert_eq!(reviews.load(Ordering::Relaxed), 1);
    // A short answer without tools is not worth a review, and the last
    // turn's review does not linger over it.
    let status = turn(checked.clone(), "thanks").await;
    assert_eq!(status.review, None);
    assert_eq!(reviews.load(Ordering::Relaxed), 1);
    // The other conversation was not switched on by this one.
    assert!(
        !server
            .conversation_status(PROJECT, &quiet)
            .await
            .unwrap()
            .options
            .auto_review
    );

    // On by default from now on: conversations that never chose follow,
    // one that chose keeps its choice.
    // (Saying what it already is writes nothing; turning it on and off
    // again is a choice.)
    assert!(!choose(quiet.clone(), false).await);
    assert!(choose(quiet.clone(), true).await);
    assert!(!choose(quiet.clone(), false).await);
    let saved = server
        .set_turn_defaults(desktop::TurnDefaults {
            auto_review: true,
            ..defaults
        })
        .await
        .unwrap();
    assert!(saved.auto_review);
    let fresh = create(&server).await;
    let status = turn(fresh.clone(), "write the file").await;
    assert!(status.options.auto_review);
    assert!(matches!(
        status.review,
        Some(desktop::ReviewOutcome::Done(_))
    ));
    assert_eq!(reviews.load(Ordering::Relaxed), 2);
    assert!(
        !server
            .conversation_status(PROJECT, &quiet)
            .await
            .unwrap()
            .options
            .auto_review
    );

    // Put away, it is gone; only through the conversation's own project.
    assert!(server.dismiss_review("missing", &fresh).await.is_err());
    server.dismiss_review(PROJECT, &fresh).await.unwrap();
    assert_eq!(
        server
            .conversation_status(PROJECT, &fresh)
            .await
            .unwrap()
            .review,
        None
    );
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_started_as_an_expert_works_under_its_instructions() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let experts = server
        .save_expert(desktop::ExpertInput {
            id: String::new(),
            name: "Statistician".into(),
            description: "Numbers".into(),
            instructions: "Always report medians with the interquartile range.".into(),
        })
        .await
        .unwrap();
    let stats = experts
        .iter()
        .find(|expert| expert.name == "Statistician")
        .unwrap()
        .clone();
    assert_eq!(server.experts().await.unwrap(), experts);

    let choose = |session: String, expert: String| {
        let server = &server;
        async move {
            let options = server
                .conversation_status(PROJECT, &session)
                .await
                .unwrap()
                .options;
            server
                .set_conversation_options(
                    PROJECT,
                    &session,
                    desktop::ConversationOptions { expert, ..options },
                )
                .await
        }
    };
    let prompt = |session: String| {
        let server = &server;
        async move {
            server.model_view(PROJECT, &session).await.unwrap().rows[0]
                .text
                .clone()
        }
    };
    let session = create(&server).await;
    assert!(choose(session.clone(), "ghost".into()).await.is_err());
    assert_eq!(
        choose(session.clone(), stats.id.clone())
            .await
            .unwrap()
            .expert,
        stats.id
    );
    send(&server, &session, "summarise the counts")
        .await
        .unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    let instructed = prompt(session.clone()).await;
    assert!(
        instructed.ends_with(
            "## Specialist: Statistician\nAlways report medians with the interquartile range."
        ) || instructed.contains(
            "## Specialist: Statistician\nAlways report medians with the interquartile range."
        ),
        "{instructed}"
    );
    // Once it has started, the conversation is what it is.
    let late = choose(session.clone(), String::new()).await.unwrap_err();
    assert!(
        late.to_string().contains("before the first message"),
        "{late}"
    );
    // Removing the expert does not rewrite the conversation.
    server.delete_expert(&stats.id).await.unwrap();
    send(&server, &session, "and again").await.unwrap();
    wait_for(&server, &session, |s| !s.running).await;
    assert!(prompt(session.clone())
        .await
        .contains("## Specialist: Statistician"));
    assert_eq!(
        server
            .conversation_status(PROJECT, &session)
            .await
            .unwrap()
            .options
            .expert,
        ""
    );

    // A conversation with no expert has no such section.
    let plain = create(&server).await;
    send(&server, &plain, "hello").await.unwrap();
    wait_for(&server, &plain, |s| !s.running).await;
    assert!(!prompt(plain).await.contains("## Specialist"));
    assert!(!server.commands().iter().any(|name| name.contains("expert")));
    server.shutdown().await;
}

#[tokio::test]
async fn a_conversation_is_exported_whole_as_markdown() {
    let temp = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    // A model that runs one tool, then answers.
    let server = Server::open_with_models(config(temp.path()), models(calls, true))
        .await
        .unwrap();
    let session = create(&server).await;
    send(&server, &session, "write the file").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty() || !s.running).await;
    for approval in &asked.approvals {
        server
            .answer_approval(
                PROJECT,
                &session,
                &approval.approval_id,
                true,
                desktop::Remember::No,
            )
            .await
            .unwrap();
    }
    wait_for(&server, &session, |s| !s.running).await;
    send(&server, &session, "and again").await.unwrap();
    let asked = wait_for(&server, &session, |s| !s.approvals.is_empty() || !s.running).await;
    for approval in &asked.approvals {
        server
            .answer_approval(
                PROJECT,
                &session,
                &approval.approval_id,
                true,
                desktop::Remember::No,
            )
            .await
            .unwrap();
    }
    wait_for(&server, &session, |s| !s.running).await;

    let text = server.export_conversation(PROJECT, &session).await.unwrap();
    // The title, the project, and both questions in the order they were asked.
    assert!(
        text.starts_with("# write the file\n\nProject: Server\n"),
        "{text}"
    );
    let first = text.find("## You\n\nwrite the file").unwrap();
    let second = text.find("## You\n\nand again").unwrap();
    assert!(first < second, "{text}");
    // What the model did and said is between them.
    let between = &text[first..second];
    assert!(between.contains("**Tool `"), "{text}");
    assert!(between.contains("## Wisp"), "{text}");
    // Only the conversation's own project may export it.
    assert!(server
        .export_conversation("missing", &session)
        .await
        .is_err());
    assert!(server
        .export_conversation(PROJECT, "missing")
        .await
        .is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn capabilities_count_what_a_project_can_draw_on() {
    let temp = tempfile::tempdir().unwrap();
    let server = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let start = server.capabilities(PROJECT).await.unwrap();
    // Every bundled connector is on to begin with; nothing else is set up.
    assert!(start.connectors.1 > 0 && start.connectors.0 == start.connectors.1);
    assert_eq!(start.mcp_servers, (0, 0));
    assert_eq!(start.hosts, (0, 0));
    assert_eq!((start.memory_notes, start.habits), (0, 0));
    assert!(start.memory_on);
    // Each tool looked for is listed, found or not.
    assert_eq!(start.tools.len(), 8);
    assert!(start.tools.iter().any(|tool| tool.name == "git"));
    for tool in &start.tools {
        if let Some(path) = &tool.path {
            assert!(std::path::Path::new(path).is_file(), "{path}");
        }
    }

    // What is switched off or added shows in the counts.
    let connector = server.connectors().await.unwrap()[0].domain.clone();
    server
        .set_connector_enabled(&connector, false)
        .await
        .unwrap();
    server
        .save_ssh_host(desktop::SshHostRow {
            alias: "cap-host".into(),
            host_name: "10.0.0.9".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    server.add_habit("Report effect sizes").await.unwrap();
    server.set_memory_enabled(false).await.unwrap();
    let after = server.capabilities(PROJECT).await.unwrap();
    assert_eq!(
        after.connectors,
        (start.connectors.1 - 1, start.connectors.1)
    );
    // Saved, but not checked: it cannot run anything yet.
    assert_eq!(after.hosts, (0, 1));
    assert_eq!(after.habits, 1);
    assert!(!after.memory_on);
    assert!(server.capabilities("missing").await.is_err());
    server.shutdown().await;
}

#[tokio::test]
async fn a_project_is_exported_as_an_archive_and_imported_elsewhere() {
    let temp = tempfile::tempdir().unwrap();
    let source = Server::open_with_models(config(temp.path()), models(Arc::default(), false))
        .await
        .unwrap();
    // A project with a conversation and a file of its own.
    let folder = temp.path().join("lab");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("counts.csv"), "gene,count\nA,1\n").unwrap();
    let lab = source.register_local_project(&folder).await.unwrap();
    let session = source.create_conversation(&lab).await.unwrap();
    source
        .0
        .store
        .rename_session(&session, &lab, "Alignment")
        .await
        .unwrap();

    let archive = temp.path().join("lab.zip");
    // Where to write must be said in full.
    assert!(source
        .export_project(&lab, std::path::Path::new("lab.zip"))
        .await
        .is_err());
    assert!(source.export_project("missing", &archive).await.is_err());
    source.export_project(&lab, &archive).await.unwrap();
    assert!(archive.is_file());
    // The project is already here: its own archive is not imported twice.
    let parent = temp.path().join("imports");
    std::fs::create_dir(&parent).unwrap();
    let again = source.import_project(&archive, &parent).await.unwrap_err();
    assert!(again.to_string().contains("already present"), "{again}");
    source.shutdown().await;
    drop(source);

    // Another host, with a database of its own, reads it back whole.
    let other = tempfile::tempdir().unwrap();
    let target = Server::open_with_models(config(other.path()), models(Arc::default(), false))
        .await
        .unwrap();
    let parent = other.path().join("imports");
    std::fs::create_dir(&parent).unwrap();
    // Not inside the host's own data, and only from a real archive.
    assert!(target
        .import_project(&archive, &other.path().join("data"))
        .await
        .is_err());
    assert!(target
        .import_project(&other.path().join("missing.zip"), &parent)
        .await
        .is_err());
    let imported = target.import_project(&archive, &parent).await.unwrap();
    assert_eq!(imported, lab);
    let settings = target.project_settings(&imported).await.unwrap();
    assert_eq!(settings.name, "lab");
    let sessions = target.0.store.list_sessions(&imported).await.unwrap();
    assert_eq!(sessions.len(), 1);
    // Its files came along, under the folder that was chosen.
    let (_, _, root) = target
        .0
        .store
        .get_project_meta(&imported)
        .await
        .unwrap()
        .unwrap();
    assert!(std::path::Path::new(&root).starts_with(std::fs::canonicalize(&parent).unwrap()));
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&root).join("counts.csv")).unwrap(),
        "gene,count\nA,1\n"
    );
    target.shutdown().await;
}
