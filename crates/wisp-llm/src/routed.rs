//! Smart provider routing — pick a low/medium/high tier per turn from the
//! last user message's complexity, ported from mangopi-cli's `RoutedProvider`
//! keyword score (the LLM-score arm is deferred to post-MVP).
//!
//! Wrap three [`Provider`]s and [`RoutedProvider`] delegates each call to the
//! chosen tier. A missing tier falls back to `medium`, then to whichever tier
//! exists.

use crate::message::{Message, Role};
use crate::provider::{Provider, StreamSink};
use crate::{Completion, ToolSchema};

pub struct RoutedProvider {
    low: Option<Box<dyn Provider>>,
    medium: Option<Box<dyn Provider>>,
    high: Option<Box<dyn Provider>>,
    low_max: i64,
    medium_max: i64,
}

impl RoutedProvider {
    pub fn new(
        low: Option<Box<dyn Provider>>,
        medium: Option<Box<dyn Provider>>,
        high: Option<Box<dyn Provider>>,
    ) -> Self {
        Self {
            low,
            medium,
            high,
            low_max: 3,
            medium_max: 7,
        }
    }

    fn last_user(messages: &[Message]) -> String {
        messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.as_text())
            .unwrap_or_default()
    }

    /// Keyword complexity score (1-10). Mirrors mangopi's `_FRAMEWORK_SCORE`.
    fn score(query: &str) -> i64 {
        let q = query.to_ascii_lowercase();
        // Frustration -> always high.
        for kw in ["fuck", "shit", "damn", "sb", "垃圾", "脑残", "卧槽", "tmd"] {
            if q.contains(kw) {
                return 10;
            }
        }
        let match_any = |kws: &[&str]| kws.iter().any(|k| q.contains(k));
        if match_any(&[
            "design",
            "架构",
            "architect",
            "选型",
            "refactor",
            "重构",
            "migrat",
            "迁移",
            "distribut",
            "分布式",
        ]) {
            return 9;
        }
        if match_any(&["reevaluate", "重新评估", "stuck", "卡住"]) {
            return 8;
        }
        if match_any(&[
            "implement",
            "实现",
            "build",
            "开发",
            "integrat",
            "集成",
            "feature",
            "api",
            "接口",
        ]) {
            return 5;
        }
        if match_any(&["optimize", "优化", "performance", "性能", "加速"]) {
            return 5;
        }
        if match_any(&[
            "debug", "报错", "bug", "error", "失败", "fail", "fix", "修复", "crash", "崩溃",
        ]) {
            return 3;
        }
        if match_any(&["investigate", "排查", "verify", "验证", "test", "测试"]) {
            return 3;
        }
        if match_any(&[
            "explain",
            "解释",
            "什么是",
            "区别",
            "原理",
            "describe",
            "describe",
        ]) {
            return 1;
        }
        4 // default -> medium
    }

    fn pick(&self, query: &str) -> &dyn Provider {
        let s = Self::score(query);
        let tier = if s <= self.low_max {
            "low"
        } else if s > self.medium_max {
            "high"
        } else {
            "medium"
        };
        let chosen = match tier {
            "low" => self.low.as_deref(),
            "high" => self.high.as_deref(),
            _ => self.medium.as_deref(),
        };
        chosen
            .or(self.medium.as_deref())
            .or(self.high.as_deref())
            .or(self.low.as_deref())
            .expect("routed provider has no tiers")
    }
}

#[async_trait::async_trait]
impl Provider for RoutedProvider {
    fn name(&self) -> &str {
        "routed"
    }
    fn model(&self) -> &str {
        "routed"
    }
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
    ) -> crate::provider::Result<Completion> {
        let q = Self::last_user(messages);
        let p = self.pick(&q);
        p.complete(messages, tools).await
    }
    async fn stream(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        sink: &mut dyn StreamSink,
    ) -> crate::provider::Result<Completion> {
        let q = Self::last_user(messages);
        let p = self.pick(&q);
        p.stream(messages, tools, sink).await
    }
}
