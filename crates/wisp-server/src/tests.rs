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
        provider: wisp_llm::ProviderConfig::openai("http://127.0.0.1:9", "synthetic", "offline"),
        max_context: 128_000,
        supports_vision: false,
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
