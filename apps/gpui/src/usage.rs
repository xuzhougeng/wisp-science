//! Figures for the Usage page: totals, the last days as a strip of bars, and
//! shares. No graphics state.
use wisp_server::desktop::UsageView;

/// Days the activity strip shows.
pub const DAYS: usize = 30;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    /// Includes what was read from the cache.
    pub input: u64,
    pub output: u64,
    pub cached: u64,
    pub reasoning: u64,
}

/// What every project used, together.
pub fn totals(view: &UsageView) -> Totals {
    view.projects
        .iter()
        .fold(Totals::default(), |sum, row| Totals {
            input: sum.input + row.input,
            output: sum.output + row.output,
            cached: sum.cached + row.cached,
            reasoning: sum.reasoning + row.reasoning,
        })
}

/// Rows the skill and MCP ranking shows; past that, the rest are one row.
pub const TOOL_ROWS: usize = 10;

/// The skill and MCP ranking as the WebView shows it: `(kind, name,
/// calls)`, most calls first, and when there are more than [`TOOL_ROWS`],
/// the first nine and the rest together as one row of kind `other`.
pub fn tool_rows(view: &UsageView) -> Vec<(String, String, i64)> {
    let mut rows: Vec<(String, String, i64)> = view
        .tools
        .iter()
        .filter(|tool| tool.calls > 0)
        .map(|tool| (tool.kind.clone(), tool.name.clone(), tool.calls))
        .collect();
    if rows.len() > TOOL_ROWS {
        let rest: i64 = rows.drain(TOOL_ROWS - 1..).map(|(_, _, calls)| calls).sum();
        rows.push(("other".into(), crate::i18n::tr("Other").into(), rest));
    }
    rows
}

/// The last `count` days of the calendar, oldest first.
pub fn recent(days: &[(String, u64)], count: usize) -> &[(String, u64)] {
    &days[days.len().saturating_sub(count)..]
}

/// How tall each day's bar is when the busiest day is `full` tall. A day
/// with any use at all is at least visible; a day with none has no bar.
pub fn heights(days: &[(String, u64)], full: f32) -> Vec<f32> {
    let most = days.iter().map(|(_, tokens)| *tokens).max().unwrap_or(0);
    days.iter()
        .map(|(_, tokens)| match tokens {
            0 => 0.,
            tokens => (*tokens as f32 / most as f32 * full).max(2.),
        })
        .collect()
}

/// The day that used the most, if any day used anything. The earliest of
/// equals.
pub fn busiest(days: &[(String, u64)]) -> Option<&(String, u64)> {
    days.iter()
        .filter(|(_, tokens)| *tokens > 0)
        .reduce(|best, day| if day.1 > best.1 { day } else { best })
}

/// `part` of `whole` in whole percent; a sliver that is not nothing shows
/// as 1.
pub fn percent(part: u64, whole: u64) -> u8 {
    if whole == 0 || part == 0 {
        return 0;
    }
    ((part as f64 / whole as f64 * 100.).round() as u8).clamp(1, 100)
}

/// A calendar date as it is said: `2026-11-04` is `4 Nov`.
pub fn day_label(date: &str) -> String {
    day_label_in(crate::i18n::current(), date)
}

fn day_label_in(locale: crate::i18n::Locale, date: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut parts = date
        .split('-')
        .skip(1)
        .map(|part| part.parse::<usize>().ok());
    match (parts.next().flatten(), parts.next().flatten()) {
        (Some(month @ 1..=12), Some(day)) if locale == crate::i18n::Locale::Zh => {
            format!("{month}月{day}日")
        }
        (Some(month @ 1..=12), Some(day)) => format!("{day} {}", MONTHS[month - 1]),
        _ => date.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wisp_server::desktop::ProjectUsage;

    #[test]
    fn the_tool_ranking_keeps_nine_and_counts_the_rest_together() {
        let view = |count: i64| UsageView {
            tools: (0..count)
                .map(|n| wisp_server::desktop::ToolUsage {
                    kind: "skill".into(),
                    name: format!("s{n}"),
                    calls: 100 - n,
                })
                .collect(),
            ..UsageView::default()
        };
        assert_eq!(tool_rows(&view(3)).len(), 3);
        let many = tool_rows(&view(12));
        assert_eq!(many.len(), TOOL_ROWS);
        assert_eq!(many[8].1, "s8");
        // s9, s10 and s11: 91 + 90 + 89.
        assert_eq!(many[9], ("other".into(), "Other".into(), 270));
    }

    fn days(tokens: &[u64]) -> Vec<(String, u64)> {
        tokens
            .iter()
            .enumerate()
            .map(|(index, tokens)| (format!("2026-11-{:02}", index + 1), *tokens))
            .collect()
    }

    #[test]
    fn totals_add_every_project() {
        let row = |input, output, cached, reasoning| ProjectUsage {
            project_id: "p".into(),
            name: "P".into(),
            conversations: 1,
            input,
            output,
            reasoning,
            cached,
        };
        let view = UsageView {
            projects: vec![row(100, 20, 80, 5), row(1_000, 200, 0, 0)],
            ..Default::default()
        };
        assert_eq!(
            totals(&view),
            Totals {
                input: 1_100,
                output: 220,
                cached: 80,
                reasoning: 5
            }
        );
        assert_eq!(totals(&UsageView::default()), Totals::default());
    }

    #[test]
    fn the_strip_shows_the_last_days_scaled_to_the_busiest() {
        let all = days(&[5, 0, 100, 1, 50]);
        assert_eq!(recent(&all, 3), &all[2..]);
        assert_eq!(recent(&all, 30), &all[..]);
        // Nothing is no bar; almost nothing is still a bar.
        assert_eq!(heights(&all, 60.), [3., 0., 60., 2., 30.]);
        assert_eq!(heights(&days(&[0, 0]), 60.), [0., 0.]);
        assert!(heights(&[], 60.).is_empty());
        assert_eq!(busiest(&all), Some(&("2026-11-03".to_string(), 100)));
        assert_eq!(
            busiest(&days(&[7, 7])),
            Some(&("2026-11-01".to_string(), 7))
        );
        assert_eq!(busiest(&days(&[0, 0])), None);
    }

    #[test]
    fn shares_round_but_never_hide_a_sliver() {
        assert_eq!(percent(50, 200), 25);
        assert_eq!(percent(1, 1_000_000), 1);
        assert_eq!(percent(0, 10), 0);
        assert_eq!(percent(10, 0), 0);
        assert_eq!(percent(10, 10), 100);
    }

    #[test]
    fn dates_are_said_the_chinese_way_in_chinese() {
        let zh = crate::i18n::Locale::Zh;
        assert_eq!(day_label_in(zh, "2026-11-04"), "11月4日");
        // What is not a date is shown as it came.
        assert_eq!(day_label_in(zh, "soon"), "soon");
    }

    #[test]
    fn dates_are_said_the_short_way() {
        assert_eq!(day_label("2026-11-04"), "4 Nov");
        assert_eq!(day_label("2026-01-31"), "31 Jan");
        assert_eq!(day_label("not a date"), "not a date");
        assert_eq!(day_label("2026-13-01"), "2026-13-01");
    }
}
