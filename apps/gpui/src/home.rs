//! Ordering and wording for the home page, kept apart from GPUI so it is
//! covered without a graphics context.
use wisp_dto::ProjectSummary;

use crate::present::Tone;

/// Which full-window surface is showing under any open layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Screen {
    /// Every project and the most recent conversations.
    #[default]
    Home,
    /// One project's conversations.
    Project,
}

/// Starred projects first, then the most recently used. Names break ties so
/// the order never flickers between refreshes.
pub fn ordered(mut projects: Vec<ProjectSummary>) -> Vec<ProjectSummary> {
    projects.sort_by(|a, b| {
        b.starred
            .cmp(&a.starred)
            .then(b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    projects
}

fn plural(count: i64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

pub fn counts(project: &ProjectSummary) -> String {
    format!(
        "{} · {}",
        plural(project.session_count, "conversation", "conversations"),
        plural(project.artifact_count, "artifact", "artifacts")
    )
}

/// What is happening in a project right now; empty when it is idle.
pub fn activity(project: &ProjectSummary) -> Vec<(String, Tone)> {
    let mut chips = Vec::new();
    if project.running_count > 0 {
        chips.push((format!("{} running", project.running_count), Tone::Active));
    }
    if project.needs_you_count > 0 {
        chips.push((
            format!("{} waiting", project.needs_you_count),
            Tone::Attention,
        ));
    }
    chips
}

/// How long ago `then` was, as a short label. Both are Unix seconds; a
/// millisecond value from an older row is accepted too.
pub fn age(now: i64, then: i64) -> String {
    let then = if then > 100_000_000_000 {
        then / 1000
    } else {
        then
    };
    if then <= 0 {
        return String::new();
    }
    let seconds = (now - then).max(0);
    match seconds {
        0..=59 => "just now".into(),
        60..=3_599 => format!("{} min ago", seconds / 60),
        3_600..=86_399 => format!("{} h ago", seconds / 3_600),
        86_400..=1_209_599 => plural(seconds / 86_400, "day", "days") + " ago",
        1_209_600..=5_183_999 => format!("{} weeks ago", seconds / 604_800),
        5_184_000..=31_535_999 => format!("{} months ago", seconds / 2_592_000),
        _ => plural(seconds / 31_536_000, "year", "years") + " ago",
    }
}

/// The folder name shown for a project path, without losing which disk or
/// home it is under when the full path is too long to read at a glance.
pub fn short_path(path: &str, home: Option<&str>) -> String {
    match home.filter(|home| !home.is_empty()) {
        Some(home) => match path.strip_prefix(home) {
            Some(rest) if rest.is_empty() => "~".into(),
            Some(rest) if rest.starts_with(['/', '\\']) => format!("~{rest}"),
            _ => path.into(),
        },
        None => path.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, starred: bool, updated_at: i64) -> ProjectSummary {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "starred": starred, "updated_at": updated_at
        }))
        .unwrap()
    }

    #[test]
    fn starred_projects_lead_then_the_most_recent() {
        let order: Vec<_> = ordered(vec![
            project("old", false, 10),
            project("new", false, 30),
            project("star-old", true, 5),
            project("star-new", true, 20),
            project("b", false, 10),
        ])
        .into_iter()
        .map(|project| project.id)
        .collect();
        assert_eq!(order, ["star-new", "star-old", "new", "b", "old"]);
    }

    #[test]
    fn counts_and_activity_read_naturally() {
        let mut one = project("p", false, 0);
        one.session_count = 1;
        one.artifact_count = 0;
        assert_eq!(counts(&one), "1 conversation · 0 artifacts");
        assert!(activity(&one).is_empty());
        one.session_count = 12;
        one.artifact_count = 1;
        one.running_count = 2;
        one.needs_you_count = 1;
        assert_eq!(counts(&one), "12 conversations · 1 artifact");
        assert_eq!(
            activity(&one),
            [
                ("2 running".to_string(), Tone::Active),
                ("1 waiting".to_string(), Tone::Attention)
            ]
        );
    }

    #[test]
    fn age_is_short_and_never_negative() {
        let now = 1_800_000_000;
        assert_eq!(age(now, now + 30), "just now");
        assert_eq!(age(now, now - 59), "just now");
        assert_eq!(age(now, now - 60), "1 min ago");
        assert_eq!(age(now, now - 7_200), "2 h ago");
        assert_eq!(age(now, now - 86_400), "1 day ago");
        assert_eq!(age(now, now - 86_400 * 13), "13 days ago");
        assert_eq!(age(now, now - 86_400 * 21), "3 weeks ago");
        assert_eq!(age(now, now - 86_400 * 90), "3 months ago");
        assert_eq!(age(now, now - 86_400 * 800), "2 years ago");
        // Millisecond timestamps and unset ones do not produce nonsense.
        assert_eq!(age(now, (now - 120) * 1000), "2 min ago");
        assert_eq!(age(now, 0), "");
    }

    #[test]
    fn paths_under_home_are_shortened_only_at_a_boundary() {
        assert_eq!(short_path("/Users/ada/lab", Some("/Users/ada")), "~/lab");
        assert_eq!(short_path("/Users/ada", Some("/Users/ada")), "~");
        assert_eq!(
            short_path("/Users/adam/lab", Some("/Users/ada")),
            "/Users/adam/lab"
        );
        assert_eq!(
            short_path(r"C:\Users\ada\lab", Some(r"C:\Users\ada")),
            r"~\lab"
        );
        assert_eq!(short_path("/data/lab", None), "/data/lab");
        assert_eq!(short_path("/data/lab", Some("")), "/data/lab");
    }
}
