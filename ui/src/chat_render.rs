use crate::acp::PlanDecision;
use crate::app_support::*;
use crate::bindings::{invoke, invoke_checked, schedule_run_output_follow};
use crate::dto::*;
use crate::i18n::{self, t, tf, use_locale, Locale};
use crate::research;
use crate::text::{event_target_value, format_duration_ms, md_to_html, tool_card_label};
use leptos::*;
use serde_wasm_bindgen::to_value;
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// True for items whose `render_item` produces an empty view, so the thread
/// loop can drop their wrapper `<div>` and avoid a dangling `.thread` gap (#19).
pub(crate) fn renders_nothing(item: &ChatItem) -> bool {
    matches!(item, ChatItem::Assistant { text, .. } if text.trim().is_empty())
        || matches!(item, ChatItem::Tool { name, .. } if name == "attempt_completion")
        || matches!(item, ChatItem::FileChanged(_))
        || matches!(item, ChatItem::QueuedUser { .. })
}

pub(crate) fn class_for(item: &ChatItem) -> &'static str {
    match item {
        ChatItem::User(_) => "msg user",
        ChatItem::QueuedUser { .. } => "msg user queued",
        ChatItem::Assistant { text, .. } if text.starts_with("Error: ") => "tool-wrap",
        ChatItem::Assistant { .. } => "msg assistant",
        ChatItem::BranchMerge { .. } => "branch-merge-card-row",
        ChatItem::Reasoning(_) => "msg reasoning",
        ChatItem::Tool { name, .. } if is_run_monitor_tool(name) => "tool-wrap run-monitor-wrap",
        ChatItem::Tool { name, .. } if is_image_generation_tool(name) => {
            "tool-wrap image-generation-wrap"
        }
        ChatItem::Tool { name, .. } if is_video_generation_tool(name) => {
            "tool-wrap video-generation-wrap"
        }
        ChatItem::Tool { .. } => "tool-wrap",
        ChatItem::FileChanged(_) => "artifact-write-marker",
        ChatItem::ApprovalPending { .. } => "tool-wrap approval-wrap-row",
        ChatItem::AcpPermission { .. } => "tool-wrap approval-wrap-row",
        ChatItem::AcpTool { .. } => "tool-wrap",
        ChatItem::Usage { .. } => "usage-row",
        ChatItem::Compaction { .. } => "context-compaction-row",
        ChatItem::ReviewTransition { .. } => "review-transition-row",
        ChatItem::Review(_) => "tool-wrap",
        ChatItem::Plan(_) => "tool-wrap plan-wrap",
        ChatItem::Question(_) => "tool-wrap plan-question-wrap",
    }
}

/// "482" below 1k, "12.3k" above — same scale the status bar uses.
pub(crate) fn fmt_tokens(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else {
        format!("{:.1}k", n as f64 / 1000.0)
    }
}

pub(crate) fn context_percent(used: usize, max: usize) -> usize {
    if max == 0 {
        0
    } else {
        ((((used as u128) * 100 + (max as u128 / 2)) / max as u128) as usize).min(100)
    }
}

/// Default bands from #931: under 70% is idle, 70–90% is a warning, above
/// 90% is danger. A missing window must not look like 0%.
pub(crate) const CONTEXT_USAGE_WARN_PCT: usize = 70;
pub(crate) const CONTEXT_USAGE_DANGER_PCT: usize = 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextUsageTone {
    Unknown,
    Ok,
    Warn,
    Danger,
}

impl ContextUsageTone {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Danger => "danger",
        }
    }
}

pub(crate) fn context_usage_tone(used: usize, max: usize) -> ContextUsageTone {
    if max == 0 {
        return ContextUsageTone::Unknown;
    }
    let pct = context_percent(used, max);
    if pct > CONTEXT_USAGE_DANGER_PCT {
        ContextUsageTone::Danger
    } else if pct >= CONTEXT_USAGE_WARN_PCT {
        ContextUsageTone::Warn
    } else {
        ContextUsageTone::Ok
    }
}

pub(crate) fn context_usage_percent_label(used: usize, max: usize) -> String {
    if max == 0 {
        "—".into()
    } else {
        format!("{}%", context_percent(used, max))
    }
}

pub(crate) fn context_usage_tooltip(snapshot: &ContextUsageSnapshot, locale: Locale) -> String {
    if snapshot.max == 0 {
        return t(locale, "context_usage.tooltip_unknown").into();
    }
    let used = fmt_context_tokens(snapshot.used);
    let max = fmt_context_limit(snapshot.max);
    tf(
        locale,
        if snapshot.estimated {
            "context_usage.tooltip_estimated"
        } else {
            "context_usage.tooltip_exact"
        },
        &[("used", &used), ("max", &max)],
    )
}

pub(crate) fn fmt_context_tokens(tokens: usize) -> String {
    if tokens < 1_000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    }
}

pub(crate) fn fmt_context_limit(tokens: usize) -> String {
    if tokens >= 1_000_000 && tokens % 1_000_000 == 0 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 && tokens % 1_000 == 0 {
        format!("{}K", tokens / 1_000)
    } else {
        fmt_context_tokens(tokens)
    }
}

#[derive(Clone)]
pub(crate) struct ContextUsageRow {
    pub(crate) label: String,
    pub(crate) tokens: usize,
    pub(crate) color: &'static str,
}

pub(crate) fn context_usage_rows(
    snapshot: &ContextUsageSnapshot,
    locale: Locale,
) -> Vec<ContextUsageRow> {
    let Some(usage) = snapshot.breakdown else {
        // ACP only reports used/max; do not invent native category splits.
        return vec![ContextUsageRow {
            label: t(locale, "context_usage.remote_context").into(),
            tokens: snapshot.used,
            color: "conversation",
        }];
    };
    [
        ("context_usage.system_prompt", usage.system_prompt, "system"),
        (
            "context_usage.tool_definitions",
            usage.tool_definitions,
            "tools",
        ),
        ("context_usage.rules", usage.rules, "rules"),
        ("context_usage.skills", usage.skills, "skills"),
        (
            "context_usage.mcp_dynamic_tools",
            usage.mcp_dynamic_tools,
            "dynamic",
        ),
        (
            "context_usage.subagent_definitions",
            usage.subagent_definitions,
            "subagents",
        ),
        (
            "context_usage.conversation",
            usage.conversation,
            "conversation",
        ),
    ]
    .into_iter()
    .map(|(key, tokens, color)| ContextUsageRow {
        label: t(locale, key).into(),
        tokens,
        color,
    })
    .collect()
}

pub(crate) fn context_usage_detail_text(details: &ContextUsageDetails, color: &str) -> String {
    let tools = |items: &[ContextToolDetail]| {
        items
            .iter()
            .map(|item| format!("{}\n{}", item.name, item.description))
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    match color {
        "system" => details.system_prompt.clone(),
        "tools" => tools(&details.tool_definitions),
        "rules" => details.rules.clone(),
        "skills" => details.skills.clone(),
        "dynamic" => tools(&details.mcp_dynamic_tools),
        "subagents" => tools(&details.subagent_definitions),
        _ => String::new(),
    }
}

#[cfg(test)]
mod token_format_tests {
    use super::{
        context_percent, context_usage_percent_label, context_usage_rows, context_usage_tone,
        context_usage_tooltip, fmt_context_limit, fmt_context_tokens, fmt_tokens, renders_nothing,
        ContextUsageTone,
    };
    use crate::dto::{ContextUsage, ContextUsageSnapshot};
    use crate::i18n::Locale;

    #[test]
    fn small_counts_are_not_rounded_to_zero() {
        assert_eq!(fmt_tokens(81), "81");
        assert_eq!(fmt_tokens(136_286), "136.3k");
    }

    #[test]
    fn queued_turns_do_not_occupy_a_transcript_row() {
        use crate::dto::ChatItem;
        assert!(renders_nothing(&ChatItem::QueuedUser {
            id: 1,
            text: "later".into(),
        }));
        assert!(!renders_nothing(&ChatItem::User("sent".into())));
    }

    #[test]
    fn context_counts_match_the_usage_panel_format() {
        assert_eq!(context_percent(79_900, 300_000), 27);
        assert_eq!(fmt_context_tokens(6_000), "6.0K");
        assert_eq!(fmt_context_tokens(79_900), "79.9K");
        assert_eq!(fmt_context_limit(300_000), "300K");
    }

    #[test]
    fn context_usage_tone_uses_warn_and_danger_bands() {
        assert_eq!(context_usage_tone(0, 0), ContextUsageTone::Unknown);
        assert_eq!(context_usage_tone(69, 100), ContextUsageTone::Ok);
        assert_eq!(context_usage_tone(70, 100), ContextUsageTone::Warn);
        assert_eq!(context_usage_tone(90, 100), ContextUsageTone::Warn);
        assert_eq!(context_usage_tone(91, 100), ContextUsageTone::Danger);
        assert_eq!(context_usage_percent_label(0, 0), "—");
        assert_eq!(context_usage_percent_label(79_900, 128_000), "62%");
        let tooltip = context_usage_tooltip(
            &ContextUsageSnapshot {
                used: 79_900,
                max: 128_000,
                breakdown: None,
                estimated: true,
            },
            Locale::En,
        );
        assert!(tooltip.contains("79.9K"));
        assert!(tooltip.contains("128K"));
        assert_eq!(
            context_usage_tooltip(
                &ContextUsageSnapshot {
                    used: 1_200,
                    max: 0,
                    breakdown: None,
                    estimated: false,
                },
                Locale::En,
            ),
            "Context window unknown for this model"
        );
    }

    #[test]
    fn acp_totals_keep_a_single_remote_row() {
        let rows = context_usage_rows(
            &ContextUsageSnapshot {
                used: 1_200,
                max: 8_000,
                breakdown: None,
                estimated: false,
            },
            Locale::En,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "Agent-reported total");
        assert_eq!(rows[0].tokens, 1_200);
    }

    #[test]
    fn categorized_native_usage_keeps_seven_rows() {
        let rows = context_usage_rows(
            &ContextUsageSnapshot {
                used: 79_900,
                max: 300_000,
                breakdown: Some(ContextUsage {
                    system_prompt: 6_000,
                    tool_definitions: 22_700,
                    rules: 2_200,
                    skills: 6_100,
                    mcp_dynamic_tools: 4_200,
                    subagent_definitions: 2_400,
                    conversation: 36_300,
                }),
                estimated: true,
            },
            Locale::En,
        );
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[6].label, "Conversation");
        assert_eq!(rows[6].tokens, 36_300);
    }
}

/// One thread render unit: either a single message, or a coalesced steps panel.
///
/// Rows store message indices, not cloned messages: the keyed `<For>` rebuilds
/// a row only when its fingerprint key changes, so cloning the message lazily
/// inside `children` (via `items.with_untracked`) turns per-flush deep clones
/// of the whole render window into one clone per actually-changed row.
#[derive(Clone)]
pub(crate) enum ThreadRow {
    AutoRun {
        run_id: String,
    },
    Item {
        i: usize,
        timestamp: Option<i64>,
        commentary: bool,
        compact_assistant: bool,
        streaming_assistant: bool,
        streaming_reasoning: bool,
    },
    Steps {
        indices: Vec<usize>,
        live: bool,
        ui_indices: String,
    },
    Activity {
        indices: Vec<usize>,
        ui_indices: String,
        duration_ms: Option<u64>,
    },
}

const STREAMING_REASONING_MAX_BYTES: usize = 64 * 1024;

fn streaming_reasoning_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len().saturating_sub(max_bytes);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    format!("…\n{}", &text[start..])
}

/// Keep the live reasoning row mounted while its source string grows. Closed
/// details do not subscribe to or materialize the body; an opened live view is
/// bounded so repeated signal flushes cannot copy an ever-growing string.
#[component]
pub(crate) fn StreamingReasoningMessage(
    items: RwSignal<Vec<ChatItem>>,
    source_item: usize,
    session_id: String,
    disclosure_state: RwSignal<HashMap<String, bool>>,
) -> impl IntoView {
    let locale = use_locale();
    let open_id = format!("{session_id}:reasoning:{source_item}");
    let toggle_id = open_id.clone();
    let open_key = open_id.clone();
    let open = create_memo(move |_| disclosure_open(disclosure_state, &open_key, false));
    view! {
        <details class="rz" open=move || open.get()>
            <summary on:click=move |event| {
                event.prevent_default();
                toggle_disclosure(disclosure_state, &toggle_id, false);
            }>{move || t(locale.get(), "chat.thinking")}</summary>
            {move || open.get().then(|| {
                let text = items.with(|rows| match rows.get(source_item) {
                    Some(ChatItem::Reasoning(text)) => {
                        streaming_reasoning_text(text, STREAMING_REASONING_MAX_BYTES)
                    }
                    _ => String::new(),
                });
                view! { <div class="body">{text}</div> }
            })}
        </details>
    }
}

#[cfg(test)]
mod streaming_reasoning_tests {
    use super::streaming_reasoning_text;

    #[test]
    fn keeps_short_text_and_bounds_utf8_tail() {
        assert_eq!(streaming_reasoning_text("短文", 16), "短文");
        let preview = streaming_reasoning_text("甲乙丙丁", 7);
        assert_eq!(preview, "…\n丙丁");
    }
}

/// Compact, foldable summary of consecutive tool calls. Collapsed by default;
/// auto-opens while it is the live tail so progress stays visible.
///
/// Built as a manual accordion (signal + `class:open`) rather than
/// `<details>/<summary>`: the UA disclosure marker survives `list-style:none`
/// + `::-webkit-details-marker` here (WebKit and Blink alike), and there is no
/// portable way to drop it — so we don't render one.
pub(crate) fn disclosure_open(
    states: RwSignal<HashMap<String, bool>>,
    id: &str,
    automatic: bool,
) -> bool {
    states.with(|values| values.get(id).copied().unwrap_or(automatic))
}

pub(crate) fn toggle_disclosure(
    states: RwSignal<HashMap<String, bool>>,
    id: &str,
    automatic: bool,
) {
    states.update(|values| {
        let current = values.get(id).copied().unwrap_or(automatic);
        values.insert(id.to_string(), !current);
    });
}

/// Collapsed-header label for a step group. `elapsed` is the pre-formatted run
/// duration, and is only ever `Some` for a settled step-count header — the
/// running and activity-done headers show it in the meta slot instead.
pub(crate) fn steps_title(
    locale: Locale,
    completed_turn: bool,
    live: bool,
    model_wait_message: Option<&str>,
    n_tools: usize,
    elapsed: Option<&str>,
) -> String {
    match (completed_turn, live, model_wait_message, n_tools, elapsed) {
        (true, _, _, _, _) => t(locale, "chat.activity_done").to_string(),
        (_, true, Some(message), _, _) => message.to_string(),
        (_, true, None, _, _) => t(locale, "chat.steps_running").to_string(),
        (_, _, _, 1, None) => t(locale, "chat.steps_1").to_string(),
        (_, _, _, 1, Some(d)) => tf(locale, "chat.steps_1_time", &[("t", d)]),
        (_, _, _, n, None) => tf(locale, "chat.steps_n", &[("n", &n.to_string())]),
        (_, _, _, n, Some(d)) => tf(
            locale,
            "chat.steps_n_time",
            &[("n", &n.to_string()), ("t", d)],
        ),
    }
}

#[cfg(test)]
mod steps_title_tests {
    use super::{steps_title, Locale};

    #[test]
    fn folds_the_run_duration_into_settled_step_counts() {
        assert_eq!(
            steps_title(Locale::En, false, false, None, 1, None),
            "Ran 1 step"
        );
        assert_eq!(
            steps_title(Locale::En, false, false, None, 1, Some("2s")),
            "Ran 1 step · 2s"
        );
        assert_eq!(
            steps_title(Locale::Zh, false, false, None, 3, Some("1.4s")),
            "已执行 3 步 · 1.4s"
        );
    }

    #[test]
    fn running_and_done_headers_ignore_the_duration() {
        assert_eq!(
            steps_title(Locale::En, false, true, None, 3, Some("2s")),
            "Working…"
        );
        assert_eq!(
            steps_title(Locale::En, true, false, None, 3, Some("2s")),
            steps_title(Locale::En, true, false, None, 3, None)
        );
    }

    #[test]
    fn explains_when_a_completed_tool_is_waiting_on_the_model() {
        assert_eq!(
            steps_title(
                Locale::En,
                false,
                true,
                Some("The model is consulting its neurons…"),
                1,
                None
            ),
            "The model is consulting its neurons…"
        );
        assert_eq!(
            steps_title(
                Locale::Zh,
                false,
                true,
                Some("模型正在和神经元商量…"),
                1,
                None
            ),
            "模型正在和神经元商量…"
        );
    }
}

pub(crate) fn render_steps_group(
    indices: Vec<usize>,
    source: RwSignal<Vec<ChatItem>>,
    live: bool,
    completed_turn: bool,
    turn_duration_ms: Option<u64>,
    group_id: String,
    disclosure_state: RwSignal<HashMap<String, bool>>,
) -> impl IntoView {
    let locale = use_locale();
    let now = now_ms();
    let (n_tools, tool_total_ms, now_line, waiting_for_model) = source.with_untracked(|items| {
        let selected = || indices.iter().filter_map(|index| items.get(*index));
        let n_tools = selected()
            .filter(|item| matches!(item, ChatItem::Tool { .. } | ChatItem::AcpTool { .. }))
            .count();
        let total_ms = selected()
            .map(|item| match item {
                ChatItem::Tool {
                    duration_ms: Some(duration),
                    ..
                } => *duration,
                ChatItem::Tool {
                    duration_ms: None,
                    started_at_ms: Some(started),
                    ok: None,
                    ..
                } if live => now.saturating_sub(*started),
                _ => 0,
            })
            .sum::<u64>();
        let now_line = live
            .then(|| {
                indices
                    .iter()
                    .rev()
                    .filter_map(|index| items.get(*index))
                    .find_map(step_now_line)
            })
            .flatten();
        let waiting_for_model = live
            && indices
                .iter()
                .rev()
                .filter_map(|index| items.get(*index))
                .find(|item| !matches!(item, ChatItem::Usage { .. } | ChatItem::Compaction { .. }))
                .is_some_and(|item| match item {
                    ChatItem::Tool { ok: Some(_), .. } => true,
                    ChatItem::AcpTool { status, .. } => {
                        status != "pending" && status != "in_progress"
                    }
                    _ => false,
                });
        (n_tools, total_ms, now_line, waiting_for_model)
    });
    let total_ms =
        turn_duration_ms.unwrap_or_else(|| if completed_turn { 0 } else { tool_total_ms });
    let total_label =
        (total_ms > 0 && (!live || n_tools > 0)).then(|| format_duration_ms(total_ms));
    // A settled step-count header reads better with the duration inline; the
    // running and activity-done headers keep it in the right-aligned meta slot,
    // where it doubles as a ticking total.
    let inline_time = (!completed_turn && !live)
        .then(|| total_label.clone())
        .flatten();
    let meta_label = inline_time.is_none().then_some(total_label).flatten();
    let model_wait_variant = group_id
        .bytes()
        .fold(0usize, |sum, byte| sum + byte as usize)
        % 4;
    let title = move || {
        let model_wait_message = waiting_for_model.then(|| {
            t(
                locale.get(),
                match model_wait_variant {
                    0 => "chat.model_wait_1",
                    1 => "chat.model_wait_2",
                    2 => "chat.model_wait_3",
                    _ => "chat.model_wait_4",
                },
            )
        });
        steps_title(
            locale.get(),
            completed_turn,
            live,
            model_wait_message.as_deref(),
            n_tools,
            inline_time.as_deref(),
        )
    };
    let class_group_id = group_id.clone();
    let toggle_group_id = group_id.clone();
    let rows_group_id = group_id;
    let group_open = create_memo(move |_| disclosure_open(disclosure_state, &class_group_id, live));
    view! {
        <div class="steps"
            class=("activity-summary", completed_turn)
            class:open=move || group_open.get()>
            <button type="button" class="steps-head"
                aria-expanded=move || group_open.get().to_string()
                on:click=move |_| {
                toggle_disclosure(disclosure_state, &toggle_group_id, live)
            }>
                <span class="steps-chevron"></span>
                <span class="steps-title">{title}</span>
                {now_line.map(|text| view! { <span class="steps-now">{text}</span> })}
                {meta_label.map(|label| view! { <span class="steps-meta">{label}</span> })}
            </button>
            {move || group_open.get().then(|| view! {
                <div class="steps-body">{
                    render_step_rows(
                        &indices,
                        source,
                        live,
                        &rows_group_id,
                        disclosure_state,
                        locale,
                        now,
                    )
                }</div>
            })}
        </div>
    }
}

fn render_step_rows(
    indices: &[usize],
    source: RwSignal<Vec<ChatItem>>,
    live: bool,
    group_id: &str,
    disclosure_state: RwSignal<HashMap<String, bool>>,
    locale: ReadSignal<Locale>,
    now: u64,
) -> View {
    indices
        .iter()
        .copied()
        .enumerate()
        .map(|(position, index)| {
            render_step_row(
                source,
                index,
                position,
                live,
                group_id,
                disclosure_state,
                locale,
                now,
            )
        })
        .collect_view()
}

#[allow(clippy::too_many_arguments)]
fn render_step_row(
    source: RwSignal<Vec<ChatItem>>,
    index: usize,
    position: usize,
    live: bool,
    group_id: &str,
    disclosure_state: RwSignal<HashMap<String, bool>>,
    locale: ReadSignal<Locale>,
    now: u64,
) -> View {
    source.with_untracked(|items| match items.get(index) {
        Some(ChatItem::Assistant { text, .. }) => {
            let step_id = format!("{group_id}:progress:{position}");
            let toggle_id = step_id.clone();
            let detail = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .trim()
                .chars()
                .take(100)
                .collect::<String>();
            let step_open = create_memo(move |_| {
                disclosure_open(disclosure_state, &step_id, false)
            });
            view! {
                <div class="step step-progress" class:open=move || step_open.get()>
                    <button type="button" class="step-head"
                        aria-expanded=move || step_open.get().to_string()
                        on:click=move |_| toggle_disclosure(disclosure_state, &toggle_id, false)>
                        <span class="step-icon progress"></span>
                        <span class="step-name">{move || t(locale.get(), "chat.progress")}</span>
                        <span class="step-detail">{detail}</span>
                    </button>
                    {move || step_open.get().then(|| {
                        let text = source.with_untracked(|items| match items.get(index) {
                            Some(ChatItem::Assistant { text, .. }) => text.clone(),
                            _ => String::new(),
                        });
                        // A progress message is sealed as soon as the following
                        // reasoning/tool event arrives. Render that completed
                        // step as Markdown immediately even while the overall
                        // turn is still live, so Markdown constructs do not
                        // remain source text until `Done`.
                        let class = if live {
                            "step-progress-body body md streaming"
                        } else {
                            "step-progress-body body md"
                        };
                        view! { <div class=class inner_html=md_to_html(&text)></div> }.into_view()
                    })}
                </div>
            }
            .into_view()
        }
        Some(ChatItem::Reasoning(_)) => {
            let step_id = format!("{group_id}:reasoning:{position}");
            let toggle_id = step_id.clone();
            let step_open = create_memo(move |_| {
                disclosure_open(disclosure_state, &step_id, false)
            });
            view! {
                <div class="step step-think" class:open=move || step_open.get()>
                    <button type="button" class="step-head"
                        aria-expanded=move || step_open.get().to_string()
                        on:click=move |_| toggle_disclosure(disclosure_state, &toggle_id, false)>
                        <span class="step-icon think"></span>
                        <span class="step-name">{move || t(locale.get(), "chat.thinking")}</span>
                    </button>
                    {move || step_open.get().then(|| {
                        let text = source.with_untracked(|items| match items.get(index) {
                            Some(ChatItem::Reasoning(text)) => text.clone(),
                            _ => String::new(),
                        });
                        view! { <div class="step-think-body">{text}</div> }
                    })}
                </div>
            }
            .into_view()
        }
        Some(ChatItem::Tool {
            name,
            ok,
            input,
            output,
            started_at_ms,
            duration_ms,
        }) => {
            let step_id = format!("{group_id}:tool:{position}");
            let automatic = ok.is_none() && live;
            let toggle_id = step_id.clone();
            let (badge_key, title) = tool_card_label(name, input);
            let mut detail = input
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .trim()
                .chars()
                .take(80)
                .collect::<String>();
            if detail == title {
                detail.clear();
            }
            // Counting an ever-growing stdout tail per chunk is quadratic. The
            // final card still reports its settled line count.
            let lines = if ok.is_some() && !output.is_empty() {
                output.lines().count()
            } else {
                0
            };
            let has_input = !input.is_empty();
            let has_output = !output.is_empty();
            let has_body = has_input || has_output;
            let icon = match ok {
                Some(true) => view! { <span class="step-icon ok">"✓"</span> }.into_view(),
                Some(false) => view! { <span class="step-icon fail">"✗"</span> }.into_view(),
                None => view! { <span class="step-icon run"><span class="run-dot"></span></span> }.into_view(),
            };
            let meta = step_tool_meta(
                locale.get(),
                *duration_ms,
                *started_at_ms,
                *ok,
                lines,
                now,
            )
            .map(|text| view! { <span class="step-meta">{text}</span> });
            let step_open = create_memo(move |_| {
                has_body && disclosure_open(disclosure_state, &step_id, automatic)
            });
            view! {
                <div class="step" class:open=move || step_open.get() class=("no-body", !has_body)>
                    <button type="button" class="step-head" disabled=!has_body
                        aria-expanded=move || step_open.get().to_string()
                        on:click=move |_| {
                            if has_body {
                                toggle_disclosure(disclosure_state, &toggle_id, automatic)
                            }
                        }>
                        {icon}
                        {badge_key.map(|key| view! {
                            <span class="tool-badge">{move || t(locale.get(), key)}</span>
                        })}
                        <span class="step-name">{title}</span>
                        {(!detail.is_empty()).then(|| view! { <span class="step-detail">{detail}</span> })}
                        {meta}
                    </button>
                    {move || step_open.get().then(|| {
                        let (input, output) = source.with_untracked(|items| match items.get(index) {
                            Some(ChatItem::Tool { input, output, .. }) => (input.clone(), output.clone()),
                            _ => (String::new(), String::new()),
                        });
                        view! {
                            <div class="step-body">
                                {has_input.then(|| view! { <pre class="tool-input">{input}</pre> })}
                                {has_output.then(|| view! { <pre class="tool-output">{output}</pre> })}
                            </div>
                        }
                    })}
                </div>
            }
            .into_view()
        }
        Some(ChatItem::AcpTool {
            call_id,
            title,
            kind,
            status,
            content,
            locations,
            ..
        }) => {
            let failed = status == "failed";
            let done = matches!(status.as_str(), "completed" | "failed");
            let stable_part = if call_id.is_empty() {
                format!("position-{position}")
            } else {
                call_id.clone()
            };
            let step_id = format!("{group_id}:acp:{stable_part}");
            let automatic = !done && live;
            let toggle_id = step_id.clone();
            let detail = acp_tool_step_detail(kind, content, locations);
            let has_body = !locations.trim().is_empty()
                || (!content.trim().is_empty() && !acp_tool_is_terminal_stub(content));
            let icon = if failed {
                view! { <span class="step-icon fail">"✗"</span> }.into_view()
            } else if done {
                view! { <span class="step-icon ok">"✓"</span> }.into_view()
            } else {
                view! { <span class="step-icon run"><span class="run-dot"></span></span> }.into_view()
            };
            let meta = (!done).then(|| status.clone());
            let title = title.clone();
            let status = status.clone();
            let step_open = create_memo(move |_| {
                has_body && disclosure_open(disclosure_state, &step_id, automatic)
            });
            view! {
                <div class="step acp-tool" data-testid="acp-tool" data-status=status
                    class:open=move || step_open.get() class=("no-body", !has_body)>
                    <button type="button" class="step-head" disabled=!has_body
                        aria-expanded=move || step_open.get().to_string()
                        on:click=move |_| {
                            if has_body {
                                toggle_disclosure(disclosure_state, &toggle_id, automatic)
                            }
                        }>
                        {icon}
                        <span class="step-name">{title}</span>
                        {(!detail.is_empty()).then(|| view! { <span class="step-detail">{detail}</span> })}
                        {meta.map(|text| view! { <span class="step-meta">{text}</span> })}
                    </button>
                    {move || step_open.get().then(|| {
                        let body = source.with_untracked(|items| match items.get(index) {
                            Some(ChatItem::AcpTool { content, locations, .. }) => {
                                acp_tool_step_body(content, locations)
                            }
                            _ => String::new(),
                        });
                        view! { <div class="step-body"><pre class="tool-output">{body}</pre></div> }
                    })}
                </div>
            }
            .into_view()
        }
        _ => view! {}.into_view(),
    })
}

/// Latest step of a live run as "name · detail", shown in the collapsed
/// steps header so folding the panel hides detail, not progress.
#[cfg(test)]
pub(crate) fn steps_now_line(items: &[ChatItem]) -> Option<String> {
    items.iter().rev().find_map(step_now_line)
}

fn step_now_line(item: &ChatItem) -> Option<String> {
    let first_line = |s: &str| -> String {
        s.lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .chars()
            .take(80)
            .collect()
    };
    match item {
        ChatItem::Tool { name, input, .. } => {
            let (_, title) = tool_card_label(name, input);
            let detail = first_line(input);
            Some(if detail.is_empty() || detail == title {
                title
            } else {
                format!("{title} · {detail}")
            })
        }
        ChatItem::AcpTool {
            title,
            kind,
            content,
            locations,
            ..
        } => {
            let detail = acp_tool_step_detail(kind, content, locations);
            Some(if detail.is_empty() {
                title.clone()
            } else {
                format!("{title} · {detail}")
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod steps_now_line_tests {
    use super::steps_now_line;
    use crate::dto::ChatItem;

    fn tool(name: &str, input: &str) -> ChatItem {
        ChatItem::Tool {
            name: name.into(),
            ok: None,
            input: input.into(),
            output: String::new(),
            started_at_ms: None,
            duration_ms: None,
        }
    }

    #[test]
    fn shows_latest_step() {
        let items = vec![
            ChatItem::Reasoning("hmm".into()),
            tool("python", "\nfrom pypdf import PdfReader\nmore"),
        ];
        assert_eq!(
            steps_now_line(&items),
            Some("python · from pypdf import PdfReader".into())
        );
        assert_eq!(steps_now_line(&[ChatItem::Reasoning("hmm".into())]), None);
        assert_eq!(steps_now_line(&[]), None);
    }
}

pub(crate) fn acp_tool_is_terminal_stub(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.starts_with('[') && trimmed.contains("\"terminalId\"") && !trimmed.contains('\n')
}

pub(crate) fn acp_tool_step_detail(kind: &str, content: &str, locations: &str) -> String {
    let from_locations = locations
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if !from_locations.is_empty() {
        return from_locations.chars().take(80).collect();
    }
    if acp_tool_is_terminal_stub(content) || content.trim().is_empty() {
        return kind.chars().take(80).collect();
    }
    content
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .chars()
        .take(80)
        .collect()
}

pub(crate) fn acp_tool_step_body(content: &str, locations: &str) -> String {
    let mut parts = Vec::new();
    if !locations.trim().is_empty() {
        parts.push(locations.trim().to_string());
    }
    if !content.trim().is_empty() && !acp_tool_is_terminal_stub(content) {
        parts.push(content.trim().to_string());
    }
    parts.join("\n")
}

#[component]
pub(crate) fn ProvenancePane(
    items: RwSignal<Vec<ChatItem>>,
    rows: Memo<Vec<(usize, u64)>>,
) -> impl IntoView {
    let locale = use_locale();
    let expanded = create_rw_signal::<HashMap<usize, bool>>(HashMap::new());
    let has_rows = create_memo(move |_| rows.with(|rows| !rows.is_empty()));
    view! {
        {move || if !has_rows.get() {
            view! {
                <div class="rp-empty">
                    <span class="rp-empty-icon"></span>
                    <div class="rp-empty-title">{move || t(locale.get(), "right.no_tools.title")}</div>
                    <p>{move || t(locale.get(), "right.no_tools.body")}</p>
                </div>
            }
            .into_view()
        } else {
            view! {
                <div class="prov-list">
                    <For
                        each=move || rows.get()
                        key=|(index, fingerprint)| (*index, *fingerprint)
                        children=move |(index, _)| {
                            let (name, ok, has_input, has_output) = items.with_untracked(|items| {
                                match items.get(index) {
                                    Some(ChatItem::Tool { name, ok, input, output, .. }) => {
                                        (name.clone(), *ok, !input.is_empty(), !output.is_empty())
                                    }
                                    _ => (String::new(), None, false, false),
                                }
                            });
                            let automatic = ok != Some(true);
                            let row_open = create_memo(move |_| {
                                expanded.with(|rows| rows.get(&index).copied().unwrap_or(automatic))
                            });
                            view! {
                                <details class="prov-item" data-provenance-index=index.to_string()
                                    open=move || row_open.get()>
                                    <summary class="prov-head" aria-expanded=move || row_open.get().to_string()
                                        on:click=move |event| {
                                            event.prevent_default();
                                            expanded.update(|rows| {
                                                rows.insert(index, !row_open.get_untracked());
                                            });
                                        }>
                                        <span class="prov-name">{name}</span>
                                        {match ok {
                                            Some(true) => view! { <span class="ok">"✓"</span> }.into_view(),
                                            Some(false) => view! { <span class="fail">"✗"</span> }.into_view(),
                                            None => view! { <span class="run">"…"</span> }.into_view(),
                                        }}
                                    </summary>
                                    {move || row_open.get().then(|| {
                                        let (input, output) = items.with_untracked(|items| {
                                            match items.get(index) {
                                                Some(ChatItem::Tool { input, output, .. }) => {
                                                    (input.clone(), output.clone())
                                                }
                                                _ => (String::new(), String::new()),
                                            }
                                        });
                                        view! {
                                            <div class="prov-detail">
                                                {has_input.then(|| view! {
                                                    <div class="prov-label">{move || t(locale.get(), "right.input")}</div>
                                                    <pre class="prov-body">{input}</pre>
                                                })}
                                                {has_output.then(|| view! {
                                                    <div class="prov-label">{move || t(locale.get(), "right.output")}</div>
                                                    <pre class="prov-body">{output}</pre>
                                                })}
                                            </div>
                                        }
                                    })}
                                </details>
                            }
                        }
                    />
                </div>
            }
            .into_view()
        }}
    }
}

fn run_monitor_meta(
    locale: Locale,
    context_id: &str,
    kind: &str,
    started: i64,
    ended_at: Option<i64>,
    active: bool,
    last_heartbeat: Option<i64>,
    timeout_secs: Option<i64>,
    now: i64,
) -> String {
    let ended = ended_at.unwrap_or(now);
    let elapsed_value = transfer_duration(ended.saturating_sub(started) as u64);
    let elapsed = tf(locale, "runs.elapsed", &[("time", &elapsed_value)]);
    let mut meta = format!("{context_id} · {kind} · {elapsed}");
    if active {
        if let Some(last_heartbeat) = last_heartbeat {
            let age = transfer_duration(now.saturating_sub(last_heartbeat) as u64);
            meta.push_str(" · ");
            meta.push_str(&tf(locale, "runs.heartbeat", &[("time", &age)]));
        }
        if let Some(limit) = timeout_secs.filter(|seconds| *seconds > 0) {
            let limit = transfer_duration(limit as u64);
            meta.push_str(" · ");
            meta.push_str(&tf(locale, "runs.timeout", &[("time", &limit)]));
        }
    }
    meta
}

#[component]
pub(crate) fn RunMonitorCard(
    run_id: String,
    runs: RwSignal<Vec<RunSummary>>,
    clock: ReadSignal<i64>,
    tool_ok: Option<bool>,
    tool_output: String,
    dismissed_runs: RwSignal<HashSet<String>>,
    /// Only foreground `monitor_run` cards nominate their Run for the
    /// results-review prompt. AutoRun cards cover exploratory command Runs,
    /// which must never interrupt with a review modal (#897).
    #[prop(optional)]
    auto_review: bool,
) -> impl IntoView {
    let locale = use_locale();
    let fallback = serde_json::from_str::<RunRecord>(&tool_output).ok();
    let detail = create_rw_signal(fallback.clone());
    let lookup_id = run_id.clone();
    let selected_id = run_id.clone();
    let fallback_for_selection = fallback.as_ref().map(RunSummary::from);
    // Polls touch the shared run vector, but this memo only publishes when this
    // card's record changes. Unrelated run updates therefore leave its DOM and
    // disclosure state alone.
    let selected_run = create_memo(move |_| {
        runs.with(|records| {
            records
                .iter()
                .find(|record| record.id == selected_id)
                .cloned()
        })
        .or_else(|| fallback_for_selection.clone())
    });
    let detail_epoch = Rc::new(Cell::new(0_u64));
    create_effect({
        let detail_epoch = Rc::clone(&detail_epoch);
        move |_| {
            let Some(summary) = selected_run.get() else {
                return;
            };
            let epoch = detail_epoch.get().wrapping_add(1);
            detail_epoch.set(epoch);
            let detail_epoch = Rc::clone(&detail_epoch);
            spawn_local(async move {
                let args = to_value(&serde_json::json!({ "runId": &summary.id })).unwrap();
                let Ok(value) = invoke_checked("get_run_detail", args).await else {
                    return;
                };
                let Ok(record) = serde_wasm_bindgen::from_value::<RunRecord>(value) else {
                    return;
                };
                // A Run may settle between the summary poll and this detail
                // read. Wait for the next summary instead of briefly rendering
                // a newer lifecycle inside an older card and remounting it on
                // the following poll.
                let same_lifecycle = record.status == summary.status
                    && record.ended_at == summary.ended_at
                    && record.exit_code == summary.exit_code;
                let changed = detail.with_untracked(|current| current.as_ref() != Some(&record));
                if detail_epoch.get() == epoch && same_lifecycle && changed {
                    detail.set(Some(record));
                    schedule_run_output_follow();
                }
            });
        }
    });
    // Outside the card closure on purpose: the run list refresh re-renders the
    // body every few seconds, which would snap a native `<details>` shut while
    // the user is reading it.
    let env_open = create_rw_signal(false);
    // Manual entry point: the review button on the card opens the modal
    // directly, for any card.
    let review_modal = use_context::<crate::overlays::RunReviewModal>().map(|modal| modal.0);
    // When a foreground-monitored SSH Run finishes successfully in this
    // session, nominate it for the results-review prompt. The root drains the
    // queue once the session goes idle and asks the backend whether the Run
    // has an unresolved product decision before opening the modal, so work in
    // progress is never interrupted and empty workspaces never prompt (#897).
    let review_queue = auto_review
        .then(|| use_context::<crate::overlays::PendingRunReviews>().map(|queue| queue.0))
        .flatten();
    if let Some(review_queue) = review_queue {
        let prompted = Rc::new(Cell::new(false));
        create_effect(move |previous: Option<Option<String>>| {
            let Some(run) = selected_run.get() else {
                return None;
            };
            let status = run.status.clone();
            let was_active = matches!(
                previous.flatten().as_deref(),
                Some("submitted") | Some("running") | Some("cancelling")
            );
            if was_active
                && status == "succeeded"
                && run.kind == "ssh_direct"
                && run.cleaned_at.is_none()
                && !prompted.get()
            {
                prompted.set(true);
                review_queue.update(|ids| {
                    if !ids.contains(&run.id) {
                        ids.push(run.id.clone());
                    }
                });
            }
            Some(status)
        });
    }
    view! {
        {move || {
            if dismissed_runs.with(|ids| ids.contains(&run_id)) {
                return view! {}.into_view();
            }
            let run = selected_run.get();
            let Some(run) = run else {
                let failed = tool_ok == Some(false);
                let status = if failed { "failed" } else { "running" };
                let status_class = format!("run-status {status}");
                let detail = if failed && !tool_output.trim().is_empty() {
                    tool_output.clone()
                } else {
                    t(locale.get(), "runs.waiting_record").to_string()
                };
                return view! {
                    <article class="run-monitor-card" data-testid="run-monitor-card" data-run-id=run_id.clone()>
                        <div class="run-monitor-head">
                            <span class="run-monitor-icon"><span class="run-dot"></span></span>
                            <div class="run-monitor-title">
                                <strong>{t(locale.get(), "runs.monitoring")}</strong>
                                <code>{run_id.clone()}</code>
                            </div>
                            <span class=status_class>{run_status_label(locale.get(), status)}</span>
                        </div>
                        <div class="run-monitor-empty">{detail}</div>
                    </article>
                }.into_view();
            };
            let title = run_title(&run);
            let status = run.status.clone();
            let status_class = format!("run-status {status}");
            let active = matches!(status.as_str(), "submitted" | "running" | "cancelling");
            let dismissible = matches!(
                status.as_str(),
                "succeeded" | "failed" | "cancelled" | "timed_out" | "lost"
            );
            let cancellable = matches!(status.as_str(), "submitted" | "running" | "cancelling");
            let force_cancel = status == "cancelling";
            let cancel_label = if force_cancel {
                t(locale.get(), "runs.force_cancel")
            } else {
                t(locale.get(), "runs.cancel")
            };
            let started = run.started_at.unwrap_or(run.created_at);
            let meta_context = run.context_id.clone();
            let meta_kind = run.kind.clone();
            let ended_at = run.ended_at;
            let last_heartbeat = run.last_polled_at;
            let timeout_secs = run.timeout_secs;
            let settled_now = js_sys::Date::now() as i64 / 1000;
            let progress = run_progress(&run);
            let detail = detail.get();
            let output = detail.as_ref().map(run_output_preview).unwrap_or_default();
            let command = detail
                .as_ref()
                .and_then(|record| record.command.clone())
                .filter(|value| !value.trim().is_empty());
            let remote_workdir = detail
                .as_ref()
                .and_then(|record| record.remote_workdir.clone())
                .or_else(|| run.remote_workdir.clone());
            let poll_error = run.last_poll_error.clone().filter(|value| !value.trim().is_empty());
            // ponytail: flat `key: value` rows only — nested `config`/`capabilities`
            // render as compact JSON, not a tree. Unparseable or empty snapshots
            // (old rows, transfer runs) yield no pairs, so the block disappears.
            let env_pairs = detail
                .as_ref()
                .map(|record| research::metadata_pairs(&record.env_snapshot_json))
                .unwrap_or_default();
            let cancel_id = run.id.clone();
            let output_id = run.id.clone();
            view! {
                <article class="run-monitor-card" data-testid="run-monitor-card" data-run-id=run.id.clone()>
                    <div class="run-monitor-head">
                        <span class="run-monitor-icon">{
                            if active {
                                view! { <span class="run-dot"></span> }.into_view()
                            } else if status == "succeeded" {
                                view! { <span class="run-monitor-done">"✓"</span> }.into_view()
                            } else {
                                view! { <span class="run-monitor-failed">"!"</span> }.into_view()
                            }
                        }</span>
                        <div class="run-monitor-title">
                            <strong>{title}</strong>
                            <code>{lookup_id.clone()}</code>
                        </div>
                        {if force_cancel {
                            let run_id = cancel_id.clone();
                            let label = run_status_label(locale.get(), &status);
                            let tip = cancel_label.clone();
                            view! {
                                <button type="button" class=status_class
                                    title=tip.clone()
                                    aria-label=tip
                                    on:click=move |_| {
                                        let run_id = run_id.clone();
                                        spawn_local(async move {
                                            let arg = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                                            let _ = invoke("cancel_run", arg).await;
                                        });
                                    }
                                >{label}</button>
                            }.into_view()
                        } else {
                            view! {
                                <span class=status_class>{run_status_label(locale.get(), &status)}</span>
                            }.into_view()
                        }}
                        {cancellable.then(|| {
                            let run_id = cancel_id.clone();
                            let tip = cancel_label.clone();
                            view! {
                                <button type="button" class="icon-btn run-monitor-cancel"
                                    title=tip.clone()
                                    aria-label=tip
                                    on:click=move |_| {
                                        let run_id = run_id.clone();
                                        spawn_local(async move {
                                            let arg = to_value(&serde_json::json!({ "runId": run_id })).unwrap();
                                            let _ = invoke("cancel_run", arg).await;
                                        });
                                    }>{compose_icon("close")}</button>
                            }
                        })}
                        {(dismissible && run.kind == "ssh_direct" && run.cleaned_at.is_none())
                            .then(|| review_modal.map(|review_modal| {
                                let review_id = run.id.clone();
                                let tip = t(locale.get(), "run_review.open");
                                view! {
                                    <button type="button" class="icon-btn run-monitor-review"
                                        data-testid="run-monitor-review"
                                        title=tip.clone()
                                        aria-label=tip
                                        on:click=move |_| review_modal.set(Some(review_id.clone()))
                                    >{compose_icon("folder")}</button>
                                }
                            }))}
                        {dismissible.then(|| {
                            let tip = t(locale.get(), "runs.dismiss");
                            let dismiss_id = run.id.clone();
                            view! {
                                <button type="button" class="icon-btn run-monitor-dismiss"
                                    title=tip.clone()
                                    aria-label=tip
                                    on:click=move |_| dismissed_runs.update(|ids| {
                                        ids.insert(dismiss_id.clone());
                                    })
                                >{compose_icon("close")}</button>
                            }
                        })}
                    </div>
                    <div class="run-monitor-meta">{move || {
                        let now = if active { clock.get() } else { settled_now };
                        run_monitor_meta(
                            locale.get(),
                            &meta_context,
                            &meta_kind,
                            started,
                            ended_at,
                            active,
                            last_heartbeat,
                            timeout_secs,
                            now,
                        )
                    }}</div>
                    {progress.map(|progress| run_progress_meter(progress, locale.get()))}
                    {command.map(|command| view! { <div class="run-monitor-command">{command}</div> })}
                    {remote_workdir.map(|workdir| view! {
                        <div class="run-monitor-remote">
                            <span>{t(locale.get(), "runs.remote_workdir")}</span>
                            <code>{workdir}</code>
                        </div>
                    })}
                    {(!output.is_empty()).then(|| view! {
                        <div class="run-monitor-output">
                            <span>{t(locale.get(), "runs.output")}</span>
                            <pre data-run-output-for=output_id.clone()>{output}</pre>
                        </div>
                    })}
                    {(!env_pairs.is_empty()).then(|| view! {
                        <details class="run-monitor-env" data-testid="run-monitor-env"
                            open=env_open.get()>
                            <summary on:click=move |event| {
                                event.prevent_default();
                                env_open.update(|open| *open = !*open);
                            }>{t(locale.get(), "runs.environment")}</summary>
                            <dl>
                                {env_pairs.into_iter().map(|(key, value)| view! {
                                    <dt>{key}</dt>
                                    <dd>{value}</dd>
                                }).collect_view()}
                            </dl>
                        </details>
                    })}
                    {poll_error.map(|error| view! { <div class="context-error">{error}</div> })}
                </article>
            }.into_view()
        }}
    }
}

pub(crate) fn render_item(
    ui_index: usize,
    item: &ChatItem,
    timestamp: Option<i64>,
    artifacts: Memo<Vec<Artifact>>,
    on_artifact: Callback<usize>,
    on_file: Callback<ModalArtifact>,
    runs: RwSignal<Vec<RunSummary>>,
    run_clock: ReadSignal<i64>,
    busy: ReadSignal<bool>,
    compact_assistant: bool,
    can_modify: bool,
    can_branch: Signal<bool>,
    show_actions: Signal<bool>,
    can_undo: Signal<bool>,
    show_explore: Signal<bool>,
    can_explore: Signal<bool>,
    on_edit: impl Fn(usize) + Clone + 'static,
    on_branch: impl Fn(usize) + Clone + 'static,
    on_undo: Callback<usize>,
    explore_turn_index: usize,
    on_explore: Callback<usize>,
    session_id: String,
    on_memory: Callback<(String, usize)>,
    on_review: Callback<String>,
    on_approval: Callback<(String, bool, Option<String>, String)>,
    on_resume: Callback<usize>,
    disclosure_state: RwSignal<HashMap<String, bool>>,
    plan_mode_active: Signal<bool>,
    plan_compat: Signal<bool>,
    on_plan_decision: Callback<PlanDecision>,
    on_question_answer: Callback<(usize, Option<String>, String)>,
    on_review_jump: Callback<usize>,
    dismissed_runs: RwSignal<HashSet<String>>,
    on_branch_merge: Callback<(String, String)>,
) -> impl IntoView {
    let locale = use_locale();
    match item {
        ChatItem::User(s) => view! {
            <UserMessage
                text=s.clone()
                timestamp=timestamp
                ui_index=ui_index
                busy=busy
                can_modify=can_modify
                can_branch=can_branch
                on_copy=Callback::new(copy_text)
                on_edit=Callback::new(on_edit)
                on_branch=Callback::new(on_branch)
                on_file=on_file
            />
        }
        .into_view(),
        ChatItem::QueuedUser { .. } => view! {}.into_view(),
        ChatItem::Assistant { text, .. } if text.trim().is_empty() => view! {}.into_view(),
        ChatItem::Assistant { text, .. } if text.starts_with("Error: ") => {
            let msg = text
                .strip_prefix("Error: ")
                .unwrap_or(text.as_str())
                .to_string();
            let copy = msg.clone();
            let hint_src = msg.clone();
            let can_resume = can_modify && !msg.starts_with(crate::dto::ACP_TURN_ERROR_PREFIX);
            view! {
                <div class="finding err">
                    <div class="finding-head">
                        <span class="finding-tag">{move || format!("● {}", t(locale.get(), "chat.error"))}</span>
                        <span class="finding-title">{msg}</span>
                        {can_resume.then(|| view! {
                            <button type="button" class="tool-btn"
                                disabled=move || busy.get()
                                on:click=move |_| on_resume.call(ui_index)>
                                {move || t(locale.get(), "chat.resume")}
                            </button>
                        })}
                        <button type="button" class="tool-btn card-copy"
                            title=move || t(locale.get(), "ctx.copy_message")
                            on:click=move |_| copy_text(copy.clone())>
                            {move || t(locale.get(), "msg.copy")}
                        </button>
                    </div>
                    {move || i18n::api_error_hint(locale.get(), &hint_src).map(|hint| view! {
                        <div class="finding-body">{hint}</div>
                    })}
                </div>
            }.into_view()
        }
        ChatItem::Assistant { text, .. } if compact_assistant => {
            let project_root = use_context::<ReadSignal<Option<ProjectInfo>>>()
                .and_then(|project| project.get().map(|project| project.root));
            let html = enrich_md_html(
                md_to_html(text),
                &[],
                &[],
                locale.get(),
                project_root.as_deref(),
            );
            view! {
                <div class="assistant-wrap">
                    <div class="body md compact-markdown"
                        inner_html=html
                        on:click=move |ev: web_sys::MouseEvent| {
                            handle_md_click(&ev, &[], &[], &on_artifact, &on_file)
                        }
                    ></div>
                </div>
            }
            .into_view()
        }
        ChatItem::Assistant {
            text,
            model,
            resources,
        } => view! {
            <AssistantMessage
                text=text.clone()
                model=model.clone()
                timestamp=timestamp
                resources=resources.clone()
                artifacts=artifacts
                source_item=ui_index
                on_artifact=on_artifact
                on_file=on_file
                on_copy=Callback::new(copy_text)
                on_memory=Callback::new({
                    let session_id = session_id.clone();
                    move |_| on_memory.call((session_id.clone(), explore_turn_index))
                })
                on_review=Callback::new(move |_| on_review.call(session_id.clone()))
                on_branch=Callback::new(on_branch)
                can_branch=can_branch
                show_actions=show_actions
                can_undo=can_undo
                on_undo=on_undo
                show_explore=show_explore
                can_explore=can_explore
                explore_turn_index=explore_turn_index
                on_explore=on_explore
            />
        }
        .into_view(),
        ChatItem::BranchMerge {
            text, branch_title, ..
        } => {
            let open_text = text.clone();
            let title = if branch_title.trim().is_empty() {
                t(locale.get(), "branch.merged_result")
            } else {
                branch_title.clone()
            };
            view! {
                <button type="button" class="branch-merge-card" data-testid="branch-merge-card"
                    on:click=move |_| on_branch_merge.call((title.clone(), open_text.clone()))>
                    <span class="branch-merge-card-icon" aria-hidden="true">{compose_icon("branch")}</span>
                    <span class="branch-merge-card-copy">
                        <strong>{t(locale.get(), "branch.merged_result")}</strong>
                        <span>{branch_title.clone()}</span>
                    </span>
                    <span class="branch-merge-card-open">{compose_icon("chevron-right")}</span>
                </button>
            }.into_view()
        }
        ChatItem::Tool { name, .. } if name == "attempt_completion" => view! {}.into_view(),
        ChatItem::FileChanged(_) => view! {}.into_view(),
        ChatItem::Tool {
            name,
            ok,
            input,
            output,
            ..
        } if is_run_monitor_tool(name) => view! {
            <RunMonitorCard
                run_id=input.trim().to_string()
                runs=runs
                clock=run_clock
                tool_ok=*ok
                tool_output=output.clone()
                dismissed_runs=dismissed_runs
                auto_review=true
            />
        }
        .into_view(),
        ChatItem::Tool {
            name,
            ok,
            input,
            output,
            ..
        } if is_image_generation_tool(name) => view! {
            <ImageGenerationCard
                path=input.trim().to_string()
                ok=*ok
                output=output.clone()
                on_file=on_file
            />
        }
        .into_view(),
        ChatItem::Tool {
            name,
            ok,
            input,
            output,
            ..
        } if is_video_generation_tool(name) => view! {
            <VideoGenerationCard
                path=input.trim().to_string()
                ok=*ok
                output=output.clone()
            />
        }
        .into_view(),
        ChatItem::Reasoning(s) => {
            // The chat row is fingerprint-keyed, so every streaming delta
            // rebuilds it and a plain `<details>` would snap shut mid-stream.
            // Keep the open state in the shared disclosure map, like the step
            // group does, and drive `open` from it.
            let open_id = format!("{session_id}:reasoning:{ui_index}");
            let toggle_id = open_id.clone();
            view! {
                <details class="rz"
                    open=move || disclosure_open(disclosure_state, &open_id, false)>
                    <summary on:click=move |event| {
                        event.prevent_default();
                        toggle_disclosure(disclosure_state, &toggle_id, false);
                    }>{move || t(locale.get(), "chat.thinking")}</summary>
                    <div class="body">{s.clone()}</div>
                </details>
            }
            .into_view()
        }
        ChatItem::Tool {
            name,
            ok,
            input,
            output,
            ..
        } => view! {
            <ToolBlock name=name.clone() ok=*ok input=input.clone() output=output.clone() />
        }
        .into_view(),
        ChatItem::Usage {
            input,
            output,
            reasoning,
            cached,
            ..
        } => {
            let (input, output, reasoning, cached) = (*input, *output, *reasoning, *cached);
            view! {
                <div class="usage-line" title=move || t(locale.get(), "msg.usage_title")>
                    {move || {
                        let loc = locale.get();
                        let mut s = tf(loc, "msg.usage", &[
                            ("in", &fmt_tokens(input)),
                            ("out", &fmt_tokens(output)),
                        ]);
                        if cached > 0 {
                            s.push_str(&tf(loc, "msg.usage.cached", &[("c", &fmt_tokens(cached))]));
                        }
                        if reasoning > 0 {
                            s.push_str(&tf(loc, "msg.usage.reasoning", &[("r", &fmt_tokens(reasoning))]));
                        }
                        s
                    }}
                </div>
            }.into_view()
        }
        ChatItem::Compaction {
            before,
            after,
            strategy,
        } => {
            if strategy == "auto_continue" {
                let count = before.to_string();
                let limit = after.to_string();
                view! {
                    <div class="context-compaction-flag auto" data-testid="auto-continue-flag">
                        {compose_icon("sync")}
                        <span>{move || tf(
                            locale.get(),
                            "chat.auto_continued",
                            &[("count", count.as_str()), ("limit", limit.as_str())],
                        )}</span>
                    </div>
                }
                .into_view()
            } else {
                let automatic = strategy == "auto";
                let counts = format!(
                    "{} → {}",
                    fmt_tokens(*before as u64),
                    fmt_tokens(*after as u64)
                );
                view! {
                    <div class="context-compaction-flag" class:auto=automatic data-testid="context-compaction-flag">
                        {compose_icon("doc")}
                        <span>{move || t(
                            locale.get(),
                            if automatic {
                                "chat.context_auto_compacted"
                            } else {
                                "chat.context_compacted"
                            },
                        )}</span>
                        <span class="context-compaction-count">{counts}</span>
                    </div>
                }.into_view()
            }
        }
        ChatItem::AcpTool {
            title,
            status,
            content,
            locations,
            ..
        } => view! {
            <article class="tool-card" data-testid="acp-tool" data-status=status.clone()>
                <header><strong>{title.clone()}</strong><span>{status.clone()}</span></header>
                {(!content.is_empty()).then(|| view! { <pre>{content.clone()}</pre> })}
                {(!locations.is_empty()).then(|| view! { <pre>{locations.clone()}</pre> })}
            </article>
        }
        .into_view(),
        ChatItem::ApprovalPending {
            tool,
            preview,
            message,
        } => view! {
            <ApprovalCard tool=tool.clone() preview=preview.clone() message=message.clone()
                session_id=session_id.clone() on_decide=on_approval
                on_artifact=on_artifact.clone() on_file=on_file.clone() />
        }
        .into_view(),
        ChatItem::AcpPermission {
            request_id,
            tool,
            options,
        } => {
            let request_id = request_id.clone();
            view! {
                <article class="approval-card" data-testid="acp-permission-card">
                    <header><strong>{tool.clone()}</strong><span>"ACP permission"</span></header>
                    <footer class="approval-actions">
                        {options.clone().into_iter().map(|option| {
                            let request_id = request_id.clone();
                            let option_id = option.id.clone();
                            let class = if option.kind.starts_with("allow") { "primary" } else { "" };
                            view! {
                                <button type="button" class=class on:click=move |_| {
                                    let request_id = request_id.clone();
                                    let option_id = option_id.clone();
                                    spawn_local(async move {
                                        let args = to_value(&serde_json::json!({ "requestId": request_id, "optionId": option_id })).unwrap();
                                        let _ = invoke_checked("respond_acp_permission", args).await;
                                    });
                                }>{option.name}</button>
                            }
                        }).collect_view()}
                    </footer>
                </article>
            }.into_view()
        }
        ChatItem::ReviewTransition { phase, model } => {
            let (icon, message_key, data_phase) = match phase {
                ReviewTransitionPhase::Reviewing => {
                    ("↗", "review.transition_to_reviewer", "reviewing")
                }
                ReviewTransitionPhase::Correcting => {
                    ("↩", "review.transition_to_agent", "correcting")
                }
                ReviewTransitionPhase::Passed => ("✓", "review.transition_passed", "passed"),
            };
            let model = model.clone();
            view! {
                <div class="review-transition" data-phase=data_phase>
                    <span class="review-transition-line"></span>
                    <span class="review-transition-icon">{icon}</span>
                    <span class="review-transition-text">{move || t(locale.get(), message_key)}</span>
                    {model.map(|model| view! { <span class="review-transition-model">{model}</span> })}
                    <span class="review-transition-line"></span>
                </div>
            }.into_view()
        }
        ChatItem::Plan(plan) => {
            let streaming = plan.state == PlanState::Streaming;
            let entries = plan.entries.clone();
            let project_root = use_context::<ReadSignal<Option<ProjectInfo>>>()
                .and_then(|project| project.get().map(|project| project.root));
            view! {
                <article class="plan-card" class:streaming=streaming
                    class:compat=move || plan_compat.get() data-testid="plan-card">
                    <header class="plan-card-head">
                        <span class="plan-card-icon">{compose_icon("plan")}</span>
                        <div>
                            <strong>{move || t(locale.get(), "plan.card.title")}</strong>
                            {move || if streaming {
                                Some(view! { <span>{t(locale.get(), "plan.card.streaming")}</span> })
                            } else {
                                plan_mode_active.get().then(|| view! {
                                    <span>{t(locale.get(), "plan.card.ready")}</span>
                                })
                            }}
                        </div>
                        {move || plan_compat.get().then(|| view! {
                            <span class="plan-card-compat" data-testid="plan-compat"
                                title=move || t(locale.get(), "plan.compat_full")>
                                {move || t(locale.get(), "plan.compat")}
                            </span>
                        })}
                    </header>
                    <ul class="plan-card-body" data-testid="plan-entries">
                        {entries.into_iter().map(|entry| {
                            let (status, mark, label) = match entry.status {
                                PlanStatus::Completed => ("completed", "✓", "plan.status.completed"),
                                PlanStatus::InProgress => ("in_progress", "▸", "plan.status.in_progress"),
                                PlanStatus::Pending => ("pending", "", "plan.status.pending"),
                            };
                            let high = entry.priority == PlanPriority::High;
                            let html = enrich_md_html(
                                md_to_html(&entry.content),
                                &[],
                                &[],
                                locale.get(),
                                project_root.as_deref(),
                            );
                            let entry_artifact = on_artifact.clone();
                            let entry_file = on_file.clone();
                            view! {
                                <li data-status=status>
                                    <span class="plan-entry-mark" role="img"
                                        aria-label=move || t(locale.get(), label)>{mark}</span>
                                    <div class="plan-entry-text md" inner_html=html
                                        on:click=move |ev: web_sys::MouseEvent| {
                                            handle_md_click(&ev, &[], &[], &entry_artifact, &entry_file)
                                        }></div>
                                    {high.then(|| view! {
                                        <span class="plan-entry-priority" role="img"
                                            aria-label=move || t(locale.get(), "plan.priority_high")>"!"</span>
                                    })}
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                    {move || plan_mode_active.get().then(|| view! {
                        <footer class="plan-card-actions">
                            <p class="plan-card-hint" data-testid="plan-revision-hint">
                                {move || t(locale.get(), "plan.modify_hint")}
                            </p>
                            <button type="button" class="primary" data-testid="plan-approve"
                                on:click=move |_| on_plan_decision.call(PlanDecision::Approve)>
                                {move || t(locale.get(), "plan.approve")}
                            </button>
                            <button type="button" data-testid="plan-save-exit"
                                on:click=move |_| on_plan_decision.call(PlanDecision::SaveExit)>
                                {move || t(locale.get(), "plan.save_exit")}
                            </button>
                        </footer>
                    })}
                </article>
            }.into_view()
        }
        ChatItem::Question(question) => {
            let state = question.state;
            let pending = state == QuestionState::Pending;
            let request_id = question.request_id.clone();
            let options = question.options.clone();
            // A question with no options can only be answered freeform.
            let allow_freeform = question.allow_freeform || options.is_empty();
            let freeform = create_rw_signal(String::new());
            let data_state = match state {
                QuestionState::Pending => "pending",
                QuestionState::Answered => "answered",
                QuestionState::Expired => "expired",
            };
            let request_id_keydown = request_id.clone();
            let request_id_click = request_id.clone();
            let project_root = use_context::<ReadSignal<Option<ProjectInfo>>>()
                .and_then(|project| project.get().map(|project| project.root));
            let question_html = enrich_md_html(
                md_to_html(&question.question),
                &[],
                &[],
                locale.get(),
                project_root.as_deref(),
            );
            view! {
                <section class="plan-question-card" data-testid="question-card" data-state=data_state>
                    <div class="plan-question-head">
                        <span class="plan-question-icon">{compose_icon("chat")}</span>
                        <strong>{move || t(locale.get(), "plan.question.title")}</strong>
                    </div>
                    <div class="plan-question-text md"
                        inner_html=question_html
                        on:click=move |ev: web_sys::MouseEvent| {
                            handle_md_click(&ev, &[], &[], &on_artifact, &on_file)
                        }
                    ></div>
                    {(pending && !options.is_empty()).then(|| view! {
                        <div class="plan-question-options">{options.into_iter().map(|option| {
                            let request_id = request_id.clone();
                            let answer = option.label.clone();
                            view! {
                                <button type="button"
                                    on:click=move |_| on_question_answer.call((ui_index, request_id.clone(), answer.clone()))>
                                    <strong>{option.label}</strong>
                                    {(!option.description.is_empty()).then(|| view! { <span>{option.description}</span> })}
                                </button>
                            }
                        }).collect_view()}</div>
                    })}
                    {(pending && allow_freeform).then(|| view! {
                        <div class="plan-question-freeform">
                            <input type="text" prop:value=move || freeform.get()
                                placeholder=move || t(locale.get(), "plan.question.placeholder")
                                on:input=move |event| freeform.set(event_target_value(&event))
                                on:keydown=move |event: web_sys::KeyboardEvent| {
                                    if event.key() == "Enter" && !event.shift_key() {
                                        event.prevent_default();
                                        on_question_answer.call((ui_index, request_id_keydown.clone(), freeform.get()));
                                    }
                                } />
                            <button type="button" class="primary" disabled=move || freeform.get().trim().is_empty()
                                on:click=move |_| on_question_answer.call((ui_index, request_id_click.clone(), freeform.get()))>
                                {move || t(locale.get(), "plan.question.send")}
                            </button>
                        </div>
                    })}
                    {(state == QuestionState::Answered).then(|| view! {
                        <footer class="plan-question-note">{move || t(locale.get(), "plan.answer_sent")}</footer>
                    })}
                    {(state == QuestionState::Expired).then(|| view! {
                        <footer class="plan-question-note expired">{move || t(locale.get(), "plan.question.expired")}</footer>
                    })}
                </section>
            }.into_view()
        }
        ChatItem::Review(report) => {
            let report = report.clone();
            let count = report.findings.len();
            let unreviewable = report.review_status == "unreviewable";
            let coverage = report.evidence_coverage.to_string();
            let count_text = count.to_string();
            let all_resolved = count > 0
                && report
                    .findings
                    .iter()
                    .all(|finding| finding.status == "resolved");
            let has_unaddressed = report
                .findings
                .iter()
                .any(|finding| finding.status == "unaddressed");
            let copy = format!(
                "{}\n\n{}",
                report.summary,
                report
                    .findings
                    .iter()
                    .map(|finding| format!(
                        "- {}\n  Evidence: {}\n  Fix: {}",
                        finding.claim, finding.evidence, finding.fix
                    ))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            let model = match (report.reviewer_model.trim(), report.reviewer_effort.trim()) {
                ("", "") => String::new(),
                (model, "") => model.to_string(),
                ("", effort) => effort.to_string(),
                (model, effort) => format!("{model} · {effort}"),
            };
            let summary = report.summary.clone();
            let coverage_gaps = report.coverage_gaps.clone();
            let findings = report
                .findings
                .into_iter()
                .enumerate()
                .map(|(index, finding)| {
                    let resolved = finding.status == "resolved";
                    let status_key = match finding.status.as_str() {
                        "resolved" => "review.resolved",
                        "unaddressed" => "review.unaddressed",
                        _ => "review.open",
                    };
                    let verdict_class = format!("review-pill verdict {}", finding.verdict);
                    let severity_class = format!("review-pill severity {}", finding.severity);
                    let message_index = finding.message_index;
                    view! {
                        <div class="review-finding" class:resolved=resolved>
                            <div class="review-finding-head">
                                <span class="review-finding-number">{index + 1}</span>
                                <span class=verdict_class>{finding.verdict}</span>
                                <span class=severity_class>{finding.severity}</span>
                                <span class="review-pill status">{move || t(locale.get(), status_key)}</span>
                                <button type="button" class="tool-btn review-jump"
                                    on:click=move |_| on_review_jump.call(message_index)>
                                    {move || t(locale.get(), "review.go_to_transcript")}
                                </button>
                            </div>
                            <div class="review-claim">{finding.claim}</div>
                            <div class="review-detail">
                                <strong>{move || t(locale.get(), "review.evidence")}</strong>
                                <span>{finding.evidence}</span>
                            </div>
                            <div class="review-detail">
                                <strong>{move || t(locale.get(), "review.fix")}</strong>
                                <span>{finding.fix}</span>
                            </div>
                        </div>
                    }
                })
                .collect_view();
            view! {
                <div class="review-card">
                    <div class="review-head">
                        <span class="review-badge">"🔍"</span>
                        <span>{move || t(locale.get(), "review.title")}</span>
                        <span class="review-count">{move || tf(locale.get(), "review.findings_n", &[("n", &count_text)])}</span>
                        {(!model.is_empty()).then(|| view! { <span class="review-model">{model}</span> })}
                        <button type="button" class="tool-btn card-copy"
                            title=move || t(locale.get(), "ctx.copy_message")
                            on:click=move |_| copy_text(copy.clone())>
                            {move || t(locale.get(), "msg.copy")}
                        </button>
                    </div>
                    <div class="review-summary">{summary}</div>
                    {(count == 0 && !unreviewable).then(|| view! {
                        <div class="review-empty">"✓ "{move || t(locale.get(), "review.no_findings")}</div>
                    })}
                    {unreviewable.then(|| view! {
                        <div class="review-empty review-unreviewable">
                            "⚠ "{move || tf(locale.get(), "review.unreviewable", &[("pct", &coverage)])}
                        </div>
                    })}
                    {(!coverage_gaps.is_empty()).then(|| view! {
                        <details class="review-coverage-gaps">
                            <summary>{move || t(locale.get(), "review.coverage_gaps")}</summary>
                            <ul>{coverage_gaps.into_iter().map(|gap| view! { <li>{gap}</li> }).collect_view()}</ul>
                        </details>
                    })}
                    {findings}
                    {(count > 0).then(|| view! {
                        <div class="review-foot" class:resolved=all_resolved class:unaddressed=has_unaddressed>
                            {move || {
                                let key = if all_resolved {
                                    "review.all_fixed"
                                } else if has_unaddressed {
                                    "review.needs_attention"
                                } else {
                                    "review.agent_correcting"
                                };
                                t(locale.get(), key)
                            }}
                        </div>
                    })}
                </div>
            }.into_view()
        }
    }
}
