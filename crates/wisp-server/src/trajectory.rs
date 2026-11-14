//! A conversation as a trajectory: its saved messages folded into turns of
//! what the user asked, what the model said, what each tool did and what
//! each model call cost, with the totals over the whole conversation.
//!
//! The shape is the WebView desktop's (`wisp_dto::TrajectorySnapshotDto`).
//! The fold is this host's own, because the host saves different events:
//! a tool's outcome is recorded against its call id, and a model call's
//! usage is written when its turn ends.

use std::collections::HashMap;

use wisp_dto::{
    TrajectoryCellDto, TrajectorySnapshotDto, TrajectoryStatsDto, TrajectoryTurnDto,
    TrajectoryUsageDto,
};
use wisp_llm::{Message, Role};
use wisp_store::SessionUiEventRecord;

use crate::output::ToolOutcome;

/// A cell's one-line summary is clipped to this many characters.
const PREVIEW: usize = 120;

/// What the host saves beside the messages, as far as a trajectory reads it.
#[derive(serde::Deserialize)]
struct Saved {
    kind: String,
    #[serde(default)]
    outcome: Option<ToolOutcome>,
    #[serde(default)]
    round: i64,
    #[serde(default)]
    model: String,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    input: i64,
    #[serde(default)]
    output: i64,
    #[serde(default)]
    reasoning: i64,
    #[serde(default)]
    cached: i64,
}

struct Turn {
    started_at: Option<i64>,
    cells: Vec<TrajectoryCellDto>,
    /// When the model had something new to answer: the question, and each
    /// tool result. A model call is timed from the latest of these.
    ready: Vec<i64>,
}

pub(crate) fn fold(
    frame: &str,
    messages: &[Message],
    events: &[SessionUiEventRecord],
) -> TrajectorySnapshotDto {
    let mut turns: Vec<Turn> = Vec::new();
    // Where each tool call's cell is: (turn, cell).
    let mut calls: HashMap<String, (usize, usize)> = HashMap::new();
    for message in messages {
        let ts = millis(message.ts);
        if message.role == Role::System {
            continue;
        }
        let text = message.content.as_text();
        if message.role == Role::User {
            turns.push(Turn {
                started_at: ts,
                cells: vec![TrajectoryCellDto {
                    kind: "user".into(),
                    summary: preview(&text),
                    detail_output: (!text.trim().is_empty()).then_some(text),
                    ts,
                    ..Default::default()
                }],
                ready: ts.into_iter().collect(),
            });
            continue;
        }
        // A conversation whose first question was compacted away still has
        // somewhere to put what followed it.
        if turns.is_empty() {
            turns.push(Turn {
                started_at: ts,
                cells: vec![],
                ready: vec![],
            });
        }
        let at = turns.len() - 1;
        if message.role == Role::Tool {
            turns[at].ready.extend(ts);
        }
        let cells = &mut turns[at].cells;
        if message.role == Role::Assistant {
            if !text.trim().is_empty() {
                cells.push(TrajectoryCellDto {
                    kind: "assistant".into(),
                    summary: preview(&text),
                    detail_output: Some(text),
                    ts,
                    ..Default::default()
                });
            }
            for call in &message.tool_calls {
                let name = &call.function.name;
                let arguments = &call.function.arguments;
                cells.push(TrajectoryCellDto {
                    kind: "tool".into(),
                    summary: tool_summary(name, Some(arguments), None),
                    detail_input: Some(arguments.clone()),
                    ts,
                    ..Default::default()
                });
                if !call.id.is_empty() {
                    calls.insert(call.id.clone(), (at, cells.len() - 1));
                }
            }
            continue;
        }
        let name = message.tool_name.clone().unwrap_or_default();
        let place = message
            .tool_call_id
            .as_ref()
            .and_then(|id| calls.get(id))
            .filter(|(turn, _)| *turn == at);
        match place {
            Some(&(_, index)) => {
                let cell = &mut cells[index];
                cell.summary = tool_summary(&name, cell.detail_input.as_deref(), Some(&text));
                cell.detail_output = Some(text);
            }
            // A result whose call is gone is kept as a step of its own.
            None => cells.push(TrajectoryCellDto {
                kind: "tool".into(),
                summary: tool_summary(&name, None, Some(&text)),
                detail_output: Some(text),
                ts,
                ..Default::default()
            }),
        }
    }

    // The turn the latest model call went to, and its round.
    let mut latest_call: Option<(usize, i64)> = None;
    for record in events {
        let Ok(saved) = serde_json::from_str::<Saved>(&record.event_json) else {
            continue;
        };
        match (saved.kind.as_str(), saved.outcome) {
            ("ServerToolResult", Some(outcome)) => {
                if let Some(&(turn, index)) = calls.get(&outcome.call_id) {
                    let cell = &mut turns[turn].cells[index];
                    cell.ok = Some(outcome.ok);
                    cell.is_error = !outcome.ok;
                    cell.duration_ms =
                        (outcome.duration_ms > 0).then_some(outcome.duration_ms as i64);
                }
            }
            ("Usage", _) => {
                let ts = millis(saved.created_at).or(record.created_at);
                if turns.is_empty() {
                    turns.push(Turn {
                        started_at: ts,
                        cells: vec![],
                        ready: vec![],
                    });
                }
                let mut at = turn_at(&turns, ts);
                // Times are whole seconds, so a turn's last call and the
                // start of a message that was waiting for it can share one.
                // A turn's calls are saved with rising rounds: a round that
                // rose still belongs to the turn before.
                if let Some((turn, round)) = latest_call {
                    if saved.round > round && at == turn + 1 && turns[at].started_at == ts {
                        at = turn;
                    }
                }
                latest_call = Some((at, saved.round));
                turns[at].cells.push(TrajectoryCellDto {
                    kind: "usage".into(),
                    summary: format!(
                        "round {} · {} in / {} out",
                        saved.round, saved.input, saved.output
                    ),
                    ts,
                    usage: Some(TrajectoryUsageDto {
                        round: saved.round,
                        model: (!saved.model.is_empty()).then_some(saved.model),
                        input_tokens: saved.input,
                        output_tokens: saved.output,
                        reasoning_tokens: saved.reasoning,
                        cached_input_tokens: saved.cached,
                    }),
                    ..Default::default()
                });
            }
            _ => {}
        }
    }

    let mut stats = TrajectoryStatsDto::default();
    let mut model = None;
    // What the calls that could be timed wrote: the speed is theirs alone.
    let mut timed_output = 0;
    let turns: Vec<_> = turns
        .into_iter()
        .enumerate()
        .map(|(index, mut turn)| {
            // In the order things happened; a step with no time keeps its
            // place after those that have one.
            turn.cells.sort_by_key(|cell| cell.ts.unwrap_or(i64::MAX));
            // The model call before this one, when it ended.
            let mut answered = None;
            for cell in &turn.cells {
                if let Some(usage) = &cell.usage {
                    stats.steps += 1;
                    // A model call ran from when it last had something new
                    // to read, or from the call before it, to when it ended.
                    // A call from before the turn had anything to answer
                    // has nothing to be timed from.
                    if let Some(ts) = cell.ts {
                        let asked = turn.ready.iter().copied().filter(|ready| *ready <= ts);
                        if let Some(asked) = asked.max() {
                            stats.llm_ms += ts - answered.map_or(asked, |before| asked.max(before));
                            timed_output += usage.output_tokens;
                        }
                        answered = Some(ts);
                    }
                    stats.input_tokens += usage.input_tokens;
                    stats.output_tokens += usage.output_tokens;
                    stats.cached_input_tokens += usage.cached_input_tokens;
                    if usage.model.is_some() {
                        model = usage.model.clone();
                    }
                }
                stats.tool_ms += cell.duration_ms.unwrap_or_default();
            }
            TrajectoryTurnDto {
                index: index as i64 + 1,
                started_at: turn.started_at,
                cells: turn.cells,
            }
        })
        .collect();
    stats.turns = turns.len() as i64;
    let read = stats.input_tokens + stats.cached_input_tokens;
    if read > 0 {
        stats.cache_hit_pct = Some(stats.cached_input_tokens as f64 / read as f64 * 100.0);
    }
    if stats.llm_ms > 0 {
        stats.tokens_per_sec = Some(timed_output as f64 / (stats.llm_ms as f64 / 1000.0));
    }
    TrajectorySnapshotDto {
        frame_id: frame.into(),
        model,
        turns,
        stats,
    }
}

/// Saved times are Unix seconds, and zero means none was saved.
fn millis(seconds: i64) -> Option<i64> {
    (seconds > 0).then(|| seconds.saturating_mul(1000))
}

/// The turn that was open at `ts`: the last one that had started by then.
/// With no time to go by it is the last turn; before every turn, the first.
fn turn_at(turns: &[Turn], ts: Option<i64>) -> usize {
    match ts {
        Some(ts) => turns
            .iter()
            .rposition(|turn| turn.started_at.is_some_and(|started| started <= ts))
            .unwrap_or(0),
        None => turns.len() - 1,
    }
}

/// The first line of `text`, clipped.
fn preview(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    let mut chars = line.chars();
    let clipped: String = chars.by_ref().take(PREVIEW).collect();
    if chars.next().is_some() {
        format!("{clipped}…")
    } else {
        clipped
    }
}

fn tool_summary(name: &str, arguments: Option<&str>, result: Option<&str>) -> String {
    let mut summary = match arguments {
        Some(arguments) if !arguments.trim().is_empty() => format!("{name} {}", preview(arguments)),
        _ => name.to_owned(),
    };
    if let Some(result) = result {
        summary = format!("{summary} → {}", preview(result));
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wisp_llm::{FunctionCall, ToolCall};

    fn at(mut message: Message, seconds: i64) -> Message {
        message.ts = seconds;
        message
    }

    fn calling(id: &str, name: &str, arguments: &str, seconds: i64) -> Message {
        let mut message = at(Message::assistant(""), seconds);
        message.tool_calls = vec![ToolCall {
            id: id.into(),
            kind: "function".into(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }];
        message
    }

    fn event(seq: i64, value: serde_json::Value) -> SessionUiEventRecord {
        SessionUiEventRecord {
            seq,
            created_at: None,
            event_json: value.to_string(),
        }
    }

    fn usage(
        seq: i64,
        round: i64,
        seconds: i64,
        input: i64,
        output: i64,
        cached: i64,
    ) -> SessionUiEventRecord {
        event(
            seq,
            json!({"kind":"Usage","frame_id":"f","round":round,"model":"m-1","created_at":seconds,
                "input":input,"output":output,"reasoning":0,"cached":cached,
                "ctx_tokens":0,"max_context":0}),
        )
    }

    #[test]
    fn a_conversation_folds_into_turns_of_steps_with_their_cost() {
        let messages = [
            Message::system("You are Wisp."),
            at(Message::user("count the reads\nin sample A"), 100),
            calling("c1", "shell", r#"{"command":"wc -l reads.fq"}"#, 102),
            at(Message::tool("c1", "shell", "400 reads.fq"), 103),
            at(Message::assistant("There are 100 reads."), 105),
            at(Message::user("thanks"), 200),
            at(Message::assistant("Any time."), 201),
        ];
        let events = [
            event(
                0,
                json!({"kind":"ServerToolResult","outcome":{"call_id":"c1","ok":true,"duration_ms":750}}),
            ),
            // Written when each turn ended; the round starts again each turn.
            usage(1, 0, 102, 1_000, 40, 0),
            usage(2, 1, 105, 300, 60, 900),
            usage(3, 0, 201, 100, 10, 1_200),
            // Anything else the conversation saved is not a step.
            event(4, json!({"kind":"Done"})),
            SessionUiEventRecord {
                seq: 5,
                created_at: None,
                event_json: "not json".into(),
            },
        ];
        let snapshot = fold("f", &messages, &events);
        assert_eq!(snapshot.frame_id, "f");
        assert_eq!(snapshot.model.as_deref(), Some("m-1"));
        assert_eq!(snapshot.turns.len(), 2);

        let first = &snapshot.turns[0];
        assert_eq!((first.index, first.started_at), (1, Some(100_000)));
        let kinds: Vec<_> = first.cells.iter().map(|cell| cell.kind.as_str()).collect();
        assert_eq!(kinds, ["user", "tool", "usage", "assistant", "usage"]);
        // A question is summarised by its first line and kept whole.
        assert_eq!(first.cells[0].summary, "count the reads");
        assert_eq!(
            first.cells[0].detail_output.as_deref(),
            Some("count the reads\nin sample A")
        );
        let tool = &first.cells[1];
        assert_eq!(
            tool.summary,
            r#"shell {"command":"wc -l reads.fq"} → 400 reads.fq"#
        );
        assert_eq!(tool.detail_output.as_deref(), Some("400 reads.fq"));
        assert_eq!(
            (tool.ok, tool.is_error, tool.duration_ms),
            (Some(true), false, Some(750))
        );
        assert_eq!(first.cells[2].summary, "round 0 · 1000 in / 40 out");

        // The second turn's call is its own, though it is also round 0.
        let second = &snapshot.turns[1];
        let kinds: Vec<_> = second.cells.iter().map(|cell| cell.kind.as_str()).collect();
        assert_eq!(kinds, ["user", "assistant", "usage"]);

        let stats = &snapshot.stats;
        assert_eq!((stats.turns, stats.steps), (2, 3));
        // 100→102, 103→105 and 200→201.
        assert_eq!(stats.llm_ms, 5_000);
        assert_eq!(stats.tool_ms, 750);
        assert_eq!(
            (
                stats.input_tokens,
                stats.output_tokens,
                stats.cached_input_tokens
            ),
            (1_400, 110, 2_100)
        );
        assert_eq!(stats.cache_hit_pct, Some(60.0));
        assert_eq!(stats.tokens_per_sec, Some(22.0));
    }

    #[test]
    fn a_call_stays_with_its_own_turn_and_is_timed_only_from_what_it_answered() {
        let messages = [
            at(Message::user("slow"), 100),
            calling("c1", "shell", "{}", 101),
            at(Message::tool("c1", "shell", "finished"), 146),
            at(Message::assistant("Done."), 147),
            // Written while the first turn ran, and sent the second it ended.
            at(Message::user("hello"), 147),
            at(Message::assistant("Hi."), 148),
        ];
        let events = [
            // Saved long before anything was asked here.
            usage(0, 0, 40, 5, 5, 0),
            usage(1, 3, 60, 5, 5, 0),
            usage(2, 0, 101, 10, 5, 0),
            usage(3, 1, 147, 10, 5, 0),
            usage(4, 0, 148, 10, 5, 0),
        ];
        let snapshot = fold("f", &messages, &events);
        let kinds = |turn: usize| -> Vec<_> {
            snapshot.turns[turn]
                .cells
                .iter()
                .map(|cell| cell.kind.as_str())
                .collect()
        };
        // The call that ended at 147 answered the first turn's tool.
        assert_eq!(
            kinds(0),
            [
                "usage",
                "usage",
                "user",
                "tool",
                "usage",
                "assistant",
                "usage"
            ]
        );
        assert_eq!(kinds(1), ["user", "assistant", "usage"]);
        assert_eq!(snapshot.stats.steps, 5);
        // 100→101, 146→147 and 147→148; the two early calls are not timed,
        // so what they wrote is not in the speed either.
        assert_eq!(snapshot.stats.llm_ms, 3_000);
        assert_eq!(snapshot.stats.output_tokens, 25);
        assert_eq!(snapshot.stats.tokens_per_sec, Some(5.0));
    }

    #[test]
    fn what_is_missing_is_left_out_rather_than_guessed() {
        // Nothing saved: no turns, no rates.
        let empty = fold("f", &[], &[]);
        assert!(empty.turns.is_empty() && empty.model.is_none());
        assert_eq!(empty.stats.cache_hit_pct, None);
        assert_eq!(empty.stats.tokens_per_sec, None);

        // The question was compacted away, the tool failed, its call is
        // gone for another result, and no time was saved for anything.
        let messages = [
            calling("c1", "shell", "", 0),
            at(Message::tool("c1", "shell", "exit 1"), 0),
            at(Message::tool("lost", "read", "x".repeat(200)), 0),
        ];
        let events = [
            event(
                0,
                json!({"kind":"ServerToolResult","outcome":{"call_id":"c1","ok":false,"duration_ms":0}}),
            ),
            event(
                1,
                json!({"kind":"ServerToolResult","outcome":{"call_id":"never","ok":true,"duration_ms":9}}),
            ),
            usage(2, 0, 0, 10, 5, 0),
        ];
        let snapshot = fold("f", &messages, &events);
        assert_eq!(snapshot.turns.len(), 1);
        let cells = &snapshot.turns[0].cells;
        assert_eq!(snapshot.turns[0].started_at, None);
        assert_eq!(cells[0].summary, "shell → exit 1");
        assert_eq!(
            (cells[0].ok, cells[0].is_error, cells[0].duration_ms),
            (Some(false), true, None)
        );
        assert_eq!(cells[1].summary, format!("read → {}…", "x".repeat(120)));
        assert_eq!(cells[2].kind, "usage");
        // No times, so no model time and no rate from it.
        assert_eq!(snapshot.stats.llm_ms, 0);
        assert_eq!(snapshot.stats.tokens_per_sec, None);
        assert_eq!(snapshot.stats.cache_hit_pct, Some(0.0));
    }
}
