use super::*;

pub(crate) fn begin_pending_turn(
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    id: &str,
) {
    pending.update(|m| {
        *m.entry(id.to_string()).or_insert(0) += 1;
    });
    running.update(|r| {
        r.insert(id.to_string());
    });
}

pub(crate) fn finish_pending_turn(
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    id: &str,
) {
    let remaining = pending.with(|m| m.get(id).copied().unwrap_or(0));
    if remaining > 1 {
        pending.update(|m| {
            if let Some(n) = m.get_mut(id) {
                *n -= 1;
            }
        });
        return;
    }
    pending.update(|m| {
        m.remove(id);
    });
    running.update(|r| {
        r.remove(id);
    });
}

pub(crate) fn clear_running_if_idle(
    pending: RwSignal<HashMap<String, usize>>,
    running: RwSignal<HashSet<String>>,
    id: &str,
) {
    if pending.with(|m| m.get(id).copied().unwrap_or(0)) == 0 {
        running.update(|r| {
            r.remove(id);
        });
    }
}

pub(crate) fn strip_approval_pending(items: &mut Vec<ChatItem>) {
    items.retain(|i| {
        !matches!(
            i,
            ChatItem::ApprovalPending { .. } | ChatItem::AcpPermission { .. }
        )
    });
}

/// Land a live plan update: it replaces only the card this turn is still
/// writing, so plans from earlier turns stay as the plan's history. Shared by
/// the ACP plan update and the built-in `propose_plan` result.
pub(crate) fn upsert_plan_card(items: &mut Vec<ChatItem>, card: PlanCard) {
    if let Some(index) = items
        .iter()
        .rposition(|row| matches!(row, ChatItem::Plan(plan) if plan.state == PlanState::Streaming))
    {
        items[index] = ChatItem::Plan(card);
    } else {
        let index = process_item_insert_index(items);
        items.insert(index, ChatItem::Plan(card));
    }
}

/// Turn end freezes the plan the turn produced. `Streaming` also marks which
/// card live updates may replace, so settling it keeps the next turn's plan a
/// separate card instead of overwriting this one.
pub(crate) fn settle_plan_cards(items: &mut [ChatItem]) {
    for item in items {
        if let ChatItem::Plan(plan) = item {
            plan.state = PlanState::Ready;
        }
    }
}

pub(crate) fn upsert_review(items: &mut Vec<ChatItem>, report: ReviewReport) {
    if let Some(existing) = items
        .iter_mut()
        .find(|item| matches!(item, ChatItem::Review(current) if current.id == report.id))
    {
        *existing = ChatItem::Review(report);
    } else {
        let index = trailing_queue_start(items);
        items.insert(index, ChatItem::Review(report));
    }
}

pub(crate) fn is_error_assistant(item: &ChatItem) -> bool {
    matches!(item, ChatItem::Assistant { text, .. } if text.starts_with("Error: "))
}

/// Follow-up suggestions belong only to a turn that actually produced a final
/// answer. In particular, assistant commentary before a tool is not a final
/// answer: if the provider drops after that tool, a stray `Done` event must not
/// make the interrupted task look complete by offering next questions.
pub(crate) fn latest_turn_has_final_answer(items: &[ChatItem]) -> bool {
    let turn_start = items
        .iter()
        .rposition(|item| matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. }))
        .map_or(0, |index| index.saturating_add(1));
    items
        .iter()
        .enumerate()
        .skip(turn_start)
        .any(|(index, item)| {
            matches!(item, ChatItem::Assistant { text, .. } if !text.trim().is_empty() && !text.starts_with("Error: "))
                && !is_commentary_at(items, index)
        })
}

pub(crate) fn strip_error_at(items: &mut Vec<ChatItem>, idx: usize) {
    if idx < items.len() && is_error_assistant(&items[idx]) {
        items.remove(idx);
    }
}

pub(crate) fn ensure_streaming_assistant(items: &mut Vec<ChatItem>, model: Option<String>) {
    let queue_start = trailing_queue_start(items);
    let has_blank = items[..queue_start]
        .iter()
        .rev()
        .any(|i| matches!(i, ChatItem::Assistant { text, .. } if text.trim().is_empty()));
    if !has_blank {
        items.insert(
            queue_start,
            ChatItem::Assistant {
                text: String::new(),
                model,
                resources: Vec::new(),
            },
        );
    }
}

pub(crate) fn last_tool_input(items: &[ChatItem], tool: &str) -> String {
    items
        .iter()
        .rev()
        .find_map(|i| match i {
            ChatItem::Tool {
                name,
                input,
                ok: None,
                ..
            } if name == tool => Some(input.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

pub(crate) fn trailing_queue_start(items: &[ChatItem]) -> usize {
    items
        .iter()
        .rposition(|item| !matches!(item, ChatItem::QueuedUser { .. }))
        .map(|i| i + 1)
        .unwrap_or(0)
}

pub(crate) fn start_user_turn(items: &mut Vec<ChatItem>, text: String, model: Option<String>) {
    let incoming_body = composer_text_from_user_message(&text);
    // ponytail: text-keyed promotion; upgrade to a backend intent_id if
    // display/echo texts ever diverge beyond the attachment suffix.
    if let Some((idx, queued)) = items.iter().enumerate().find_map(|(i, item)| match item {
        ChatItem::QueuedUser { text: queued, .. }
            if queued == &text || composer_text_from_user_message(queued) == incoming_body =>
        {
            Some((i, queued.clone()))
        }
        _ => None,
    }) {
        // Prefer the longer display form, mirroring the ack path below.
        let display = if queued.len() > text.len() {
            queued
        } else {
            text
        };
        items.splice(
            idx..=idx,
            [
                ChatItem::User(display),
                ChatItem::Assistant {
                    text: String::new(),
                    model,
                    resources: Vec::new(),
                },
            ],
        );
    } else if let Some(idx) = items.windows(2).position(|pair| {
        matches!(
            &pair[0],
            ChatItem::User(s)
                if s == &text || composer_text_from_user_message(s) == incoming_body
        ) && matches!(&pair[1], ChatItem::Assistant { text: assistant, .. } if assistant.is_empty())
    }) {
        // Normal sends are rendered optimistically. The backend User event is
        // only an acknowledgement in that case, so do not append a duplicate.
        // Prefer the longer display form when one side still lacks the
        // "Uploaded files:" (or reference) suffix.
        if let ChatItem::User(existing) = &mut items[idx] {
            if text.len() > existing.len() {
                *existing = text;
            }
        }
    } else {
        items.push(ChatItem::User(text));
        items.push(ChatItem::Assistant {
            text: String::new(),
            model,
            resources: Vec::new(),
        });
    }
}

#[cfg(test)]
mod start_user_turn_tests {
    use super::{
        append_assistant_delta, append_reasoning_delta, completed_activity_end,
        composer_text_from_user_message, is_commentary_at, is_image_generation_tool,
        is_tool_activity, message_with_attachments, message_with_composer_context,
        message_with_quotes, message_with_read_only_quotes, process_item_insert_index,
        runtime_object_quote, selection_targets_center_file, start_user_turn, trailing_queue_start,
        ComposerQuote, ComposerReferenceChip,
    };
    use crate::dto::{ChatItem, ContextUsage};

    #[test]
    fn message_with_attachments_appends_suffix() {
        assert_eq!(
            message_with_attachments("描述下图片", &["uploads/a.png".into()]),
            "描述下图片\n\nUploaded files: uploads/a.png"
        );
        assert_eq!(
            message_with_attachments("  ", &["uploads/a.png".into()]),
            "Uploaded files: uploads/a.png"
        );
    }

    #[test]
    fn message_with_context_keeps_reference_labels_for_transcript_ui() {
        let refs = vec![
            ComposerReferenceChip::Artifact {
                id: "a1".into(),
                name: "counts.csv".into(),
            },
            ComposerReferenceChip::Session {
                id: "s1".into(),
                title: "QC review".into(),
                project_name: "Atlas".into(),
            },
            ComposerReferenceChip::Project {
                id: "p1".into(),
                name: "Atlas".into(),
            },
            ComposerReferenceChip::Skill {
                name: "bear-review".into(),
            },
            ComposerReferenceChip::Workflow {
                id: "roundtable".into(),
                name: "Roundtable".into(),
            },
            ComposerReferenceChip::Context {
                id: "ssh:cpu1".into(),
                label: "CPU1".into(),
            },
            ComposerReferenceChip::Runtime {
                context_id: "local".into(),
                context_label: "Local".into(),
                language: "r".into(),
            },
        ];
        assert_eq!(
            message_with_composer_context(
                "Compare these",
                &["uploads/plot.png".into()],
                &refs,
                &[]
            ),
            "Compare these\n\nUploaded files: uploads/plot.png\n\nAttached artifacts: counts.csv\n\nAttached sessions: Atlas / QC review\n\nProject context: Atlas\n\nSelected skills: bear-review\n\nSelected workflows: Roundtable\n\nTarget environments: CPU1\n\nTarget runtimes: R · Local"
        );
    }

    #[test]
    fn runtime_object_quote_skips_placeholder_fields() {
        assert_eq!(
            runtime_object_quote("Python", "df", "DataFrame", "(100, 3)", "2.3 MB"),
            "[Python runtime] df: DataFrame = (100, 3) (2.3 MB)"
        );
        assert_eq!(
            runtime_object_quote("R", "fit", "lm", "—", "—"),
            "[R runtime] fit: lm"
        );
    }

    #[test]
    fn message_with_quotes_prefixes_blockquotes() {
        assert_eq!(
            message_with_quotes(
                "这是什么意思?",
                &[ComposerQuote::plain("line one\nline two")]
            ),
            "> line one\n> line two\n\n这是什么意思?"
        );
        assert_eq!(message_with_quotes("plain", &[]), "plain");
        assert_eq!(
            message_with_quotes("", &[ComposerQuote::plain("ctx")]),
            "> ctx"
        );
    }

    #[test]
    fn center_selection_match_requires_two_real_paths() {
        assert!(selection_targets_center_file(
            Some("analysis.R"),
            Some("analysis.R")
        ));
        assert!(!selection_targets_center_file(None, None));
        assert!(!selection_targets_center_file(Some("analysis.R"), None));
        assert!(!selection_targets_center_file(None, Some("analysis.R")));
    }

    #[test]
    fn read_only_quotes_keep_source_without_edit_instruction() {
        let message = message_with_read_only_quotes(
            "这是什么意思?",
            &[ComposerQuote::from_selection(
                "plot(1:3)",
                Some("analysis.R".into()),
            )],
        );
        assert!(message.starts_with(
            "Selected excerpt from workspace file `analysis.R`:\n> plot(1:3)\n\n这是什么意思?"
        ));
        assert!(!message.contains("AI source-edit instruction"));
    }

    #[test]
    fn workspace_quote_carries_an_actionable_edit_target() {
        let message = message_with_quotes(
            "改成散点图",
            &[ComposerQuote::from_selection(
                "plot(1:3)",
                Some("analysis.R".into()),
            )],
        );
        assert!(message.starts_with(
            "Selected excerpt from workspace file `analysis.R`:\n> plot(1:3)\n\n改成散点图"
        ));
        assert!(message.contains("read the selected workspace file first"));
        assert!(message.contains("edit tool"));
        assert!(message.ends_with("Target file: `analysis.R`"));
    }

    #[test]
    fn immutable_reference_quote_does_not_request_a_file_edit() {
        let message = message_with_quotes(
            "解释一下",
            &[ComposerQuote::from_selection(
                "result",
                Some("artifact:report".into()),
            )],
        );
        assert!(message.starts_with("Selected excerpt from reference `artifact:report`:"));
        assert!(!message.contains("AI source-edit instruction"));

        let binary = message_with_quotes(
            "改一下",
            &[ComposerQuote::from_selection(
                "rendered text",
                Some("manuscript.docx".into()),
            )],
        );
        assert!(binary.starts_with("Selected excerpt from reference `manuscript.docx`:"));
        assert!(!binary.contains("AI source-edit instruction"));
    }

    #[test]
    fn does_not_duplicate_when_backend_acks_bare_body() {
        let display = message_with_attachments("图片里有啥文字?", &["uploads/img.png".into()]);
        let mut items = vec![
            ChatItem::User(display.clone()),
            ChatItem::Assistant {
                text: String::new(),
                model: Some("gpt".into()),
                resources: Vec::new(),
            },
        ];
        start_user_turn(&mut items, "图片里有啥文字?".into(), Some("gpt".into()));
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], ChatItem::User(s) if s == &display));
    }

    #[test]
    fn upgrades_optimistic_row_when_ack_has_suffix() {
        let display = message_with_attachments("描述下图片", &["uploads/img.png".into()]);
        let mut items = vec![
            ChatItem::User("描述下图片".into()),
            ChatItem::Assistant {
                text: String::new(),
                model: None,
                resources: Vec::new(),
            },
        ];
        start_user_turn(&mut items, display.clone(), None);
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], ChatItem::User(s) if s == &display));
        assert_eq!(composer_text_from_user_message(&display), "描述下图片");
    }

    #[test]
    fn acp_activity_preserves_stream_order_after_a_started_reply() {
        let mut items = vec![
            ChatItem::User("查下文献".into()),
            ChatItem::Assistant {
                text: "我先查近年文献".into(),
                model: Some("Codex".into()),
                resources: Vec::new(),
            },
        ];

        append_reasoning_delta(&mut items, "Searching for literature.".into());
        let idx = process_item_insert_index(&items);
        items.insert(
            idx,
            ChatItem::AcpTool {
                call_id: "1".into(),
                title: "web_search".into(),
                kind: "search".into(),
                status: "in_progress".into(),
                content: String::new(),
                locations: String::new(),
            },
        );

        assert!(matches!(&items[0], ChatItem::User(_)));
        assert!(matches!(&items[1], ChatItem::Assistant { text, .. } if text == "我先查近年文献"));
        assert!(matches!(&items[2], ChatItem::Reasoning(t) if t == "Searching for literature."));
        assert!(matches!(&items[3], ChatItem::AcpTool { .. }));
        assert!(is_commentary_at(&items, 1));
    }

    #[test]
    fn native_thinking_precedes_reply_and_stays_at_the_tail() {
        // Native reasoning models can emit thinking before any visible reply;
        // keep it after the streaming placeholder in arrival order.
        let mut items = vec![
            ChatItem::User("hi".into()),
            ChatItem::Assistant {
                text: String::new(),
                model: None,
                resources: Vec::new(),
            },
        ];

        append_reasoning_delta(&mut items, "Let me think.".into());

        assert!(matches!(&items[1], ChatItem::Assistant { text, .. } if text.is_empty()));
        assert!(matches!(&items[2], ChatItem::Reasoning(t) if t == "Let me think."));
    }

    #[test]
    fn active_deltas_stay_before_queued_turns() {
        let mut items = vec![
            ChatItem::User("alpha".into()),
            ChatItem::Assistant {
                text: "echo:alpha".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::QueuedUser {
                id: 1,
                text: "queued".into(),
            },
        ];

        assert_eq!(trailing_queue_start(&items), 2);
        append_assistant_delta(&mut items, ":tail".into(), None);

        assert!(matches!(
            &items[1],
            ChatItem::Assistant { text, .. } if text == "echo:alpha:tail"
        ));
        assert!(matches!(&items[2], ChatItem::QueuedUser { text, .. } if text == "queued"));
    }

    #[test]
    fn assistant_deltas_do_not_cross_tool_rows() {
        let mut items = vec![
            ChatItem::User("question".into()),
            ChatItem::Assistant {
                text: "first status".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::Tool {
                name: "python".into(),
                ok: Some(true),
                input: "print(1)".into(),
                output: "1".into(),
                started_at_ms: None,
                duration_ms: Some(1),
                call_key: None,
            },
        ];

        append_assistant_delta(&mut items, "final answer".into(), None);

        assert!(matches!(&items[1], ChatItem::Assistant { text, .. } if text == "first status"));
        assert!(matches!(&items[3], ChatItem::Assistant { text, .. } if text == "final answer"));
    }

    #[test]
    fn intermediate_assistant_rows_are_commentary_not_final_answers() {
        let assistant = |text: &str| ChatItem::Assistant {
            text: text.into(),
            model: None,
            resources: Vec::new(),
        };
        let tool = |name: &str| ChatItem::Tool {
            name: name.into(),
            ok: Some(true),
            input: String::new(),
            output: String::new(),
            started_at_ms: None,
            duration_ms: None,
            call_key: None,
        };
        let items = vec![
            ChatItem::User("question".into()),
            assistant("first status"),
            tool("python"),
            assistant("second status"),
            ChatItem::Reasoning("checking".into()),
            tool("read_file"),
            assistant("final answer"),
        ];

        assert!(is_commentary_at(&items, 1));
        assert!(is_commentary_at(&items, 3));
        assert!(!is_commentary_at(&items, 6));
    }

    #[test]
    fn image_generation_is_not_folded_into_tool_activity() {
        let image = ChatItem::Tool {
            name: "generate_image".into(),
            ok: None,
            input: "figures/pathway.png".into(),
            output: String::new(),
            started_at_ms: None,
            duration_ms: None,
            call_key: None,
        };

        assert!(is_image_generation_tool("generate_image"));
        assert!(!is_tool_activity(&image));
    }

    #[test]
    fn completed_activity_folds_until_the_final_answer() {
        let assistant = |text: &str| ChatItem::Assistant {
            text: text.into(),
            model: None,
            resources: Vec::new(),
        };
        let items = vec![
            ChatItem::User("question".into()),
            assistant("checking"),
            ChatItem::Reasoning("thinking".into()),
            ChatItem::Tool {
                name: "read".into(),
                ok: Some(true),
                input: String::new(),
                output: String::new(),
                started_at_ms: None,
                duration_ms: Some(4),
                call_key: None,
            },
            ChatItem::FileChanged("results/new.csv".into()),
            ChatItem::Tool {
                name: "write".into(),
                ok: Some(true),
                input: String::new(),
                output: String::new(),
                started_at_ms: None,
                duration_ms: Some(2),
                call_key: None,
            },
            assistant("final answer"),
        ];

        assert_eq!(completed_activity_end(&items, 1, false), Some(6));
        assert_eq!(completed_activity_end(&items, 1, true), None);
    }

    #[test]
    fn historical_activity_can_fold_while_the_latest_turn_is_busy() {
        let assistant = |text: &str| ChatItem::Assistant {
            text: text.into(),
            model: None,
            resources: Vec::new(),
        };
        let items = vec![
            ChatItem::User("old".into()),
            ChatItem::Reasoning("old thought".into()),
            assistant("old answer"),
            ChatItem::User("current".into()),
            ChatItem::Reasoning("current thought".into()),
        ];

        assert_eq!(completed_activity_end(&items, 1, true), Some(2));
        assert_eq!(completed_activity_end(&items, 4, true), None);
    }

    #[test]
    fn tool_rows_stay_before_the_usage_tail() {
        let items = vec![
            ChatItem::User("question".into()),
            ChatItem::Assistant {
                text: "status".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::Usage {
                input: 10,
                output: 2,
                reasoning: 0,
                cached: 0,
                ctx_tokens: 0,
                max_context: 0,
                context_usage: ContextUsage::default(),
            },
            ChatItem::QueuedUser {
                id: 1,
                text: "next".into(),
            },
        ];

        assert_eq!(process_item_insert_index(&items), 2);
    }

    #[test]
    fn promotes_queued_turn_when_backend_acks_bare_body() {
        let display = message_with_attachments("图片里有啥文字?", &["uploads/img.png".into()]);
        let mut items = vec![
            ChatItem::User("alpha".into()),
            ChatItem::Assistant {
                text: "done".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::QueuedUser {
                id: 1,
                text: display.clone(),
            },
        ];

        start_user_turn(&mut items, "图片里有啥文字?".into(), None);

        assert_eq!(items.len(), 4);
        assert!(matches!(&items[2], ChatItem::User(s) if s == &display));
        assert!(matches!(
            &items[3],
            ChatItem::Assistant { text, .. } if text.is_empty()
        ));
    }

    #[test]
    fn backend_user_event_promotes_the_matching_queued_turn() {
        let mut items = vec![
            ChatItem::User("alpha".into()),
            ChatItem::Assistant {
                text: "done".into(),
                model: None,
                resources: Vec::new(),
            },
            ChatItem::QueuedUser {
                id: 1,
                text: "queued".into(),
            },
            ChatItem::QueuedUser {
                id: 2,
                text: "later".into(),
            },
        ];

        start_user_turn(&mut items, "queued".into(), Some("model".into()));

        assert!(matches!(&items[2], ChatItem::User(text) if text == "queued"));
        assert!(matches!(
            &items[3],
            ChatItem::Assistant { text, model, .. } if text.is_empty() && model.as_deref() == Some("model")
        ));
        assert!(matches!(&items[4], ChatItem::QueuedUser { text, .. } if text == "later"));
    }
}

pub(crate) fn append_assistant_delta(
    items: &mut Vec<ChatItem>,
    delta: String,
    model: Option<String>,
) {
    let queue_start = trailing_queue_start(items);
    if let Some(ChatItem::Assistant { text, .. }) = queue_start
        .checked_sub(1)
        .and_then(|idx| items.get_mut(idx))
    {
        text.push_str(&delta);
        return;
    }
    items.insert(
        queue_start,
        ChatItem::Assistant {
            text: delta,
            model,
            resources: Vec::new(),
        },
    );
}

/// Keep the per-turn usage line at the tail while a new tool row is inserted.
/// Otherwise usage splits the assistant preamble from the tool it introduces.
pub(crate) fn process_item_insert_index(items: &[ChatItem]) -> usize {
    let queue_start = trailing_queue_start(items);
    if queue_start > 0 && matches!(items[queue_start - 1], ChatItem::Usage { .. }) {
        queue_start - 1
    } else {
        queue_start
    }
}

/// Upsert a live tool-call draft row (arguments still streaming). Drafts are
/// keyed by `call_key` (`{round}:{index}`), never by tool name, so two
/// concurrent calls of the same tool keep separate rows. The row is a
/// placeholder with no start time; the host clears it right before the real
/// ToolCall event arrives, and `clear_tool_drafts` drops any leftovers.
pub(crate) fn upsert_tool_draft(
    items: &mut Vec<ChatItem>,
    call_key: String,
    name: String,
    preview: String,
) {
    if let Some(row) = items.iter_mut().rev().find(|item| {
        matches!(item, ChatItem::Tool { call_key: Some(key), ok: None, .. } if *key == call_key)
    }) {
        if let ChatItem::Tool {
            name: row_name,
            input,
            ..
        } = row
        {
            *row_name = name;
            *input = preview;
        }
        return;
    }
    let idx = process_item_insert_index(items);
    items.insert(
        idx,
        ChatItem::Tool {
            name,
            ok: None,
            input: preview,
            output: String::new(),
            started_at_ms: None,
            duration_ms: None,
            call_key: Some(call_key),
        },
    );
}

/// Drop draft rows: just `Some(key)` when its real call starts, or every
/// unfinished draft (`None`) at a turn boundary so no ghost rows remain.
/// Finished rows are never drafts, so a late clear cannot remove real output.
pub(crate) fn clear_tool_drafts(items: &mut Vec<ChatItem>, call_key: Option<&str>) {
    items.retain(|item| {
        !matches!(item, ChatItem::Tool { call_key: Some(key), ok: None, .. } if call_key.is_none_or(|target| key == target))
    });
}

pub(crate) fn is_run_monitor_tool(name: &str) -> bool {
    matches!(name, "monitor_run" | "wisp_monitor_run")
}

pub(crate) fn is_image_generation_tool(name: &str) -> bool {
    name == "generate_image"
}

pub(crate) fn is_tool_activity(item: &ChatItem) -> bool {
    match item {
        ChatItem::Tool { name, .. } => {
            name != "attempt_completion"
                && !is_run_monitor_tool(name)
                && !is_image_generation_tool(name)
        }
        ChatItem::AcpTool { .. } => true,
        _ => false,
    }
}

/// Assistant text that introduces a tool is visible commentary, while the last
/// assistant row in a turn keeps the full answer treatment.
pub(crate) fn is_commentary_at(items: &[ChatItem], index: usize) -> bool {
    if !matches!(&items[index], ChatItem::Assistant { text, .. } if !text.trim().is_empty()) {
        return false;
    }
    items[index + 1..]
        .iter()
        .find(|item| {
            !matches!(
                item,
                ChatItem::Reasoning(_) | ChatItem::Usage { .. } | ChatItem::FileChanged(_)
            )
        })
        .is_some_and(is_tool_activity)
}

pub(crate) fn is_turn_activity_at(items: &[ChatItem], index: usize) -> bool {
    matches!(items.get(index), Some(ChatItem::Reasoning(_)))
        || items.get(index).is_some_and(is_tool_activity)
        || (index < items.len() && is_commentary_at(items, index))
}

/// End (exclusive) of the contiguous process activity that can collapse once
/// its turn is complete. The active tail stays expanded while `busy` is true;
/// turns followed by another user message are historical and may still fold.
pub(crate) fn completed_activity_end(
    items: &[ChatItem],
    start: usize,
    busy: bool,
) -> Option<usize> {
    if !is_turn_activity_at(items, start) {
        return None;
    }

    let boundary = items[start + 1..]
        .iter()
        .position(|item| matches!(item, ChatItem::User(_) | ChatItem::QueuedUser { .. }))
        .map(|offset| start + 1 + offset);
    let historical = boundary.is_some_and(|index| matches!(items[index], ChatItem::User(_)));
    if busy && !historical {
        return None;
    }

    let turn_end = boundary.unwrap_or(items.len());
    let mut end = start;
    while end < turn_end {
        if is_turn_activity_at(items, end)
            || matches!(&items[end], ChatItem::Assistant { text, .. } if text.trim().is_empty())
            || matches!(&items[end], ChatItem::FileChanged(_))
        {
            end += 1;
        } else {
            break;
        }
    }
    Some(end)
}

pub(crate) fn append_reasoning_delta(items: &mut Vec<ChatItem>, delta: String) {
    let queue_start = trailing_queue_start(items);
    if let Some(ChatItem::Reasoning(text)) = queue_start
        .checked_sub(1)
        .and_then(|idx| items.get_mut(idx))
    {
        text.push_str(&delta);
        return;
    }
    items.insert(queue_start, ChatItem::Reasoning(delta));
}

/// Cap on streamed tool output kept in the transcript. Long tasks (package
/// installs, training loops) can otherwise grow one string without bound and
/// every re-render lays out megabytes of DOM.
pub(crate) const MAX_STREAM_OUTPUT_BYTES: usize = 64 * 1024;

/// Terminal-style append: `\r` rewinds to the start of the current line so
/// progress bars (`====>`, tqdm, curl) overwrite in place instead of piling
/// up thousands of stale frames, `\r\n` stays a plain newline, and the stored
/// text is trimmed to a bounded tail. A chunk ending in a bare `\r` keeps it
/// so the next chunk can distinguish CRLF from a rewind.
pub(crate) fn push_terminal_chunk(output: &mut String, chunk: &str) {
    let mut rest = chunk;
    if output.ends_with('\r') && !rest.is_empty() {
        output.pop();
        if let Some(stripped) = rest.strip_prefix('\n') {
            output.push('\n');
            rest = stripped;
        } else {
            truncate_last_line(output);
        }
    }
    while let Some(pos) = rest.find('\r') {
        output.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        if after.is_empty() {
            output.push('\r');
            rest = after;
            break;
        }
        if let Some(stripped) = after.strip_prefix('\n') {
            output.push('\n');
            rest = stripped;
        } else {
            truncate_last_line(output);
            rest = after;
        }
    }
    output.push_str(rest);
    if output.len() > MAX_STREAM_OUTPUT_BYTES {
        let mut cut = output.len() - MAX_STREAM_OUTPUT_BYTES;
        while !output.is_char_boundary(cut) {
            cut += 1;
        }
        output.drain(..cut);
    }
}

fn truncate_last_line(output: &mut String) {
    let line_start = output.rfind('\n').map_or(0, |index| index + 1);
    output.truncate(line_start);
}

/// Render-time variant for text persisted with raw `\r` (run stdout tails).
pub(crate) fn fold_carriage_returns(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    let mut folded = String::with_capacity(text.len());
    push_terminal_chunk(&mut folded, text);
    if folded.ends_with('\r') {
        folded.pop();
    }
    folded
}

pub(crate) fn append_stdout_chunk(items: &mut Vec<ChatItem>, chunk: String) {
    let queue_start = trailing_queue_start(items);
    if let Some(idx) = items[..queue_start]
        .iter()
        .rposition(|item| matches!(item, ChatItem::Tool { .. }))
    {
        if let ChatItem::Tool { output, .. } = &mut items[idx] {
            push_terminal_chunk(output, &chunk);
            return;
        }
    }
    let mut output = String::new();
    push_terminal_chunk(&mut output, &chunk);
    items.insert(
        queue_start,
        ChatItem::Tool {
            name: "stdout".into(),
            ok: None,
            input: String::new(),
            output,
            started_at_ms: None,
            duration_ms: None,
            call_key: None,
        },
    );
}

// --- Streaming delta batching (#65) ------------------------------------------
//
// The backend emits one `agent` event per LLM/stdout chunk. Applying each one
// writes the `items` signal, and every write re-runs the thread view and the
// artifact scan — O(conversation length) work per token, which freezes long
// conversations. Buffer the append-only deltas and flush them on a short timer
// so the signal is written at most ~20×/s regardless of token rate.

pub(crate) enum PendingDelta {
    Text(String),
    Reasoning(String),
    Stdout(String),
}

pub(crate) type DeltaBuf = Rc<RefCell<HashMap<String, Vec<PendingDelta>>>>;

/// Append a delta to a session's queue, coalescing consecutive same-kind chunks.
pub(crate) fn queue_delta(buf: &DeltaBuf, fid: String, d: PendingDelta) {
    let mut map = buf.borrow_mut();
    let q = map.entry(fid).or_default();
    match (q.last_mut(), d) {
        (Some(PendingDelta::Text(s)), PendingDelta::Text(n)) => s.push_str(&n),
        (Some(PendingDelta::Reasoning(s)), PendingDelta::Reasoning(n)) => s.push_str(&n),
        (Some(PendingDelta::Stdout(s)), PendingDelta::Stdout(n)) => s.push_str(&n),
        (_, d) => q.push(d),
    }
}

/// Apply all buffered deltas to their sessions in arrival order.
pub(crate) fn flush_delta_buf(
    buf: &DeltaBuf,
    active: RwSignal<Option<String>>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    models: RwSignal<Vec<ModelProfile>>,
    session_models: RwSignal<HashMap<String, String>>,
) {
    let drained: Vec<_> = buf.borrow_mut().drain().collect();
    if drained.is_empty() {
        return;
    }
    let profiles = models.get_untracked();
    let bindings = session_models.get_untracked();
    for (fid, deltas) in drained {
        let model = session_model_label(&profiles, &bindings, Some(&fid));
        route_items(active, items, transcripts, &fid, move |v| {
            for d in deltas {
                match d {
                    PendingDelta::Text(s) => append_assistant_delta(v, s, model.clone()),
                    PendingDelta::Reasoning(s) => append_reasoning_delta(v, s),
                    PendingDelta::Stdout(s) => append_stdout_chunk(v, s),
                }
            }
        });
    }
}

pub(crate) fn schedule_delta_flush(
    buf: &DeltaBuf,
    scheduled: &Rc<Cell<bool>>,
    active: RwSignal<Option<String>>,
    items: RwSignal<Vec<ChatItem>>,
    transcripts: RwSignal<HashMap<String, Vec<ChatItem>>>,
    models: RwSignal<Vec<ModelProfile>>,
    session_models: RwSignal<HashMap<String, String>>,
) {
    if scheduled.get() {
        return;
    }
    scheduled.set(true);
    let buf = buf.clone();
    let scheduled = scheduled.clone();
    set_timeout(
        move || {
            scheduled.set(false);
            flush_delta_buf(&buf, active, items, transcripts, models, session_models);
        },
        std::time::Duration::from_millis(50),
    );
}

pub(crate) fn format_relative_time(ts: i64, locale: Locale) -> String {
    if ts <= 0 {
        return String::new();
    }
    let now_ms = js_sys::Date::now();
    let ts_ms = if ts > 1_000_000_000_000 {
        ts as f64
    } else {
        ts as f64 * 1000.0
    };
    let secs = ((now_ms - ts_ms) / 1000.0).max(0.0) as i64;
    if secs < 45 {
        return t(locale, "time.just_now").into();
    }
    if secs < 3600 {
        return tf(
            locale,
            "time.minutes",
            &[("n", &(secs / 60).max(1).to_string())],
        );
    }
    if secs < 86_400 {
        return tf(locale, "time.hours", &[("n", &(secs / 3600).to_string())]);
    }
    tf(locale, "time.days", &[("n", &(secs / 86_400).to_string())])
}

/// Compact local clock for transcript metadata. Today's messages use `HH:mm`;
/// older messages add the date so revisiting long-running sessions is unambiguous.
pub(crate) fn format_message_time(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let ts_ms = if ts > 1_000_000_000_000 {
        ts as f64
    } else {
        ts as f64 * 1000.0
    };
    let date = js_sys::Date::new(&JsValue::from_f64(ts_ms));
    let now = js_sys::Date::new(&JsValue::from_f64(js_sys::Date::now()));
    let clock = format!("{:02}:{:02}", date.get_hours(), date.get_minutes());
    if date.get_full_year() == now.get_full_year()
        && date.get_month() == now.get_month()
        && date.get_date() == now.get_date()
    {
        clock
    } else if date.get_full_year() == now.get_full_year() {
        format!("{:02}-{:02} {clock}", date.get_month() + 1, date.get_date())
    } else {
        format!(
            "{:04}-{:02}-{:02} {clock}",
            date.get_full_year(),
            date.get_month() + 1,
            date.get_date()
        )
    }
}

pub(crate) fn format_message_datetime(ts: i64, locale: Locale) -> String {
    if ts <= 0 {
        return String::new();
    }
    let ts_ms = if ts > 1_000_000_000_000 {
        ts as f64
    } else {
        ts as f64 * 1000.0
    };
    js_sys::Date::new(&JsValue::from_f64(ts_ms))
        .to_locale_string(
            if locale == Locale::Zh {
                "zh-CN"
            } else {
                "en-US"
            },
            &JsValue::UNDEFINED,
        )
        .as_string()
        .unwrap_or_default()
}

#[cfg(test)]
mod terminal_chunk_tests {
    use super::*;

    #[test]
    fn carriage_return_overwrites_the_current_line() {
        let mut output = String::new();
        push_terminal_chunk(&mut output, "step 1\n10%\r20%\r100%");
        assert_eq!(output, "step 1\n100%");
    }

    #[test]
    fn crlf_stays_a_plain_newline() {
        let mut output = String::new();
        push_terminal_chunk(&mut output, "line1\r\nline2");
        assert_eq!(output, "line1\nline2");
    }

    #[test]
    fn carriage_return_split_across_chunks_is_deferred() {
        let mut output = String::new();
        push_terminal_chunk(&mut output, "done\r");
        push_terminal_chunk(&mut output, "\nnext");
        assert_eq!(output, "done\nnext");

        let mut output = String::new();
        push_terminal_chunk(&mut output, "50%\r");
        push_terminal_chunk(&mut output, "60%");
        assert_eq!(output, "60%");
    }

    #[test]
    fn output_is_capped_to_a_tail() {
        let mut output = String::new();
        for _ in 0..3 {
            push_terminal_chunk(&mut output, &"x".repeat(MAX_STREAM_OUTPUT_BYTES / 2));
        }
        push_terminal_chunk(&mut output, "tail");
        assert!(output.len() <= MAX_STREAM_OUTPUT_BYTES);
        assert!(output.ends_with("tail"));
    }

    #[test]
    fn fold_carriage_returns_keeps_only_final_progress_frames() {
        let folded = fold_carriage_returns("fetch\n|== |\r|====|\r|=====| done\nok");
        assert_eq!(folded, "fetch\n|=====| done\nok");
        assert_eq!(fold_carriage_returns("no progress"), "no progress");
    }

    #[test]
    fn stdout_chunks_fold_into_the_last_tool_output() {
        let mut items = vec![ChatItem::Tool {
            name: "shell".into(),
            ok: None,
            input: "make".into(),
            output: String::new(),
            started_at_ms: None,
            duration_ms: None,
            call_key: None,
        }];
        append_stdout_chunk(&mut items, "building\n10%\r".into());
        append_stdout_chunk(&mut items, "90%\r100%\n".into());
        match &items[0] {
            ChatItem::Tool { output, .. } => assert_eq!(output, "building\n100%\n"),
            _ => panic!("expected tool item"),
        }
    }

    #[test]
    fn tool_drafts_upsert_by_call_key_not_by_name() {
        let mut items = vec![ChatItem::User("go".into())];
        upsert_tool_draft(&mut items, "1:0".into(), "read".into(), "a.t".into());
        upsert_tool_draft(&mut items, "1:1".into(), "read".into(), "b.t".into());
        // Same tool name, two concurrent calls: two rows, each with its own key.
        assert_eq!(items.len(), 3);
        // A later snapshot updates only its own row.
        upsert_tool_draft(&mut items, "1:1".into(), "read".into(), "b.tsv".into());
        assert_eq!(items.len(), 3);
        let row = |key: &str| {
            items
                .iter()
                .find_map(|item| match item {
                    ChatItem::Tool {
                        call_key: Some(k),
                        input,
                        ..
                    } if k == key => Some(input.clone()),
                    _ => None,
                })
                .unwrap()
        };
        assert_eq!(row("1:0"), "a.t");
        assert_eq!(row("1:1"), "b.tsv");
    }

    #[test]
    fn clearing_tool_drafts_never_removes_real_rows() {
        let mut items = vec![
            ChatItem::User("go".into()),
            ChatItem::Tool {
                name: "read".into(),
                ok: Some(true),
                input: "done.tsv".into(),
                output: "contents".into(),
                started_at_ms: None,
                duration_ms: Some(3),
                call_key: None,
            },
        ];
        upsert_tool_draft(&mut items, "1:0".into(), "read".into(), "a.tsv".into());
        upsert_tool_draft(&mut items, "1:1".into(), "shell".into(), "make".into());
        // The real call for 1:0 starts: only that draft is dropped.
        clear_tool_drafts(&mut items, Some("1:0"));
        assert_eq!(items.len(), 3);
        assert!(items.iter().any(|item| matches!(
            item,
            ChatItem::Tool { call_key: Some(k), .. } if k == "1:1"
        )));
        // Turn boundary: every remaining draft goes, real output rows stay.
        clear_tool_drafts(&mut items, None);
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|item| matches!(
            item,
            ChatItem::Tool { ok: Some(true), output, .. } if output == "contents"
        )));
    }
}
