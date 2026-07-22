//! Conversation context. The manager NEVER rewrites history on its own:
//! crossing the 80% warning threshold only fires `Output::context_warning`
//! once, and the user decides when to run `/compact`.
//!
//! `/compact` (`ContextManager::compact`) first archives the full history to a
//! file — every tombstone it leaves behind names that file, so the model can
//! read/grep it to retrieve anything folded away — then:
//! 1. `fold_old_turns` — tombstone old tool outputs, truncate oversized old
//!    assistant text/reasoning, replace old images with text notes.
//! 2. `compact_conversation` — drop oldest turns while still over budget.
//! 3. `full_compact` — LLM-driven summary (last resort).

use crate::output::Output;
use std::path::Path;
use wisp_llm::{Content, Message, Part, Provider, Role, ToolCall};

/// Turns kept verbatim by `/compact`.
const RETAIN_TURNS: usize = 10;
/// Recent turns the drop-oldest fallback protects when folding isn't enough.
const RETAIN_TURNS_HARD: usize = 8;
/// Old assistant text larger than this (estimated tokens) is head/tail-cut.
const OLD_ASSISTANT_MAX_TOKENS: usize = 1500;
const OLD_ASSISTANT_KEEP: (usize, usize) = (350, 350);
/// Old reasoning larger than this (estimated tokens) is head/tail-cut.
const OLD_REASONING_MAX_TOKENS: usize = 500;
const OLD_REASONING_KEEP: (usize, usize) = (125, 125);
/// Prefix of every tombstone `/compact` writes. Also the "already compacted"
/// marker: a later `/compact` must not overwrite an old tombstone, or it would
/// repoint at a newer archive that itself only contains tombstones.
pub const TOMBSTONE_PREFIX: &str = "[compacted;";

/// Largest char boundary `<= i` (std's `floor_char_boundary` is still unstable).
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Smallest char boundary `>= i`.
fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

pub struct ContextManager {
    pub messages: Vec<Message>,
    pub max_context: usize,
    /// 80% of `max_context`; crossing it fires a one-time `context_warning`.
    warn_threshold: usize,
    /// Set once the warning fired; reset when back under the threshold.
    warned: bool,
    pub runtime_injections: Vec<Message>,
}

impl ContextManager {
    pub fn new(max_context: usize) -> Self {
        Self {
            messages: vec![],
            max_context,
            warn_threshold: (max_context as f64 * 0.8) as usize,
            warned: false,
            runtime_injections: vec![],
        }
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    pub fn append_system(&mut self, content: impl Into<String>) {
        self.messages.push(Message::system(content));
    }
    pub fn append_user(&mut self, content: impl Into<String>) {
        self.append_user_content(Content::text(content));
    }
    pub fn append_user_content(&mut self, content: Content) {
        let mut message = Message::user("");
        message.content = content;
        self.messages.push(message);
    }
    pub fn inject_user(&mut self, content: impl Into<String>) {
        self.runtime_injections.push(Message::user(content));
    }
    pub fn clear_runtime_injections(&mut self) {
        self.runtime_injections.clear();
    }

    pub fn append_assistant(
        &mut self,
        content: String,
        tool_calls: Vec<ToolCall>,
        reasoning: Option<String>,
    ) {
        let mut m = Message::assistant(content);
        m.tool_calls = tool_calls;
        m.reasoning = reasoning;
        self.messages.push(m);
    }

    pub fn append_tool(
        &mut self,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: wisp_llm::Content,
    ) {
        let mut m = Message::tool(tool_call_id, tool_name, content.as_text());
        m.content = content;
        self.messages.push(m);
    }

    pub fn get_messages(&self) -> &[Message] {
        &self.messages
    }
    pub fn get_latest(&self, n: usize) -> &[Message] {
        let start = self.messages.len().saturating_sub(n);
        &self.messages[start..]
    }

    pub fn load(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(s) => match serde_json::from_str::<Vec<Message>>(&s) {
                Ok(v) => self.messages = v,
                Err(e) => {
                    self.backup(path);
                    self.messages.clear();
                    tracing::warn!("session file corrupted ({e}); backed up and reset.");
                }
            },
            Err(_) => {
                self.messages.clear();
            }
        }
    }

    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let s = serde_json::to_string_pretty(&self.messages).unwrap_or_default();
        let _ = std::fs::write(path, s);
    }

    pub fn backup(&self, path: &Path) {
        if !path.exists() {
            return;
        }
        let bak = path.with_extension(format!("{}.backup", chrono::Utc::now().timestamp()));
        let _ = std::fs::rename(path, &bak);
    }

    pub fn compact_text(text: &str, head: usize, tail: usize) -> String {
        let t = text.trim();
        if t.is_empty() {
            return String::new();
        }
        if t.len() <= head + tail {
            return t.to_string();
        }
        // head/tail are byte budgets; snap them to UTF-8 char boundaries so
        // multi-byte (e.g. CJK) text never slices mid-character. A mid-char
        // slice panics, and with panic=abort that crashes the whole app when a
        // long conversation gets compacted (#45).
        let h = floor_char_boundary(t, head);
        let ts = ceil_char_boundary(t, t.len() - tail);
        format!("{}\n...\n{}", &t[..h], &t[ts..])
    }

    /// Like `compact_text` but with a caller-supplied elision marker between
    /// head and tail. Caller guarantees `text.len() > head + tail`. Same UTF-8
    /// boundary snapping as `compact_text` (#45).
    pub fn truncate_middle(text: &str, head: usize, tail: usize, marker: &str) -> String {
        let h = floor_char_boundary(text, head);
        let t = ceil_char_boundary(text, text.len() - tail);
        format!("{}\n{}\n{}", &text[..h], marker, &text[t..])
    }

    /// Split into turns: each turn = one user message + the assistant/tool
    /// messages that follow, up to the next user message. System skipped.
    pub fn split_turns(&self) -> Vec<Vec<Message>> {
        let mut turns: Vec<Vec<Message>> = vec![];
        let mut current: Vec<Message> = vec![];
        for m in &self.messages {
            if m.role == Role::System {
                continue;
            }
            if m.role == Role::User && !current.is_empty() {
                turns.push(std::mem::take(&mut current));
            }
            current.push(m.clone());
        }
        if !current.is_empty() {
            turns.push(current);
        }
        turns
    }

    fn role_msgs(&self, role: Role, n: Option<usize>) -> Vec<&Message> {
        let slice = match n {
            Some(n) => self.get_latest(n),
            None => &self.messages[..],
        };
        slice.iter().filter(|m| m.role == role).collect()
    }

    fn tool_names_of(msgs: &[&Message]) -> Vec<String> {
        msgs.iter()
            .filter(|m| m.role == Role::Tool)
            .filter_map(|m| m.tool_name.clone())
            .collect()
    }

    #[allow(dead_code)]
    fn last_user_content(&self, msgs: Option<&[Message]>) -> String {
        let src = msgs.unwrap_or(&self.messages);
        src.iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_text())
            .unwrap_or_default()
    }

    fn under_threshold(&self) -> bool {
        self.total_tokens() < self.warn_threshold
    }

    pub fn tool_pattern(&self, n: usize) -> Option<Vec<String>> {
        let tools = Self::tool_names_of(&self.role_msgs(Role::Tool, Some(n)));
        if tools.is_empty() {
            None
        } else {
            Some(tools)
        }
    }

    pub fn tool_context(&self, n: usize, cap: usize) -> String {
        let mut tc = vec![];
        for m in self.role_msgs(Role::Tool, Some(n)) {
            let content = m.content.as_text();
            let compacted = if content.len() > cap {
                Self::compact_text(&content, 200, 200)
            } else {
                content.clone()
            };
            let name = m.tool_name.clone().unwrap_or_default();
            tc.push(format!("[{name} tool] {compacted}"));
        }
        tc.join("\n\n")
    }

    /// Rough token estimate (~JSON length / 4) from field lengths directly.
    /// The old serialize-to-measure version dominated the compaction hot path:
    /// it re-encoded every message to JSON on every `total_tokens()` call.
    pub fn estimated_tokens(msg: &Message) -> usize {
        let mut n = 32; // role + envelope punctuation
        n += match &msg.content {
            Content::Text(s) => s.len(),
            Content::Parts(parts) => parts
                .iter()
                .map(|p| match p {
                    Part::Text { text, .. } => text.len() + 24,
                    // Base64 size is not an image model's token cost. Keep a
                    // conservative fixed allowance so a normal attachment
                    // cannot trigger text-context compaction before first use.
                    Part::Image { .. } => 8_192,
                })
                .sum(),
        };
        for tc in &msg.tool_calls {
            n += tc.id.len() + tc.function.name.len() + tc.function.arguments.len() + 48;
        }
        n += msg.tool_call_id.as_deref().map_or(0, |s| s.len() + 20);
        n += msg.tool_name.as_deref().map_or(0, |s| s.len() + 16);
        n += msg.reasoning.as_deref().map_or(0, |s| s.len() + 16);
        n += msg.model_name.as_deref().map_or(0, |s| s.len() + 18);
        n / 4 + 4
    }
    pub fn total_tokens(&self) -> usize {
        self.messages.iter().map(Self::estimated_tokens).sum()
    }

    /// Replace every `Part::Image` in the message with a text tombstone. Old
    /// images cost a fixed 8K-token allowance each (`estimated_tokens`); the
    /// original data URL survives in the archive.
    fn age_images(m: &mut Message, tombstone: &str) {
        if let Content::Parts(parts) = &mut m.content {
            for p in parts.iter_mut() {
                if matches!(p, Part::Image { .. }) {
                    *p = Part::Text {
                        kind: "text".into(),
                        text: tombstone.into(),
                    };
                }
            }
        }
    }

    /// Fold turns older than the last `retain_turns`: tool outputs become the
    /// tombstone, oversized assistant text/reasoning is head/tail-cut, images
    /// become text notes. System messages are always kept.
    fn fold_old_turns(&mut self, retain_turns: usize, tombstone: &str) -> bool {
        let systems: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .cloned()
            .collect();
        let turns = self.split_turns();
        if turns.len() <= retain_turns {
            return false;
        }
        let old = &turns[..turns.len() - retain_turns];
        let recent = &turns[turns.len() - retain_turns..];
        let mut compacted: Vec<Message> = vec![];
        for turn in old {
            for mut m in turn.clone() {
                Self::age_images(&mut m, tombstone);
                if m.role == Role::Tool {
                    let content = m.content.as_text();
                    if !content.is_empty() && !content.starts_with(TOMBSTONE_PREFIX) {
                        m.content = wisp_llm::Content::text(tombstone);
                    }
                } else if m.role == Role::Assistant {
                    let content = m.content.as_text();
                    if (content.len() / 4 + 4) > OLD_ASSISTANT_MAX_TOKENS {
                        m.content = wisp_llm::Content::text(Self::compact_text(
                            &content,
                            OLD_ASSISTANT_KEEP.0,
                            OLD_ASSISTANT_KEEP.1,
                        ));
                    }
                    if let Some(r) = m.reasoning.clone() {
                        if (r.len() / 4 + 4) > OLD_REASONING_MAX_TOKENS {
                            m.reasoning = Some(Self::compact_text(
                                &r,
                                OLD_REASONING_KEEP.0,
                                OLD_REASONING_KEEP.1,
                            ));
                        }
                    }
                }
                compacted.push(m);
            }
        }
        for turn in recent {
            compacted.extend(turn.clone());
        }
        self.messages = systems.into_iter().chain(compacted).collect();
        true
    }

    fn compact_conversation(&mut self, retain_turns: usize) {
        let systems: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .cloned()
            .collect();
        let turns = self.split_turns();
        if turns.is_empty() {
            return;
        }
        let mut rebuilt: Vec<Message> = systems.clone();
        for turn in &turns {
            rebuilt.extend(turn.clone());
        }
        if rebuilt.iter().map(Self::estimated_tokens).sum::<usize>() <= self.warn_threshold {
            self.messages = rebuilt;
            return;
        }
        let n_old = turns.len().saturating_sub(retain_turns);
        let mut trimmed_old = turns[..n_old].to_vec();
        let recent = &turns[n_old..];
        while !trimmed_old.is_empty() {
            let mut candidate = systems.clone();
            for turn in &trimmed_old {
                candidate.extend(turn.clone());
            }
            for turn in recent {
                candidate.extend(turn.clone());
            }
            if candidate.iter().map(Self::estimated_tokens).sum::<usize>() <= self.warn_threshold {
                self.messages = candidate;
                return;
            }
            trimmed_old.remove(0);
        }
        let mut trimmed_recent = recent.to_vec();
        while trimmed_recent.len() > 1 {
            let mut candidate = systems.clone();
            for turn in &trimmed_recent {
                candidate.extend(turn.clone());
            }
            if candidate.iter().map(Self::estimated_tokens).sum::<usize>() <= self.warn_threshold {
                self.messages = candidate;
                return;
            }
            trimmed_recent.remove(0);
        }
        self.messages = systems
            .into_iter()
            .chain(trimmed_recent.into_iter().flatten())
            .collect();
    }

    async fn full_compact(
        &mut self,
        provider: &dyn Provider,
        archive_note: &str,
    ) -> Result<String, String> {
        const PROMPT: &str = "\
Create a detailed summary of the conversation so far. Focus on: user's original intent, \
files modified with key code snippets, errors encountered and their fixes, and the current \
work in progress. Use this structure:
1. Primary Request and Intent
2. Key Technical Concepts
3. Files and Code Sections (most recent first)
4. Errors and fixes
5. Problem Solving
6. All user messages
7. Pending Tasks
8. Current Work";
        self.append_user(PROMPT);
        let comp = provider
            .complete(&self.messages, &[])
            .await
            .map_err(|e| format!("full compact err: {e}"))?;
        let summary = comp.content;
        if summary.trim().is_empty() {
            return Err("full compact err: llm respon null".into());
        }
        let systems: Vec<Message> = self
            .messages
            .iter()
            .filter(|m| m.role == Role::System)
            .cloned()
            .collect();
        self.messages = systems;
        self.append_user(format!("{summary}\n\n{archive_note}"));
        Ok(summary)
    }

    /// User-triggered `/compact`. Archives the FULL history to `archive_path`
    /// first — the tombstones and the summary all name that file, so anything
    /// folded away stays retrievable via read/grep — then folds old turns, and
    /// only while still over the warning threshold escalates to dropping the
    /// oldest turns and finally an LLM summary. Returns (before, after)
    /// estimated tokens.
    pub async fn compact(
        &mut self,
        provider: &dyn Provider,
        archive_path: &Path,
    ) -> Result<(usize, usize), String> {
        let before = self.total_tokens();
        self.save(archive_path);
        if !archive_path.is_file() {
            // Never fold anything we failed to archive — retrievability is the
            // contract of /compact.
            return Err(format!(
                "compact aborted: could not write archive {}",
                archive_path.display()
            ));
        }
        let tombstone = format!(
            "{TOMBSTONE_PREFIX} full content archived at {} — read/grep that file to retrieve it]",
            archive_path.display()
        );
        let archive_note = format!(
            "[The pre-compact conversation history is archived at {} — read/grep that file to retrieve details.]",
            archive_path.display()
        );
        self.fold_old_turns(RETAIN_TURNS, &tombstone);
        if !self.under_threshold() {
            self.compact_conversation(RETAIN_TURNS_HARD);
        }
        if !self.under_threshold() {
            self.full_compact(provider, &archive_note)
                .await
                .map_err(|e| {
                    format!(
                        "folded to ~{} tokens, but the LLM summary step failed: {e}",
                        self.total_tokens()
                    )
                })?;
        }
        self.warned = false;
        Ok((before, self.total_tokens()))
    }

    /// Return the messages to send to the model (persisted + runtime
    /// injections). NEVER rewrites history — compaction is user-triggered only
    /// — so every message serializes byte-identically round after round and
    /// provider prefix caches stay valid. Crossing the warning threshold fires
    /// `Output::context_warning` once; it re-arms after a `/compact` (or
    /// anything else) brings the estimate back under.
    pub fn prepare_for_api(&mut self, output: &dyn Output) -> std::borrow::Cow<'_, [Message]> {
        let total = self.total_tokens();
        if total < self.warn_threshold {
            self.warned = false;
        } else if !self.warned {
            self.warned = true;
            output.context_warning(total, self.max_context);
        }
        if self.runtime_injections.is_empty() {
            std::borrow::Cow::Borrowed(&self.messages)
        } else {
            let mut out = self.messages.clone();
            out.extend(self.runtime_injections.iter().cloned());
            std::borrow::Cow::Owned(out)
        }
    }
}

/// A minimal JSON helper for tool-result content when carrying an image.
pub fn image_content(label: &str, data_url: &str) -> wisp_llm::Content {
    wisp_llm::Content::Parts(vec![
        wisp_llm::Part::Text {
            kind: "text".into(),
            text: label.into(),
        },
        wisp_llm::Part::Image {
            kind: "image_url".into(),
            image_url: wisp_llm::ImageUrl {
                url: data_url.into(),
            },
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    // #45: with panic=abort, a mid-UTF-8 slice during compaction crashes the
    // whole app ("闪退"). compact_text must snap its byte budgets to char
    // boundaries so multi-byte (e.g. CJK) text never slices mid-character.
    #[test]
    fn compact_text_snaps_multibyte_to_char_boundary() {
        // All-CJK text: byte 350 lands inside a 3-byte char, so `&t[..350]`
        // would panic ("byte index 350 is not a char boundary").
        let cn = "分析进度：我们已经完成了数据清洗、比对和初步统计。".repeat(40);
        assert!(
            cn.len() > 700 && !cn.is_char_boundary(350),
            "premise: 350 is mid-char"
        );
        let out = ContextManager::compact_text(&cn, 350, 350);
        assert!(out.contains("\n...\n"), "long input should be truncated");
        assert!(out.starts_with("分析进度"), "head kept and char-aligned");
        assert!(out.ends_with('。'), "tail kept and char-aligned");
    }

    // Short text (<= head + tail) is returned intact, still no mid-char slicing.
    #[test]
    fn compact_text_keeps_short_multibyte_intact() {
        let s = "简短中文";
        assert_eq!(ContextManager::compact_text(s, 350, 350), s);
    }

    // The field-length token estimate replaced a serialize-to-measure version;
    // compaction thresholds only need it to stay in the same ballpark.
    #[test]
    fn estimated_tokens_tracks_json_length() {
        let mut m = Message::user("hello world, this is a normal chat message about data analysis");
        m.tool_calls = vec![ToolCall {
            id: "call_1".into(),
            kind: "function".into(),
            function: wisp_llm::FunctionCall {
                name: "read".into(),
                arguments: r#"{"path":"/tmp/some/file.csv","limit":200}"#.into(),
            },
        }];
        let est = ContextManager::estimated_tokens(&m);
        let json = serde_json::to_string(&m).unwrap().len() / 4 + 4;
        assert!(
            est >= json / 2 && est <= json * 2,
            "estimate {est} should be within 2x of json-based {json}"
        );
    }

    use async_trait::async_trait;
    use wisp_llm::{Completion, ToolSchema};

    /// Provider stub for /compact tests. Panics if the LLM-summary step is
    /// reached when a test expects folding alone to suffice.
    struct StubProvider {
        allow_summary: bool,
    }

    #[async_trait]
    impl Provider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }
        fn model(&self) -> &str {
            "stub"
        }
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolSchema],
        ) -> wisp_llm::Result<Completion> {
            assert!(self.allow_summary, "full_compact must not run in this test");
            Ok(Completion {
                content: "summary".into(),
                ..Completion::default()
            })
        }
        async fn stream(
            &self,
            messages: &[Message],
            tools: &[ToolSchema],
            _sink: &mut dyn wisp_llm::StreamSink,
        ) -> wisp_llm::Result<Completion> {
            self.complete(messages, tools).await
        }
    }

    fn archive_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("wisp-compact-tests-{}", std::process::id()))
            .join(name)
    }

    fn seed_turns(ctx: &mut ContextManager, n: usize) {
        for i in 0..n {
            ctx.append_user(format!("question {i}"));
            ctx.append_assistant(format!("answer {i}"), vec![], None);
            ctx.append_tool(
                format!("call{i}"),
                "shell",
                Content::text(format!("tool-output-{i} {}", "x".repeat(50))),
            );
        }
    }

    // /compact archives the full history first, then tombstones old tool
    // outputs (naming the archive so the model can read/grep it back) and
    // replaces old images with text notes, while recent turns stay verbatim.
    #[tokio::test]
    async fn compact_archives_then_tombstones_old_turns_and_ages_images() {
        let mut ctx = ContextManager::new(1_000_000);
        ctx.append_system("sys");
        ctx.append_user_content(image_content("old plot", "data:image/png;base64,AAAA"));
        ctx.append_assistant("looked at the old plot".into(), vec![], None);
        seed_turns(&mut ctx, 11);
        ctx.append_user_content(image_content("new plot", "data:image/png;base64,BBBB"));

        let archive = archive_path("tombstones.json");
        let provider = StubProvider {
            allow_summary: false,
        };
        let (before, after) = ctx.compact(&provider, &archive).await.unwrap();
        assert!(before > after, "folding must shrink the estimate");

        let archived = std::fs::read_to_string(&archive).unwrap();
        assert!(
            archived.contains("tool-output-0"),
            "archive keeps originals"
        );
        assert!(archived.contains("base64,AAAA"), "archive keeps image data");

        // Turn 1 (old): image gone, replaced by a tombstone naming the archive.
        let old_user = &ctx.messages[1];
        let Content::Parts(parts) = &old_user.content else {
            panic!("old user message should stay multipart");
        };
        assert!(!parts.iter().any(|p| matches!(p, Part::Image { .. })));
        assert!(old_user.content.as_text().contains(TOMBSTONE_PREFIX));

        // Old tool output → tombstone with the archive path; recent one intact.
        let tools: Vec<&Message> = ctx
            .messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .collect();
        let first = tools.first().unwrap().content.as_text();
        assert!(first.starts_with(TOMBSTONE_PREFIX), "old tool tombstoned");
        assert!(first.contains(&archive.display().to_string()));
        let last = tools.last().unwrap().content.as_text();
        assert!(last.contains("tool-output-10"), "recent tool kept verbatim");

        // The newest image survives untouched.
        let new_user = ctx.messages.last().unwrap();
        let Content::Parts(parts) = &new_user.content else {
            panic!("new user message should stay multipart");
        };
        assert!(parts.iter().any(|p| matches!(p, Part::Image { .. })));
    }

    // A second /compact must not overwrite existing tombstones: they point at
    // the only archive that still holds the original content.
    #[tokio::test]
    async fn compact_never_repoints_existing_tombstones() {
        let mut ctx = ContextManager::new(1_000_000);
        ctx.append_user("first question".to_string());
        ctx.append_assistant("a".into(), vec![], None);
        ctx.append_tool(
            "call0",
            "shell",
            Content::text(format!(
                "{TOMBSTONE_PREFIX} full content archived at FIRST]"
            )),
        );
        seed_turns(&mut ctx, 11);

        let archive = archive_path("second.json");
        let provider = StubProvider {
            allow_summary: false,
        };
        ctx.compact(&provider, &archive).await.unwrap();
        let first_tool = ctx
            .messages
            .iter()
            .find(|m| m.role == Role::Tool)
            .unwrap()
            .content
            .as_text();
        assert!(
            first_tool.contains("FIRST"),
            "old tombstone must keep its original archive path, got: {first_tool}"
        );
    }

    struct WarnCounter(std::sync::atomic::AtomicUsize);
    impl Output for WarnCounter {
        fn context_warning(&self, _ctx_tokens: usize, _max_context: usize) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    // The agent never compacts on its own: crossing the threshold only warns,
    // and warns exactly once until the context drops back under and re-crosses.
    #[test]
    fn prepare_for_api_warns_once_per_crossing_and_never_rewrites() {
        let counter = WarnCounter(std::sync::atomic::AtomicUsize::new(0));
        let mut ctx = ContextManager::new(1_000);
        ctx.append_user("x".repeat(4_000));
        let before = ctx.messages.clone();

        ctx.prepare_for_api(&counter);
        ctx.prepare_for_api(&counter);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(
            serde_json::to_string(&ctx.messages).unwrap(),
            serde_json::to_string(&before).unwrap(),
            "history must never be rewritten automatically"
        );

        ctx.clear();
        ctx.prepare_for_api(&counter); // under threshold — re-arms the warning
        ctx.append_user("y".repeat(4_000));
        ctx.prepare_for_api(&counter);
        assert_eq!(counter.0.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[test]
    fn estimated_tokens_do_not_count_base64_bytes_as_text() {
        let message = Message {
            role: Role::User,
            content: image_content(
                "plot",
                &format!("data:image/png;base64,{}", "a".repeat(1_000_000)),
            ),
            tool_calls: vec![],
            tool_call_id: None,
            tool_name: None,
            reasoning: None,
            ts: 0,
            model_name: None,
        };

        assert!(ContextManager::estimated_tokens(&message) < 3_000);
    }
}
