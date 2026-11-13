use std::time::Duration;

use gpui::{prelude::*, *};
use gpui_component::{
    ActiveTheme, Disableable, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState},
    text::TextView,
};
use serde_json::{Value, json};
use wisp_dto::{
    ProjectSummary, SessionSearchInfo,
    native_conversations::{Item, Snapshot},
};

use crate::{
    backend::{Backend, receive},
    state::WorkspaceState,
};

pub struct Workspace {
    backend: Backend,
    state: WorkspaceState,
    projects: Vec<ProjectSummary>,
    sessions: Vec<SessionSearchInfo>,
    composer: Entity<InputState>,
    search: Entity<InputState>,
    rows: Vec<Item>,
    history: Vec<Item>,
    before: Option<i64>,
    list: ListState,
    ready: bool,
    pending: bool,
    loading_history: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
    poll: Option<Task<()>>,
}

impl Workspace {
    pub fn new(backend: Backend, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .auto_grow(3, 9)
                .placeholder("Describe your research task…")
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Filter conversations…"));
        let subscriptions = vec![
            cx.subscribe_in(&composer, window, |this, _, event, window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.state
                        .save_draft(this.composer.read(cx).value().to_string());
                    cx.notify();
                }
                // Let the input component and platform IME consume composition.
                // Enter inserts a newline; platform secondary+Enter sends.
                if matches!(event, InputEvent::PressEnter { secondary: true }) {
                    this.send(window, cx);
                }
            }),
            cx.subscribe(&search, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        let mut view = Self {
            backend,
            state: WorkspaceState::default(),
            projects: vec![],
            sessions: vec![],
            composer,
            search,
            rows: vec![],
            history: vec![],
            before: None,
            list: ListState::new(0, ListAlignment::Bottom, px(300.)),
            ready: false,
            pending: false,
            loading_history: false,
            error: None,
            _subscriptions: subscriptions,
            poll: None,
        };
        view.initialize(window, cx);
        view
    }

    fn initialize(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending = true;
        self.error = None;
        let reply = self.backend.initialize();
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(()) => {
                        this.ready = true;
                        this.load_projects(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self
            .backend
            .request::<Vec<ProjectSummary>>("list_projects", None, json!({}));
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(projects) => {
                        this.projects = projects;
                        if this.state.project.is_none() && !this.pending {
                            if let Some(first) = this.projects.first() {
                                this.select_project(first.id.clone(), window, cx);
                            }
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_project(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state
            .save_draft(self.composer.read(cx).value().to_string());
        self.state.select(Some(id), None);
        self.sessions.clear();
        self.clear_transcript();
        self.composer
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.load_sessions(window, cx);
    }

    fn clear_transcript(&mut self) {
        self.poll = None;
        self.rows.clear();
        self.history.clear();
        self.before = None;
        self.loading_history = false;
        self.error = None;
        self.list.reset(0);
    }

    fn load_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        let reply = self.backend.request::<Vec<SessionSearchInfo>>(
            "remote_sessions",
            selection.project.clone(),
            json!({}),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(sessions) => {
                        this.sessions = sessions;
                        if this.state.session.is_none() && !this.pending {
                            if let Some(first) = this.sessions.first() {
                                this.select_session(first.id.clone(), window, cx);
                            }
                        }
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn select_session(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.state
            .save_draft(self.composer.read(cx).value().to_string());
        self.state.select(self.state.project.clone(), Some(id));
        self.clear_transcript();
        let draft = self.state.draft();
        self.composer.update(cx, |input, cx| {
            input.set_value(draft, window, cx);
            input.focus(window, cx);
        });
        self.start_poll(window, cx);
    }

    fn start_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.state.selection();
        if selection.session.is_none() {
            return;
        }
        let backend = self.backend.clone();
        self.poll = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                let result = receive(backend.request::<Snapshot>(
                    "native_conversation_snapshot",
                    selection.project.clone(),
                    json!({"session_id": selection.session}),
                ))
                .await;
                let keep_polling = this
                    .update_in(cx, |this, window, cx| {
                        if !this.state.current(&selection) {
                            return false;
                        }
                        match result {
                            Ok(snapshot) => {
                                let was_running =
                                    this.state.snapshot.as_ref().is_some_and(|s| s.running);
                                let running = snapshot.running;
                                if this.state.accept(&selection, snapshot) {
                                    this.update_rows();
                                    if was_running && !running {
                                        this.load_sessions(window, cx);
                                    }
                                    cx.notify();
                                }
                                running
                            }
                            Err(error) => {
                                this.error = Some(error);
                                cx.notify();
                                false
                            }
                        }
                    })
                    .unwrap_or(false);
                if !keep_polling {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }
        }));
    }

    fn update_rows(&mut self) {
        let Some(snapshot) = &self.state.snapshot else {
            return;
        };
        if self.history.is_empty() {
            self.before = snapshot.next_before_seq;
        }
        let mut rows = self.history.clone();
        rows.extend(snapshot.items.clone());
        // Keep cached heights for unchanged rows. Streaming invalidates just its
        // changing suffix; GPUI lays out only visible rows and the overdraw area.
        let prefix = self
            .rows
            .iter()
            .zip(&rows)
            .take_while(|(a, b)| {
                a.role == b.role
                    && a.text == b.text
                    && a.tool_name == b.tool_name
                    && a.input == b.input
                    && a.ok == b.ok
                    && a.status == b.status
            })
            .count();
        self.list
            .splice(prefix..self.rows.len(), rows.len() - prefix);
        self.rows = rows;
    }

    fn older(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(before) = self.before else {
            return;
        };
        if self.loading_history {
            return;
        }
        self.loading_history = true;
        let selection = self.state.selection();
        let reply = self.backend.request::<Snapshot>(
            "native_conversation_snapshot",
            selection.project.clone(),
            json!({"session_id":selection.session,"before_seq":before}),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update(cx, |this, cx| {
                if !this.state.current(&selection) {
                    return;
                }
                this.loading_history = false;
                match result {
                    Ok(page) => {
                        let mut history = page.items;
                        history.append(&mut this.history);
                        this.history = history;
                        this.before = page.next_before_seq;
                        this.update_rows();
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn create_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || self.state.project.is_none() {
            return;
        }
        self.pending = true;
        let selection = self.state.selection();
        let reply = self.backend.request::<String>(
            "native_conversation_create",
            selection.project.clone(),
            json!({}),
        );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(id) => {
                        this.select_session(id, window, cx);
                        this.load_sessions(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().to_string();
        if !self.state.can_send(&text, self.pending) || self.loading_history {
            return;
        }
        self.pending = true;
        self.error = None;
        self.poll = None;
        let selection = self.state.selection();
        let reply = self
            .backend
            .request::<wisp_dto::native_conversations::SendAccepted>(
                "native_conversation_send",
                selection.project.clone(),
                json!({
                    "session_id": selection.session, "request_id": uuid::Uuid::new_v4().to_string(),
                    "message": text.trim(),
                }),
            );
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if !this.state.current(&selection) {
                    return;
                }
                match result {
                    Ok(_) => {
                        // Clear only after durable acceptance. Failure preserves the draft.
                        this.composer
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.state.save_draft(String::new());
                        this.history.clear();
                        if let Some(snapshot) = &mut this.state.snapshot {
                            snapshot.running = true;
                        }
                        this.load_sessions(window, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                this.start_poll(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn control(
        &mut self,
        command: &'static str,
        mut args: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending {
            return;
        }
        self.pending = true;
        self.error = None;
        let selection = self.state.selection();
        args["session_id"] = json!(selection.session);
        let reply = self
            .backend
            .request::<Value>(command, selection.project.clone(), args);
        cx.spawn_in(window, async move |this, cx| {
            let result = receive(reply).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                if !this.state.current(&selection) {
                    return;
                }
                if let Err(error) = result {
                    this.error = Some(error);
                }
                this.start_poll(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending || !self.ready {
            return;
        }
        self.pending = true;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open research folder".into()),
        });
        let backend = self.backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let paths = picker
                    .await
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                match paths.and_then(|mut paths| paths.pop()) {
                    Some(path) => receive(backend.register_project(path)).await.map(Some),
                    None => Ok(None),
                }
            }
            .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.pending = false;
                match result {
                    Ok(Some(id)) => {
                        this.select_project(id, window, cx);
                        this.load_projects(window, cx);
                    }
                    Ok(None) => {}
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn row(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.rows[index];
        let heading = item
            .tool_name
            .clone()
            .unwrap_or_else(|| match item.role.as_str() {
                "user" => "You".into(),
                "assistant" => "Wisp".into(),
                other => other.into(),
            });
        let body = if item.text.is_empty() {
            item.input.clone().unwrap_or_default()
        } else {
            item.text.clone()
        };
        div()
            .v_flex()
            .gap_2()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(heading),
            )
            .child(
                TextView::markdown(
                    SharedString::from(format!(
                        "message-{}-{index}",
                        self.state.selection().generation
                    )),
                    body,
                    window,
                    cx,
                )
                .selectable(true),
            )
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.state.snapshot.as_ref().is_some_and(|s| s.running);
        let disabled = self.pending || !self.ready;
        let query = self.search.read(cx).value().to_lowercase();
        let title = self
            .sessions
            .iter()
            .find(|s| Some(&s.id) == self.state.session.as_ref())
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "Research workspace".into());
        let weak = cx.entity().downgrade();
        let transcript = list(self.list.clone(), move |index, window, cx| {
            weak.update(cx, |this, cx| this.row(index, window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .flex_1()
        .min_h_0();
        let mut sidebar = div()
            .v_flex()
            .w(px(260.))
            .flex_shrink_0()
            .h_full()
            .p_3()
            .gap_3()
            .bg(cx.theme().muted)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(div().text_lg().child("Wisp Science"))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("GPUI Preview"),
            )
            .child(
                Button::new("open-project")
                    .label("Open folder")
                    .disabled(disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.open_project(window, cx))),
            )
            .child(div().text_sm().child("Projects"));
        let mut projects = div()
            .id("projects")
            .v_flex()
            .gap_1()
            .max_h(px(200.))
            .overflow_y_scroll();
        for project in &self.projects {
            let id = project.id.clone();
            projects = projects.child(
                Button::new(SharedString::from(format!("project-{id}")))
                    .label(project.name.clone())
                    .selected(Some(&id) == self.state.project.as_ref())
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_project(id.clone(), window, cx)
                    })),
            );
        }
        sidebar = sidebar
            .child(projects)
            .child(
                Button::new("new-chat")
                    .label("New conversation")
                    .disabled(disabled || self.state.project.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.create_session(window, cx))),
            )
            .child(Input::new(&self.search));
        let mut sessions = div()
            .id("sessions")
            .v_flex()
            .gap_1()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll();
        for session in self
            .sessions
            .iter()
            .filter(|s| s.title.to_lowercase().contains(&query))
        {
            let id = session.id.clone();
            sessions = sessions.child(
                Button::new(SharedString::from(format!("session-{id}")))
                    .label(session.title.clone())
                    .selected(Some(&id) == self.state.session.as_ref())
                    .disabled(disabled)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_session(id.clone(), window, cx)
                    })),
            );
        }
        sidebar = sidebar.child(sessions);
        let mut content = div().v_flex().flex_1().min_w_0().h_full().child(
            div()
                .h_flex()
                .justify_between()
                .p_4()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(div().child(title))
                .child(
                    Button::new("refresh")
                        .label("Refresh")
                        .disabled(disabled)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.error = None;
                            this.load_projects(window, cx);
                            if this.state.project.is_some() {
                                this.load_sessions(window, cx);
                            }
                            this.start_poll(window, cx);
                        })),
                ),
        );
        if let Some(error) = self
            .error
            .as_ref()
            .or_else(|| self.state.snapshot.as_ref().and_then(|s| s.error.as_ref()))
        {
            content = content.child(
                div()
                    .p_4()
                    .text_color(cx.theme().danger)
                    .child(error.clone()),
            );
        }
        if !self.ready {
            content = content.child(div().v_flex().p_6().gap_3()
                .child("The GPUI desktop uses a local Rust host.")
                .child("Configure WISP_MODEL and WISP_API_KEY before launching. See docs/gpui-desktop.md for provider and workspace settings.")
                .child(Button::new("retry").label("Retry startup").disabled(self.pending)
                    .on_click(cx.listener(|this, _, window, cx| this.initialize(window, cx)))));
        } else if self.state.session.is_none() {
            content = content.child(
                div()
                    .p_6()
                    .child("Open a research folder and start a conversation."),
            );
        }
        if self.before.is_some() {
            content = content.child(
                Button::new("older")
                    .label("Load earlier messages")
                    .disabled(self.loading_history || self.pending || running)
                    .on_click(cx.listener(|this, _, window, cx| this.older(window, cx))),
            );
        }
        content = content.child(transcript);
        let mut approvals = div()
            .id("approvals")
            .v_flex()
            .gap_2()
            .max_h(px(220.))
            .overflow_y_scroll();
        if let Some(snapshot) = &self.state.snapshot {
            for approval in &snapshot.approvals {
                let approve = approval.approval_id.clone();
                let deny = approve.clone();
                approvals = approvals.child(div().v_flex().gap_2().p_3().border_t_1().border_color(cx.theme().border)
                    .child(format!("{}: {}", approval.tool, approval.message))
                    .child(approval.preview.clone())
                    .child(div().h_flex().gap_2()
                        .child(Button::new(SharedString::from(format!("allow-{approve}"))).label("Allow once").disabled(self.pending)
                            .on_click(cx.listener(move |this, _, window, cx| this.control("native_conversation_approve",
                                json!({"approval_id":approve,"approved":true,"scope":"once"}), window, cx))))
                        .child(Button::new(SharedString::from(format!("deny-{deny}"))).label("Deny").disabled(self.pending)
                            .on_click(cx.listener(move |this, _, window, cx| this.control("native_conversation_approve",
                                json!({"approval_id":deny,"approved":false,"scope":"once"}), window, cx))))));
            }
        }
        let can_send = self
            .state
            .can_send(&self.composer.read(cx).value(), self.pending)
            && !self.loading_history;
        content = content.child(approvals).child(
            div()
                .v_flex()
                .p_4()
                .gap_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    Input::new(&self.composer).disabled(disabled || self.state.session.is_none()),
                )
                .child(
                    div()
                        .h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(if running {
                                    "Agent working…"
                                } else if cfg!(target_os = "macos") {
                                    "Enter: newline · Cmd+Enter: send"
                                } else {
                                    "Enter: newline · Ctrl+Enter: send"
                                }),
                        )
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .child(
                                    Button::new("stop")
                                        .label("Stop")
                                        .disabled(!running || self.pending)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.control(
                                                "native_conversation_stop",
                                                json!({}),
                                                window,
                                                cx,
                                            )
                                        })),
                                )
                                .child(
                                    Button::new("send")
                                        .label("Send")
                                        .primary()
                                        .disabled(!can_send)
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.send(window, cx)
                                            }),
                                        ),
                                ),
                        ),
                ),
        );
        let _ = window;
        div()
            .h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(sidebar)
            .child(content)
    }
}
