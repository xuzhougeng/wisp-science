//! Conversation context with three-tier compaction, ported from mangopi-cli's
//! `ContextManager`.
//!
//! Tiers (auto, fires before each model call once context exceeds 80% of
//! `max_context`):
//! 1. `micro_compact` — head/tail-truncate stale, oversized tool outputs.
//! 2. `session_memory_compact` — drop old turns' tool output, keep last 10.
//! 3. `compact_conversation` — drop oldest turns while still over budget.
//! 4. `full_compact` — LLM-driven summary (last resort).

use crate::output::Output;
use std::path::Path;
use wisp_llm::{Message, Provider, Role, ToolCall};

struct CompactRule {
    max_tokens: usize,
    keep_head: usize,
    keep_tail: usize,
    max_age: i64,
}

fn rules() -> (CompactRule, CompactRule, CompactRule) {
    let tool = CompactRule {
        max_tokens: 800,
        keep_head: 200,
        keep_tail: 200,
        max_age: 21_600,
    };
    let reasoning = CompactRule {
        max_tokens: 500,
        keep_head: 125,
        keep_tail: 125,
        max_age: 7_200,
    };
    let assistant = CompactRule {
        max_tokens: 1500,
        keep_head: 350,
        keep_tail: 350,
        max_age: 10_800,
    };
    (tool, reasoning, assistant)
}

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
    auto_compact_threshold: usize,
    auto_compact_disabled: bool,
    continuous_failures: u32,
    max_failures: u32,
    pub runtime_injections: Vec<Message>,
    white_tool_list: Vec<String>,
}

impl ContextManager {
    pub fn new(max_context: usize) -> Self {
        Self {
            messages: vec![],
            max_context,
            auto_compact_threshold: (max_context as f64 * 0.8) as usize,
            auto_compact_disabled: false,
            continuous_failures: 0,
            max_failures: 3,
            runtime_injections: vec![],
            white_tool_list: vec!["attempt_completion".into()],
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
    pub fn disable_auto_compact(&mut self) {
        self.auto_compact_disabled = true;
    }

    pub fn append_system(&mut self, content: impl Into<String>) {
        self.messages.push(Message::system(content));
    }
    pub fn append_user(&mut self, content: impl Into<String>) {
        self.messages.push(Message::user(content));
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
        self.total_tokens() < self.auto_compact_threshold
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

    pub fn detect_loop(&self, threshold: usize) -> (bool, Option<String>) {
        let recent = self.role_msgs(Role::Tool, Some(20));
        if recent.len() < 10 {
            return (false, None);
        }
        let mut last_tool: Option<&str> = None;
        let mut fail_streak = 0;
        for m in &recent {
            let tool = m.tool_name.as_deref().unwrap_or("");
            let content = m.content.as_text().to_ascii_lowercase();
            if Some(tool) == last_tool && (content.contains("fail") || content.contains("error")) {
                fail_streak += 1;
            } else {
                if Some(tool) != last_tool {
                    fail_streak = 0;
                }
            }
            last_tool = Some(tool);
            if fail_streak >= threshold {
                return (true, Some(tool.to_string()));
            }
        }
        let tail = &recent[recent.len().saturating_sub(12)..];
        let mut fail_set = std::collections::HashSet::new();
        let mut fail_count = 0;
        for m in tail {
            let c = m.content.as_text().to_ascii_lowercase();
            if c.contains("fail") || c.contains("error") {
                if let Some(n) = &m.tool_name {
                    fail_set.insert(n.clone());
                }
                fail_count += 1;
            }
        }
        if fail_set.len() >= 2 && fail_count >= (threshold * 2) as usize {
            let mut s: Vec<String> = fail_set.into_iter().collect();
            s.sort();
            return (true, Some(s.join(",")));
        }
        (false, None)
    }

    pub fn detect_phase(&self) -> String {
        let (is_looping, _) = self.detect_loop(3);
        if is_looping {
            return "stuck".into();
        }
        let all = Self::tool_names_of(&self.role_msgs(Role::Tool, None));
        if all.is_empty() {
            return "start".into();
        }
        let recent = &all[all.len().saturating_sub(5)..];
        if recent.iter().any(|t| t == "edit" || t == "write") {
            return "executing".into();
        }
        if recent
            .iter()
            .all(|t| t == "read" || t == "grep" || t == "search")
        {
            return "exploring".into();
        }
        if recent.iter().filter(|t| t == &"shell").count() >= 2 {
            return "verifying".into();
        }
        "executing".into()
    }

    pub fn estimated_tokens(msg: &Message) -> usize {
        let s = serde_json::to_string(msg).unwrap_or_default();
        s.len() / 4 + 4
    }
    pub fn total_tokens(&self) -> usize {
        self.messages.iter().map(Self::estimated_tokens).sum()
    }

    fn micro_compact(&mut self) {
        let (tool_rule, _, _) = rules();
        let now = chrono::Utc::now().timestamp();
        for m in &mut self.messages {
            if m.role != Role::Tool {
                continue;
            }
            let name = m.tool_name.as_deref().unwrap_or("");
            if self.white_tool_list.iter().any(|w| w == name) {
                continue;
            }
            let content = m.content.as_text();
            if content.is_empty() || content.ends_with(' ') {
                continue;
            }
            if now - m.ts < tool_rule.max_age {
                continue;
            }
            if content.len() / 4 + 4 <= tool_rule.max_tokens {
                continue;
            }
            m.content = wisp_llm::Content::text(Self::compact_text(
                &content,
                tool_rule.keep_head,
                tool_rule.keep_tail,
            ));
        }
    }

    fn session_memory_compact(&mut self, retain_turns: usize) -> bool {
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
        let (_, reasoning_rule, assistant_rule) = rules();
        let old = &turns[..turns.len() - retain_turns];
        let recent = &turns[turns.len() - retain_turns..];
        let mut compacted: Vec<Message> = vec![];
        for turn in old {
            for mut m in turn.clone() {
                if m.role == Role::Tool {
                    m.content = wisp_llm::Content::text(" ");
                } else if m.role == Role::Assistant {
                    let content = m.content.as_text();
                    if !content.is_empty()
                        && !content.ends_with(' ')
                        && (content.len() / 4 + 4) > assistant_rule.max_tokens
                    {
                        m.content = wisp_llm::Content::text(Self::compact_text(
                            &content,
                            assistant_rule.keep_head,
                            assistant_rule.keep_tail,
                        ));
                    }
                    if let Some(r) = m.reasoning.clone() {
                        if !r.is_empty()
                            && !r.ends_with(' ')
                            && (r.len() / 4 + 4) > reasoning_rule.max_tokens
                        {
                            m.reasoning = Some(Self::compact_text(
                                &r,
                                reasoning_rule.keep_head,
                                reasoning_rule.keep_tail,
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
        if rebuilt.iter().map(Self::estimated_tokens).sum::<usize>() <= self.auto_compact_threshold
        {
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
            if candidate.iter().map(Self::estimated_tokens).sum::<usize>()
                <= self.auto_compact_threshold
            {
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
            if candidate.iter().map(Self::estimated_tokens).sum::<usize>()
                <= self.auto_compact_threshold
            {
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

    async fn full_compact(&mut self, provider: &dyn Provider) -> Result<String, String> {
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
        let messages = self.messages.clone();
        let comp = provider
            .complete(&messages, &[])
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
        self.append_user(summary.clone());
        Ok(summary)
    }

    async fn auto_compact_if_needed(&mut self, provider: &dyn Provider, _output: &dyn Output) {
        if self.auto_compact_disabled || self.under_threshold() {
            return;
        }
        self.session_memory_compact(10);
        if self.under_threshold() {
            return;
        }
        self.compact_conversation(8);
        if self.under_threshold() {
            return;
        }
        if self.continuous_failures >= self.max_failures {
            return;
        }
        match self.full_compact(provider).await {
            Ok(_) => self.continuous_failures = 0,
            Err(e) => {
                self.continuous_failures += 1;
                tracing::warn!("{e}");
            }
        }
    }

    /// micro-compact, then auto-compact if over threshold; return the messages
    /// to send to the model (persisted + runtime injections).
    pub async fn prepare_for_api(
        &mut self,
        provider: &dyn Provider,
        output: &dyn Output,
    ) -> Vec<Message> {
        self.micro_compact();
        let before = self.total_tokens();
        self.auto_compact_if_needed(provider, output).await;
        let after = self.total_tokens();
        if before > after {
            output.compaction(before, after, "auto");
        }
        let mut out = self.messages.clone();
        out.extend(self.runtime_injections.clone());
        out
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
}
