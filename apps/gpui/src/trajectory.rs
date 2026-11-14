//! The trajectory of a conversation, as the page words it: the totals at
//! the top, a heading for each turn and one line for each step. It holds no
//! graphics state; the host folds the conversation and this only reads it.
use crate::i18n::tr;
use chrono::{TimeZone, Timelike};
use wisp_dto::{TrajectoryCellDto, TrajectorySnapshotDto, TrajectoryStatsDto, TrajectoryTurnDto};

use crate::controls::compact;

/// As much of a step's text as the page shows when it is opened.
pub const DETAIL_LIMIT: usize = 4_000;

/// A length of time short enough to sit in a line: 850 ms, 2.4 s, 3 min 05 s.
pub fn duration(ms: i64) -> String {
    let ms = ms.max(0);
    match ms {
        0..=999 => format!("{ms} ms"),
        1_000..=59_949 => format!("{:.1} s", ms as f64 / 1_000.0),
        _ => {
            let seconds = (ms + 500) / 1_000;
            format!("{} min {:02} s", seconds / 60, seconds % 60)
        }
    }
}

fn tokens(count: i64) -> String {
    compact(count.max(0) as u64)
}

/// The totals over the whole conversation, as labelled figures. A figure
/// that could not be worked out is left out instead of shown as zero.
pub fn totals(stats: &TrajectoryStatsDto) -> Vec<(&'static str, String)> {
    let mut totals = vec![
        (tr("Turns"), stats.turns.to_string()),
        (tr("Model calls"), stats.steps.to_string()),
    ];
    if stats.llm_ms > 0 {
        totals.push((tr("Model time"), duration(stats.llm_ms)));
    }
    if stats.tool_ms > 0 {
        totals.push((tr("Tool time"), duration(stats.tool_ms)));
    }
    totals.push((tr("Input"), tokens(stats.input_tokens)));
    totals.push((tr("Output"), tokens(stats.output_tokens)));
    if stats.cached_input_tokens > 0 {
        let cached = tokens(stats.cached_input_tokens);
        totals.push((
            tr("Cached"),
            match stats.cache_hit_pct {
                Some(hit) => format!("{cached} ({hit:.0}%)"),
                None => cached,
            },
        ));
    }
    if let Some(speed) = stats.tokens_per_sec {
        // A slow model is not shown as standing still.
        let speed = if speed < 10.0 {
            format!("{speed:.1}")
        } else {
            format!("{speed:.0}")
        };
        totals.push((tr("Speed"), format!("{speed} tokens/s")));
    }
    totals
}

/// The line under the title: which model, or that nothing was asked yet.
pub fn subtitle(snapshot: &TrajectorySnapshotDto) -> String {
    if snapshot.turns.is_empty() {
        return tr("Nothing has been asked in this conversation yet.").into();
    }
    let steps = "Every turn's steps in the order they happened. Times are to the second.";
    match &snapshot.model {
        Some(model) => format!("{steps} Latest model: {model}."),
        None => steps.into(),
    }
}

/// A turn's heading: its number, when it began in local time, and what it
/// took.
pub fn turn_heading<Tz: TimeZone>(turn: &TrajectoryTurnDto, zone: &Tz) -> String {
    turn_heading_in(crate::i18n::current(), turn, zone)
}

fn turn_heading_in<Tz: TimeZone>(
    locale: crate::i18n::Locale,
    turn: &TrajectoryTurnDto,
    zone: &Tz,
) -> String {
    let chinese = locale == crate::i18n::Locale::Zh;
    let mut parts = vec![if chinese {
        format!("第 {} 轮", turn.index)
    } else {
        format!("Turn {}", turn.index)
    }];
    if let Some(local) = turn
        .started_at
        .and_then(|ms| zone.timestamp_millis_opt(ms).earliest())
    {
        parts.push(format!("{:02}:{:02}", local.hour(), local.minute()));
    }
    let count = |kind: &str| turn.cells.iter().filter(|cell| cell.kind == kind).count();
    let plural = |count: usize, one: &str, measured: &str| match (chinese, count) {
        (true, _) => format!("{count} {measured}"),
        (false, 1) => format!("1 {one}"),
        (false, _) => format!("{count} {one}s"),
    };
    let (calls, tools) = (count("usage"), count("tool"));
    if calls > 0 {
        parts.push(plural(calls, "model call", "次模型调用"));
    }
    if tools > 0 {
        parts.push(plural(tools, "tool", "次工具调用"));
    }
    let used: i64 = turn
        .cells
        .iter()
        .filter_map(|cell| cell.usage.as_ref())
        .map(|usage| usage.input_tokens + usage.output_tokens)
        .sum();
    if used > 0 {
        parts.push(format!(
            "{} {}",
            tokens(used),
            if chinese { "token" } else { "tokens" }
        ));
    }
    parts.join(" · ")
}

/// How a step went, for the colour of its label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Failed,
    Quiet,
}

/// One step as a row: who or what it was, what it did, and what it took.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    pub label: &'static str,
    pub text: String,
    /// Shown at the end of the row, when there is something to say.
    pub note: Option<String>,
    pub tone: Tone,
}

pub fn step(cell: &TrajectoryCellDto) -> Step {
    let plain = |label, tone| Step {
        label,
        text: cell.summary.clone(),
        note: None,
        tone,
    };
    match cell.kind.as_str() {
        "user" => plain(tr("You"), Tone::Plain),
        "assistant" => plain("Wisp", Tone::Plain),
        "tool" => {
            let failed = cell.is_error || cell.ok == Some(false);
            let took = cell.duration_ms.map(duration);
            Step {
                note: match (failed, took) {
                    (true, Some(took)) => Some(format!("failed · {took}")),
                    (true, None) => Some("failed".into()),
                    (false, took) => took,
                },
                ..plain(tr("Tool"), if failed { Tone::Failed } else { Tone::Plain })
            }
        }
        "usage" => match &cell.usage {
            Some(usage) => {
                let mut parts = vec![
                    format!("{} in", tokens(usage.input_tokens)),
                    format!("{} out", tokens(usage.output_tokens)),
                ];
                if usage.reasoning_tokens > 0 {
                    parts.push(format!("{} thinking", tokens(usage.reasoning_tokens)));
                }
                if usage.cached_input_tokens > 0 {
                    parts.push(format!("{} cached", tokens(usage.cached_input_tokens)));
                }
                Step {
                    label: tr("Model call"),
                    text: parts.join(" · "),
                    note: usage.model.clone(),
                    tone: Tone::Quiet,
                }
            }
            None => plain(tr("Model call"), Tone::Quiet),
        },
        // A kind this build does not know is still a step that happened.
        _ => plain(tr("Step"), Tone::Quiet),
    }
}

/// What opening a step shows: what a tool was given and what came back, or
/// the whole of what was said. Nothing when the row already says it all.
pub fn detail(cell: &TrajectoryCellDto) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(input) = cell.detail_input.as_deref().filter(|text| !text.is_empty()) {
        parts.push(format!("Input\n{}", clipped(input)));
    }
    if let Some(output) = cell
        .detail_output
        .as_deref()
        .filter(|text| !text.is_empty())
    {
        parts.push(match cell.kind.as_str() {
            "tool" => format!("Result\n{}", clipped(output)),
            _ => clipped(output),
        });
    }
    let text = parts.join("\n\n");
    // A one-line message is already whole in its row.
    (!text.is_empty() && text.trim() != cell.summary.trim()).then_some(text)
}

fn clipped(text: &str) -> String {
    let mut chars = text.chars();
    let kept: String = chars.by_ref().take(DETAIL_LIMIT).collect();
    match chars.count() {
        0 => kept,
        more => format!("{kept}\n… {more} more characters"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;
    use wisp_dto::TrajectoryUsageDto;

    fn cell(kind: &str, summary: &str) -> TrajectoryCellDto {
        TrajectoryCellDto {
            kind: kind.into(),
            summary: summary.into(),
            ..Default::default()
        }
    }

    fn usage(input: i64, output: i64, reasoning: i64, cached: i64) -> TrajectoryCellDto {
        TrajectoryCellDto {
            usage: Some(TrajectoryUsageDto {
                round: 0,
                model: Some("m-1".into()),
                input_tokens: input,
                output_tokens: output,
                reasoning_tokens: reasoning,
                cached_input_tokens: cached,
            }),
            ..cell("usage", "round 0")
        }
    }

    #[test]
    fn durations_read_in_the_unit_that_fits() {
        assert_eq!(duration(-5), "0 ms");
        assert_eq!(duration(850), "850 ms");
        assert_eq!(duration(2_400), "2.4 s");
        assert_eq!(duration(59_900), "59.9 s");
        // Rounding never shows sixty seconds.
        assert_eq!(duration(59_960), "1 min 00 s");
        assert_eq!(duration(185_000), "3 min 05 s");
    }

    #[test]
    fn totals_leave_out_what_was_not_measured() {
        let measured = TrajectoryStatsDto {
            turns: 2,
            steps: 3,
            llm_ms: 5_000,
            tool_ms: 750,
            input_tokens: 1_400,
            output_tokens: 110,
            cached_input_tokens: 2_100,
            cache_hit_pct: Some(60.0),
            tokens_per_sec: Some(22.0),
        };
        assert_eq!(
            totals(&measured),
            [
                ("Turns", "2".to_owned()),
                ("Model calls", "3".into()),
                ("Model time", "5.0 s".into()),
                ("Tool time", "750 ms".into()),
                ("Input", "1.4k".into()),
                ("Output", "110".into()),
                ("Cached", "2.1k (60%)".into()),
                ("Speed", "22 tokens/s".into()),
            ]
        );
        // No times and no cache: no made-up zero rates.
        let bare = TrajectoryStatsDto {
            turns: 1,
            steps: 1,
            input_tokens: 10,
            output_tokens: 5,
            ..Default::default()
        };
        let labels: Vec<_> = totals(&bare).into_iter().map(|(label, _)| label).collect();
        assert_eq!(labels, ["Turns", "Model calls", "Input", "Output"]);
        let slow = TrajectoryStatsDto {
            tokens_per_sec: Some(0.44),
            ..bare
        };
        assert_eq!(totals(&slow).last().unwrap().1, "0.4 tokens/s");
    }

    #[test]
    fn a_turn_is_headed_by_when_it_began_and_what_it_took() {
        let zone = FixedOffset::east_opt(8 * 3600).unwrap();
        let turn = TrajectoryTurnDto {
            index: 3,
            // 2026-11-14 06:05:00 UTC.
            started_at: Some(1_794_636_300_000),
            cells: vec![
                cell("user", "count"),
                cell("tool", "shell"),
                usage(1_000, 40, 0, 0),
                cell("assistant", "done"),
                usage(300, 60, 0, 900),
            ],
        };
        assert_eq!(
            turn_heading(&turn, &zone),
            "Turn 3 · 14:05 · 2 model calls · 1 tool · 1.4k tokens"
        );
        // A turn with nothing saved about it but the question.
        let bare = TrajectoryTurnDto {
            index: 1,
            started_at: None,
            cells: vec![cell("user", "hello")],
        };
        assert_eq!(turn_heading(&bare, &zone), "Turn 1");
        assert_eq!(
            turn_heading_in(crate::i18n::Locale::Zh, &turn, &zone),
            "第 3 轮 · 14:05 · 2 次模型调用 · 1 次工具调用 · 1.4k token"
        );
    }

    #[test]
    fn each_step_says_who_did_what_and_what_it_took() {
        assert_eq!(
            step(&cell("user", "count the reads")),
            Step {
                label: "You",
                text: "count the reads".into(),
                note: None,
                tone: Tone::Plain,
            }
        );
        assert_eq!(step(&cell("assistant", "done")).label, "Wisp");

        let mut tool = cell("tool", "shell {} → ok");
        tool.ok = Some(true);
        tool.duration_ms = Some(750);
        assert_eq!(step(&tool).note.as_deref(), Some("750 ms"));
        assert_eq!(step(&tool).tone, Tone::Plain);
        tool.ok = Some(false);
        tool.is_error = true;
        assert_eq!(step(&tool).note.as_deref(), Some("failed · 750 ms"));
        assert_eq!(step(&tool).tone, Tone::Failed);
        tool.duration_ms = None;
        assert_eq!(step(&tool).note.as_deref(), Some("failed"));
        // A tool whose outcome was not saved claims neither.
        assert_eq!(step(&cell("tool", "read")).note, None);

        let call = step(&usage(1_000, 40, 12, 900));
        assert_eq!(call.label, "Model call");
        assert_eq!(call.text, "1.0k in · 40 out · 12 thinking · 900 cached");
        assert_eq!(call.note.as_deref(), Some("m-1"));
        assert_eq!(step(&usage(10, 5, 0, 0)).text, "10 in · 5 out");
        assert_eq!(step(&cell("later", "something new")).label, "Step");
    }

    #[test]
    fn opening_a_step_shows_what_its_row_could_not() {
        // A one-line message is whole in its row.
        let mut said = cell("assistant", "done");
        said.detail_output = Some("done".into());
        assert_eq!(detail(&said), None);
        // The row trims what was said; that is not more to show.
        said.detail_output = Some("done \n".into());
        assert_eq!(detail(&said), None);
        said.detail_output = Some("done\nand more".into());
        assert_eq!(detail(&said).as_deref(), Some("done\nand more"));

        let mut tool = cell("tool", "shell");
        tool.detail_input = Some(r#"{"command":"ls"}"#.into());
        tool.detail_output = Some("a\nb".into());
        assert_eq!(
            detail(&tool).as_deref(),
            Some("Input\n{\"command\":\"ls\"}\n\nResult\na\nb")
        );
        // A long result is cut, and says by how much.
        tool.detail_input = None;
        tool.detail_output = Some("é".repeat(DETAIL_LIMIT + 7));
        let shown = detail(&tool).unwrap();
        assert!(shown.ends_with("\n… 7 more characters"), "{shown}");
        assert_eq!(detail(&cell("usage", "round 0")), None);
    }

    #[test]
    fn the_subtitle_names_the_model_or_says_nothing_was_asked() {
        let mut snapshot = TrajectorySnapshotDto::default();
        assert_eq!(
            subtitle(&snapshot),
            "Nothing has been asked in this conversation yet."
        );
        snapshot.turns.push(TrajectoryTurnDto::default());
        assert!(!subtitle(&snapshot).contains("Latest model"));
        snapshot.model = Some("m-1".into());
        assert!(subtitle(&snapshot).ends_with("Latest model: m-1."));
    }
}
