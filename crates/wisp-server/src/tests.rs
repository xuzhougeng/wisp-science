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
    for result in [
        server.archive_conversation(PROJECT, &session, true).await,
        server.delete_conversation(PROJECT, &session).await,
    ] {
        assert!(result.unwrap_err().to_string().contains("Stop the running"));
    }
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
        model: String::new(),
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
        .answer_approval(PROJECT, &asking, &approval, false, true)
        .await
        .is_err());
    assert!(server
        .answer_approval(PROJECT, &asking, "missing", true, false)
        .await
        .is_err());
    server
        .answer_approval(PROJECT, &asking, &approval, true, true)
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
