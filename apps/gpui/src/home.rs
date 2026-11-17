//! Ordering and wording for the home page, kept apart from GPUI so it is
//! covered without a graphics context.
use wisp_dto::ProjectSummary;
use wisp_server::desktop::{FolderContents, ProjectFolder};

use crate::{
    i18n::{Locale, tr},
    present::Tone,
};

/// Where the question about deleting a project's folder stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FolderQuestion {
    /// The folder is being looked at.
    Looking,
    /// It may not be deleted with the project, and why.
    Refused(String),
    /// What would go, to be confirmed by typing the project's name.
    Ask(ProjectFolder),
}

/// What deleting the folder would do, said before it is done.
pub fn folder_warning(folder: &ProjectFolder) -> String {
    match folder.contents {
        FolderContents::Missing => crate::i18n::fill(
            tr(
                "The folder of \"{}\" is already gone. Only its conversations and records are removed from Wisp.",
            ),
            &[&folder.name],
        ),
        FolderContents::Link => crate::i18n::fill(
            tr("{} is a link. The link is deleted; the folder it points at is kept."),
            &[&folder.path],
        ),
        FolderContents::Files { files, bytes, more } => crate::i18n::fill(
            tr(
                "This deletes {} for good, with {}{} ({}), and removes the project from Wisp. It cannot be undone and nothing goes to the Trash.",
            ),
            &[
                &folder.path,
                &if more { tr("at least ") } else { "" },
                &crate::files::count(files as usize),
                &crate::files::size_label(bytes),
            ],
        ),
    }
}

/// Whether what was typed is the project's name, so the folder may go.
pub fn name_typed(typed: &str, folder: &ProjectFolder) -> bool {
    let name = folder.name.trim();
    !name.is_empty() && typed.trim() == name
}

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
    counts_in(crate::i18n::current(), project)
}

fn counts_in(locale: Locale, project: &ProjectSummary) -> String {
    match locale {
        Locale::En => format!(
            "{} · {}",
            plural(project.session_count, "conversation", "conversations"),
            plural(project.artifact_count, "artifact", "artifacts")
        ),
        // Chinese counts with a measure word and has no plural.
        Locale::Zh => format!(
            "{} 个会话 · {} 个产物",
            project.session_count, project.artifact_count
        ),
    }
}

/// What is happening in a project right now; empty when it is idle.
pub fn activity(project: &ProjectSummary) -> Vec<(String, Tone)> {
    activity_in(crate::i18n::current(), project)
}

fn activity_in(locale: Locale, project: &ProjectSummary) -> Vec<(String, Tone)> {
    let (running, waiting) = match locale {
        Locale::En => ("running", "waiting"),
        Locale::Zh => ("个运行中", "个等你处理"),
    };
    let mut chips = Vec::new();
    if project.running_count > 0 {
        chips.push((format!("{} {running}", project.running_count), Tone::Active));
    }
    if project.needs_you_count > 0 {
        chips.push((
            format!("{} {waiting}", project.needs_you_count),
            Tone::Attention,
        ));
    }
    chips
}

/// Where a cloud-drive folder project's latest changes stand, for its
/// card; none for a project that is not saved to such a folder.
pub fn folder_badge(project: &ProjectSummary) -> Option<(&'static str, Tone)> {
    Some(match project.folder_sync.as_deref()? {
        "saved" => (tr("Saved to project folder"), Tone::Success),
        "unpublished" => (tr("Changes not yet saved to folder"), Tone::Attention),
        "remote-newer" => (tr("Newer version in folder"), Tone::Active),
        "conflict" => (tr("Folder version conflict"), Tone::Danger),
        _ => (tr("Waiting for the cloud drive"), Tone::Neutral),
    })
}

/// How long ago `then` was, as a short label. Both are Unix seconds; a
/// millisecond value from an older row is accepted too.
pub fn age(now: i64, then: i64) -> String {
    age_in(crate::i18n::current(), now, then)
}

fn age_in(locale: Locale, now: i64, then: i64) -> String {
    let then = if then > 100_000_000_000 {
        then / 1000
    } else {
        then
    };
    if then <= 0 {
        return String::new();
    }
    let seconds = (now - then).max(0);
    if locale == Locale::Zh {
        return match seconds {
            0..=59 => "刚刚".into(),
            60..=3_599 => format!("{} 分钟前", seconds / 60),
            3_600..=86_399 => format!("{} 小时前", seconds / 3_600),
            86_400..=1_209_599 => format!("{} 天前", seconds / 86_400),
            1_209_600..=5_183_999 => format!("{} 周前", seconds / 604_800),
            5_184_000..=31_535_999 => format!("{} 个月前", seconds / 2_592_000),
            _ => format!("{} 年前", seconds / 31_536_000),
        };
    }
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

/// Whether the last cell of a grid of `count` cells, `columns` to a line,
/// is alone on its line and takes the whole of it, so no empty bordered
/// cell stands beside it.
pub fn last_cell_spans(count: usize, columns: usize) -> bool {
    columns > 1 && count % columns != 0
}

/// When `then` happened, as the time column of a record row: the clock
/// time today (`17:03`), "Yesterday", the month and day this year
/// (`11-03`), the full date before. Both are Unix seconds, read in the
/// local time zone; a millisecond value from an older row is accepted too.
pub fn clock(now: i64, then: i64) -> String {
    use chrono::{Local, TimeZone};
    let then = if then > 100_000_000_000 {
        then / 1000
    } else {
        then
    };
    if then <= 0 {
        return String::new();
    }
    match (
        Local.timestamp_opt(now, 0).single(),
        Local.timestamp_opt(then, 0).single(),
    ) {
        (Some(now), Some(then)) => clock_at(now.naive_local(), then.naive_local()),
        _ => String::new(),
    }
}

fn clock_at(now: chrono::NaiveDateTime, then: chrono::NaiveDateTime) -> String {
    use chrono::Datelike;
    let (today, day) = (now.date(), then.date());
    if day >= today {
        then.format("%H:%M").to_string()
    } else if today.pred_opt() == Some(day) {
        tr("Yesterday").to_owned()
    } else if day.year() == today.year() {
        then.format("%m-%d").to_string()
    } else {
        then.format("%Y-%m-%d").to_string()
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

/// A cleanup window as Project settings shows it: the days, or nothing
/// when runs are kept until cleaned up by hand.
pub fn days_text(days: Option<i64>) -> String {
    days.map(|days| days.to_string()).unwrap_or_default()
}

/// A cleanup window as it was typed: empty keeps runs, a whole number of
/// days from 1 to the limit sets one, anything else is said to be wrong.
pub fn days_typed(text: &str) -> Result<Option<i64>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<i64>() {
        Ok(days) if (1..=wisp_server::desktop::RETENTION_DAYS_LIMIT).contains(&days) => {
            Ok(Some(days))
        }
        _ => Err(crate::i18n::fill(
            tr("Keep runs and files from 1 to {} days, or leave the field empty"),
            &[&wisp_server::desktop::RETENTION_DAYS_LIMIT],
        )),
    }
}

/// Whether privacy mode hides `project` now: only while it is on, and only
/// the projects chosen for it.
pub fn concealed(mode: &wisp_dto::project_browser::PrivacyMode, project: &str) -> bool {
    mode.active && mode.project_ids.iter().any(|id| id == project)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_windows_are_whole_days_or_nothing() {
        assert_eq!(days_typed(""), Ok(None));
        assert_eq!(days_typed("  "), Ok(None));
        assert_eq!(days_typed(" 14 "), Ok(Some(14)));
        assert_eq!(days_typed("3650"), Ok(Some(3650)));
        for wrong in ["0", "-3", "3651", "1.5", "two weeks"] {
            assert!(days_typed(wrong).is_err(), "{wrong}");
        }
        assert_eq!(days_text(Some(7)), "7");
        assert_eq!(days_text(None), "");
    }

    #[test]
    fn a_cloud_folder_project_says_where_its_changes_stand() {
        let mut project = ProjectSummary {
            starred: false,
            id: "p".into(),
            name: "RNA lab".into(),
            description: String::new(),
            workspace_dir: "/work/rna".into(),
            session_count: 0,
            artifact_count: 0,
            updated_at: 0,
            running_count: 0,
            needs_you_count: 0,
            sync_configured: false,
            last_synced_at: None,
            folder_sync: None,
        };
        assert_eq!(folder_badge(&project), None);
        for (state, label, tone) in [
            ("saved", "Saved to project folder", Tone::Success),
            (
                "unpublished",
                "Changes not yet saved to folder",
                Tone::Attention,
            ),
            ("remote-newer", "Newer version in folder", Tone::Active),
            ("conflict", "Folder version conflict", Tone::Danger),
            ("waiting", "Waiting for the cloud drive", Tone::Neutral),
        ] {
            project.folder_sync = Some(state.into());
            assert_eq!(folder_badge(&project), Some((label, tone)), "{state}");
        }
    }

    #[test]
    fn deleting_a_folder_says_what_goes_and_needs_the_name() {
        let folder = |contents| ProjectFolder {
            name: "RNA lab".into(),
            path: "/work/rna".into(),
            contents,
        };
        let full = folder(FolderContents::Files {
            files: 1,
            bytes: 2_048,
            more: false,
        });
        assert_eq!(
            folder_warning(&full),
            "This deletes /work/rna for good, with 1 file (2.0 KB), and removes the project \
             from Wisp. It cannot be undone and nothing goes to the Trash."
        );
        let large = folder(FolderContents::Files {
            files: 200_000,
            bytes: 5 * 1024 * 1024,
            more: true,
        });
        assert!(folder_warning(&large).contains("with at least 200000 files (5.0 MB)"));
        assert!(folder_warning(&folder(FolderContents::Link)).contains("it points at is kept"));
        assert!(folder_warning(&folder(FolderContents::Missing)).contains("already gone"));

        // The name, as it is written; spaces around it do not matter.
        assert!(name_typed(" RNA lab ", &full));
        assert!(!name_typed("rna lab", &full));
        assert!(!name_typed("RNA", &full));
        assert!(!name_typed("", &full));
        // A project without a name cannot be confirmed by typing nothing.
        let unnamed = ProjectFolder {
            name: "  ".into(),
            ..full
        };
        assert!(!name_typed("", &unnamed));
        assert!(!name_typed("  ", &unnamed));
    }

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
    fn counts_activity_and_age_read_naturally_in_chinese() {
        let mut one = project("p", false, 0);
        one.session_count = 12;
        one.artifact_count = 1;
        one.running_count = 2;
        one.needs_you_count = 1;
        // A measure word, and no plural to get wrong.
        assert_eq!(counts_in(Locale::Zh, &one), "12 个会话 · 1 个产物");
        assert_eq!(
            activity_in(Locale::Zh, &one),
            [
                ("2 个运行中".to_string(), Tone::Active),
                ("1 个等你处理".to_string(), Tone::Attention)
            ]
        );
        let now = 1_800_000_000;
        let ago = |seconds: i64| age_in(Locale::Zh, now, now - seconds);
        assert_eq!(ago(5), "刚刚");
        assert_eq!(ago(120), "2 分钟前");
        assert_eq!(ago(7_200), "2 小时前");
        assert_eq!(ago(86_400), "1 天前");
        assert_eq!(ago(3 * 604_800), "3 周前");
        assert_eq!(ago(90 * 86_400), "3 个月前");
        assert_eq!(ago(2 * 31_536_000), "2 年前");
        // No time saved: nothing to say, in either language.
        assert_eq!(age_in(Locale::Zh, now, 0), "");
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
    fn a_records_time_is_the_clock_today_then_a_day_then_a_date() {
        let at = |date: (i32, u32, u32), hour: u32, minute: u32| {
            chrono::NaiveDate::from_ymd_opt(date.0, date.1, date.2)
                .unwrap()
                .and_hms_opt(hour, minute, 0)
                .unwrap()
        };
        let now = at((2026, 11, 16), 18, 30);
        assert_eq!(clock_at(now, at((2026, 11, 16), 17, 3)), "17:03");
        assert_eq!(clock_at(now, at((2026, 11, 16), 0, 0)), "00:00");
        assert_eq!(clock_at(now, at((2026, 11, 15), 23, 59)), "Yesterday");
        assert_eq!(clock_at(now, at((2026, 11, 3), 9, 0)), "11-03");
        assert_eq!(clock_at(now, at((2025, 12, 31), 9, 0)), "2025-12-31");
        // A clock a little behind the record's own still says the time.
        assert_eq!(clock_at(now, at((2026, 11, 17), 0, 5)), "00:05");
        // Yesterday across a new year.
        let new_year = at((2027, 1, 1), 8, 0);
        assert_eq!(clock_at(new_year, at((2026, 12, 31), 22, 0)), "Yesterday");
        // No time saved: nothing to say.
        assert_eq!(clock(1_800_000_000, 0), "");
        // Milliseconds are read as seconds.
        assert_eq!(
            clock(1_800_000_000, 1_800_000_000_000),
            clock(1_800_000_000, 1_800_000_000)
        );
    }

    #[test]
    fn a_lone_last_project_takes_the_whole_line() {
        assert!(last_cell_spans(1, 2));
        assert!(last_cell_spans(3, 2));
        assert!(!last_cell_spans(2, 2));
        assert!(!last_cell_spans(4, 2));
        assert!(!last_cell_spans(0, 2));
        assert!(!last_cell_spans(3, 1));
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

    #[test]
    fn privacy_mode_hides_only_the_chosen_projects_while_it_is_on() {
        use wisp_dto::project_browser::PrivacyMode;
        let mut mode = PrivacyMode {
            active: true,
            project_ids: vec!["lab".into()],
        };
        assert!(concealed(&mode, "lab"));
        assert!(!concealed(&mode, "field"));
        mode.active = false;
        assert!(!concealed(&mode, "lab"));
    }
}
