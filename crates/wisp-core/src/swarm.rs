//! Host-observed progress shared by batch workers and read-only subagents.
use std::sync::Mutex;
use wisp_dto::{SwarmPhase, SwarmProgress, SwarmTask, SwarmTaskStatus};

pub struct SwarmReporter<'a> {
    state: Mutex<SwarmProgress>,
    emit: Box<dyn Fn(SwarmProgress) + Send + Sync + 'a>,
}

pub fn clip(text: &str, chars: usize) -> String {
    let mut value: String = text.chars().take(chars).collect();
    if text.chars().count() > chars {
        value.push('…');
    }
    value
}

impl<'a> SwarmReporter<'a> {
    fn event(state: &SwarmProgress, tasks: Vec<SwarmTask>) -> SwarmProgress {
        SwarmProgress {
            id: state.id.clone(),
            source: state.source.clone(),
            title: state.title.clone(),
            model: state.model.clone(),
            started_at_ms: state.started_at_ms,
            revision: state.revision,
            phase: state.phase,
            total: state.total,
            summary: state.summary.clone(),
            tasks,
        }
    }
    pub fn new(
        source: &str,
        title: &str,
        model: &str,
        emit: impl Fn(SwarmProgress) + Send + Sync + 'a,
    ) -> Self {
        let state = SwarmProgress {
            id: uuid::Uuid::new_v4().to_string(),
            source: source.into(),
            title: clip(title, 240),
            model: model.into(),
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            revision: 1,
            phase: SwarmPhase::Starting,
            total: 0,
            tasks: vec![],
            summary: String::new(),
        };
        emit(state.clone());
        Self {
            state: Mutex::new(state),
            emit: Box::new(emit),
        }
    }

    pub fn for_tool(
        source: &str,
        title: &str,
        model: &str,
        env: &'a dyn wisp_tools::ToolEnv,
    ) -> Self {
        Self::new(source, title, model, move |event| {
            if let Ok(payload) = serde_json::to_value(event) {
                env.publish_progress(wisp_dto::SWARM_PRESENTATION, &payload);
            }
        })
    }

    pub fn tasks(&self, tasks: Vec<SwarmTask>, model: &str) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.tasks = tasks;
        state.total = state.tasks.len();
        state.model = model.into();
        state.revision += 1;
        (self.emit)(state.clone());
    }

    pub fn phase(&self, phase: SwarmPhase, summary: &str) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.revision += 1;
        state.phase = phase;
        state.summary = clip(summary, 8_000);
        if phase.terminal() {
            let revision = state.revision;
            for task in &mut state.tasks {
                if matches!(
                    task.status,
                    SwarmTaskStatus::Queued | SwarmTaskStatus::Running
                ) {
                    task.status = if phase == SwarmPhase::Cancelled {
                        SwarmTaskStatus::Cancelled
                    } else {
                        SwarmTaskStatus::Paused
                    };
                    task.revision = revision;
                }
            }
        }
        let update = Self::event(
            &state,
            if phase.terminal() {
                state.tasks.clone()
            } else {
                vec![]
            },
        );
        (self.emit)(update);
    }

    pub fn task(&self, id: &str, change: impl FnOnce(&mut SwarmTask)) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.revision += 1;
        let revision = state.revision;
        let Some(task) = state.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        change(task);
        task.revision = revision;
        task.result = clip(&task.result, 8_000);
        if task.log.len() > 12 {
            task.log.drain(..task.log.len() - 12);
        }
        for line in &mut task.log {
            *line = clip(line, 400);
        }
        let task = task.clone();
        let update = Self::event(&state, vec![task]);
        (self.emit)(update);
    }

    pub fn note(&self, id: &str, message: &str) {
        self.task(id, |task| task.log.push(message.into()));
    }
}

impl Drop for SwarmReporter<'_> {
    fn drop(&mut self) {
        let active = !self
            .state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .phase
            .terminal();
        if active {
            self.phase(
                SwarmPhase::Paused,
                "Execution ended before a final result was reported.",
            );
        }
    }
}

/// Emit tool observations from a nested loop; never relay its private CoT.
pub struct SwarmOutput<'a, 'b> {
    pub reporter: &'a SwarmReporter<'b>,
    pub task_id: &'a str,
}

impl crate::Output for SwarmOutput<'_, '_> {
    fn tool_call(&self, name: &str, preview: &str) {
        self.reporter
            .note(self.task_id, &format!("{name}: {preview}"));
    }
    fn tool_result(&self, name: &str, ok: bool, content: &str, _duration_ms: u64) {
        self.reporter.note(
            self.task_id,
            &format!(
                "{name} — {}: {}",
                if ok { "OK" } else { "Failed" },
                clip(content, 320)
            ),
        );
    }
}
