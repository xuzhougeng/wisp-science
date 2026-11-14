//! How the sidebar arranges a project's conversations: the sort and grouping
//! the user chose, the rows that follow from them, and what select mode has
//! picked. It holds no graphics state and is exercised without a window.
use std::collections::{HashMap, HashSet};

use wisp_dto::SessionSearchInfo;
use wisp_server::desktop::Folder;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Grouping {
    /// Under the groups the user made.
    #[default]
    Groups,
    /// By how long ago each conversation was last active.
    Date,
    None,
}

impl Grouping {
    pub const ALL: [Grouping; 3] = [Grouping::Groups, Grouping::Date, Grouping::None];

    pub fn label(self) -> &'static str {
        match self {
            Grouping::Groups => "Groups",
            Grouping::Date => "Date",
            Grouping::None => "None",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Grouping::Groups => "groups",
            Grouping::Date => "date",
            Grouping::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sorting {
    #[default]
    Newest,
    Name,
}

impl Sorting {
    pub const ALL: [Sorting; 2] = [Sorting::Newest, Sorting::Name];

    pub fn label(self) -> &'static str {
        match self {
            Sorting::Newest => "Newest",
            Sorting::Name => "Name",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Sorting::Newest => "newest",
            Sorting::Name => "name",
        }
    }
}

/// The sidebar's sort and grouping, as it is saved between launches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Arrangement {
    pub grouping: Grouping,
    pub sorting: Sorting,
}

impl Arrangement {
    pub fn encode(self) -> String {
        format!("{},{}", self.grouping.key(), self.sorting.key())
    }

    /// Reads a saved arrangement. A part this build does not know falls
    /// back to its default instead of discarding the other part.
    pub fn decode(text: &str) -> Self {
        let mut parts = text.split(',').map(str::trim);
        let grouping = parts.next().unwrap_or_default();
        let sorting = parts.next().unwrap_or_default();
        Self {
            grouping: Grouping::ALL
                .into_iter()
                .find(|choice| choice.key() == grouping)
                .unwrap_or_default(),
            sorting: Sorting::ALL
                .into_iter()
                .find(|choice| choice.key() == sorting)
                .unwrap_or_default(),
        }
    }
}

/// Which list the sidebar is showing. Sections are for everyday browsing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listing {
    Browse,
    /// Search results keep the host's order.
    Search,
    Archive,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    Section {
        /// Stable name for remembering that the section is folded.
        key: String,
        label: String,
        count: usize,
        /// Set when the section is a group the user made.
        folder: Option<String>,
        collapsed: bool,
    },
    Conversation {
        /// Index into the conversations the layout was made from.
        index: usize,
        /// How many branches deep it sits under the conversation it came
        /// from; 0 for one that is nobody's branch here.
        depth: usize,
    },
}

/// Everything a layout is made from.
pub struct Listed<'a> {
    pub rows: &'a [SessionSearchInfo],
    pub pinned: &'a [String],
    pub folders: &'a [Folder],
    /// Conversation id to the id of its group.
    pub grouped: &'a HashMap<String, String>,
    /// Branch id to the id of the conversation it was branched from.
    pub branched: &'a HashMap<String, String>,
    /// Keys of folded sections.
    pub collapsed: &'a HashSet<String>,
    /// Unix seconds.
    pub now: i64,
}

const DAY: i64 = 86_400;
const AGES: [(i64, &str); 4] = [
    (DAY, "Last 24 hours"),
    (7 * DAY, "Last 7 days"),
    (30 * DAY, "Last 30 days"),
    (i64::MAX, "Older"),
];

fn seconds(ts: i64) -> i64 {
    // Older rows carry milliseconds.
    if ts > 100_000_000_000 { ts / 1000 } else { ts }
}

/// Each conversation of `order` that is nobody's branch here, followed by
/// its branches and theirs, as `(index, depth)`. A branch whose source is
/// not listed stands on its own.
fn families(listed: &Listed, order: &[usize]) -> Vec<Vec<(usize, usize)>> {
    let place: HashMap<&str, usize> = listed
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.id.as_str(), index))
        .collect();
    let mut branches: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut sources = Vec::new();
    for &index in order {
        let source = listed
            .branched
            .get(&listed.rows[index].id)
            .and_then(|id| place.get(id.as_str()))
            .filter(|source| **source != index);
        match source {
            Some(source) => branches.entry(*source).or_default().push(index),
            None => sources.push(index),
        }
    }
    let mut placed = HashSet::new();
    let mut families = Vec::new();
    for source in sources {
        let mut family = Vec::new();
        let mut next = vec![(source, 0)];
        while let Some((index, depth)) = next.pop() {
            if !placed.insert(index) {
                continue;
            }
            family.push((index, depth));
            for branch in branches.get(&index).into_iter().flatten().rev() {
                next.push((*branch, depth + 1));
            }
        }
        families.push(family);
    }
    // Links that form a ring have no source to hang from; list their
    // members plainly instead of losing them.
    for &index in order {
        if placed.insert(index) {
            families.push(vec![(index, 0)]);
        }
    }
    families
}

/// The rows of the conversation list, top to bottom. Pinned conversations
/// lead their section; the chosen order applies after that; a branch sits
/// under the conversation it came from.
pub fn layout(listed: &Listed, arrangement: Arrangement, listing: Listing) -> Vec<Row> {
    let mut order: Vec<usize> = (0..listed.rows.len()).collect();
    let row = |(index, depth): (usize, usize)| Row::Conversation { index, depth };
    if listing == Listing::Search {
        return order.into_iter().map(|index| row((index, 0))).collect();
    }
    let pinned = |index: &usize| listed.pinned.contains(&listed.rows[*index].id);
    order.sort_by(|a, b| {
        let (left, right) = (&listed.rows[*a], &listed.rows[*b]);
        let newest = seconds(right.activity_at).cmp(&seconds(left.activity_at));
        pinned(b)
            .cmp(&pinned(a))
            .then_with(|| match arrangement.sorting {
                Sorting::Newest => newest,
                Sorting::Name => left
                    .title
                    .to_lowercase()
                    .cmp(&right.title.to_lowercase())
                    .then(newest),
            })
    });

    let families = families(listed, &order);

    // Sections in display order: key, label, the group it stands for. A
    // family goes where the conversation at its head belongs.
    let mut sections: Vec<(String, String, Option<String>, Vec<(usize, usize)>)> = Vec::new();
    match arrangement.grouping {
        Grouping::Groups if listing == Listing::Browse && !listed.folders.is_empty() => {
            for folder in listed.folders {
                sections.push((
                    format!("folder:{}", folder.id),
                    folder.name.clone(),
                    Some(folder.id.clone()),
                    Vec::new(),
                ));
            }
            sections.push(("ungrouped".into(), "No group".into(), None, Vec::new()));
            let last = sections.len() - 1;
            for family in families {
                let folder = listed.grouped.get(&listed.rows[family[0].0].id);
                let section = sections
                    .iter()
                    .position(|(_, _, id, _)| id.is_some() && id.as_ref() == folder)
                    .unwrap_or(last);
                sections[section].3.extend(family);
            }
            // A group the user made is shown even when it is empty.
            if sections[last].3.is_empty() {
                sections.pop();
            }
        }
        Grouping::Date if listing == Listing::Browse => {
            for (index, (_, label)) in AGES.iter().enumerate() {
                sections.push((format!("age:{index}"), (*label).into(), None, Vec::new()));
            }
            for family in families {
                let age = listed.now - seconds(listed.rows[family[0].0].activity_at);
                let section = AGES.iter().position(|(limit, _)| age < *limit).unwrap_or(3);
                sections[section].3.extend(family);
            }
            sections.retain(|(.., members)| !members.is_empty());
        }
        _ => return families.into_iter().flatten().map(row).collect(),
    }

    let mut rows = Vec::new();
    for (key, label, folder, members) in sections {
        let collapsed = listed.collapsed.contains(&key);
        rows.push(Row::Section {
            key,
            label,
            count: members.len(),
            folder,
            collapsed,
        });
        if !collapsed {
            rows.extend(members.into_iter().map(row));
        }
    }
    rows
}

/// Ids of the conversations on screen, top to bottom.
pub fn visible(rows: &[Row], sessions: &[SessionSearchInfo]) -> Vec<String> {
    rows.iter()
        .filter_map(|row| match row {
            Row::Conversation { index, .. } => sessions.get(*index).map(|row| row.id.clone()),
            Row::Section { .. } => None,
        })
        .collect()
}

/// The group order after moving one group a place up or down, or `None`
/// when it is already at that end.
pub fn moved(folders: &[Folder], id: &str, up: bool) -> Option<Vec<String>> {
    let mut order: Vec<String> = folders.iter().map(|folder| folder.id.clone()).collect();
    let from = order.iter().position(|other| other == id)?;
    let to = if up {
        from.checked_sub(1)?
    } else {
        (from + 1 < order.len()).then_some(from + 1)?
    };
    order.swap(from, to);
    Some(order)
}

/// The conversations picked in select mode.
#[derive(Default)]
pub struct Picks {
    ids: HashSet<String>,
    /// Where a shift-click's range starts.
    anchor: Option<String>,
}

impl Picks {
    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchor = None;
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// A plain click: flips one conversation and anchors the next range.
    pub fn toggle(&mut self, id: &str) {
        if !self.ids.remove(id) {
            self.ids.insert(id.to_owned());
        }
        self.anchor = Some(id.to_owned());
    }

    /// A shift-click: picks everything from the anchor to `id` as listed.
    /// Without an anchor on screen it is a plain click.
    pub fn extend(&mut self, id: &str, visible: &[String]) {
        let place = |wanted: &str| visible.iter().position(|other| other == wanted);
        let ends = self.anchor.as_deref().and_then(place).zip(place(id));
        let Some((from, to)) = ends else {
            return self.toggle(id);
        };
        let range = from.min(to)..=from.max(to);
        self.ids.extend(visible[range].iter().cloned());
    }

    pub fn covers(&self, visible: &[String]) -> bool {
        !visible.is_empty() && visible.iter().all(|id| self.ids.contains(id))
    }

    /// Picks every conversation on screen, or none when they all are.
    pub fn toggle_all(&mut self, visible: &[String]) {
        if self.covers(visible) {
            self.clear();
        } else {
            self.ids.extend(visible.iter().cloned());
        }
    }

    /// Forgets picks that are no longer in the list at all.
    pub fn keep(&mut self, listed: &[SessionSearchInfo]) {
        self.ids.retain(|id| listed.iter().any(|row| &row.id == id));
        if let Some(anchor) = &self.anchor {
            if !listed.iter().any(|row| &row.id == anchor) {
                self.anchor = None;
            }
        }
    }

    /// The picks in the order the list has them.
    pub fn ordered(&self, listed: &[SessionSearchInfo]) -> Vec<String> {
        listed
            .iter()
            .filter(|row| self.ids.contains(&row.id))
            .map(|row| row.id.clone())
            .collect()
    }
}

/// A file name for a conversation saved as a document: its title with
/// what a file name cannot hold replaced, not too long, ending in `.md`.
pub fn export_name(title: &str) -> String {
    let cleaned: String = title
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    let name: String = words.join(" ").chars().take(80).collect();
    let name = name.trim().trim_matches('.').trim();
    if name.is_empty() {
        "conversation.md".into()
    } else {
        format!("{name}.md")
    }
}

/// One line saying how a batch went: `done` conversations were `verb`
/// ("Archived", "Deleted"), and the first refusal says why the rest were not.
pub fn report(verb: &str, done: usize, refused: &[(String, String)]) -> String {
    let count = |n: usize| {
        if n == 1 {
            "1 conversation".to_string()
        } else {
            format!("{n} conversations")
        }
    };
    let Some((_, reason)) = refused.first() else {
        return format!("{verb} {}.", count(done));
    };
    let left = if refused.len() == 1 {
        "1 was left as it is".to_string()
    } else {
        format!("{} were left as they are", refused.len())
    };
    if done == 0 {
        format!("Nothing changed: {reason}.")
    } else {
        format!("{verb} {}. {left}: {reason}.", count(done))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn row(id: &str, title: &str, age: i64) -> SessionSearchInfo {
        SessionSearchInfo {
            id: id.into(),
            project_id: "p".into(),
            project_name: "P".into(),
            title: title.into(),
            ts: NOW - age,
            activity_at: NOW - age,
            status: "complete".into(),
            shelved: false,
        }
    }

    fn folder(id: &str, name: &str) -> Folder {
        Folder {
            id: id.into(),
            name: name.into(),
        }
    }

    /// Renders a layout as text: `[label n]` for a section, ids otherwise,
    /// each `>` in front of an id being one level of branching.
    fn shape(rows: &[Row], sessions: &[SessionSearchInfo]) -> String {
        rows.iter()
            .map(|row| match row {
                Row::Section {
                    label,
                    count,
                    collapsed,
                    ..
                } => format!(
                    "[{label} {count}{}]",
                    if *collapsed { " folded" } else { "" }
                ),
                Row::Conversation { index, depth } => {
                    format!("{}{}", ">".repeat(*depth), sessions[*index].id)
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn the_saved_arrangement_survives_a_round_trip_and_unknown_parts() {
        for grouping in Grouping::ALL {
            for sorting in Sorting::ALL {
                let arrangement = Arrangement { grouping, sorting };
                assert_eq!(Arrangement::decode(&arrangement.encode()), arrangement);
            }
        }
        assert_eq!(Arrangement::decode(""), Arrangement::default());
        assert_eq!(
            Arrangement::decode("kanban, name"),
            Arrangement {
                grouping: Grouping::Groups,
                sorting: Sorting::Name
            }
        );
        assert_eq!(Arrangement::decode("date").grouping, Grouping::Date);
    }

    #[test]
    fn groups_show_in_the_users_order_with_pins_leading_each_section() {
        let sessions = [
            row("a", "Zebrafish", 10),
            row("b", "apoptosis", 20),
            row("c", "Kinase screen", 30),
            row("d", "Blot", 40),
            row("e", "stray", 50),
        ];
        let folders = [folder("f2", "Screens"), folder("f1", "Writing")];
        let grouped: HashMap<String, String> = [
            ("a", "f1"),
            ("b", "f1"),
            ("c", "f2"),
            // A group that no longer exists.
            ("e", "gone"),
        ]
        .into_iter()
        .map(|(id, folder)| (id.to_string(), folder.to_string()))
        .collect();
        let pinned = ["b".to_string()];
        let mut collapsed = HashSet::new();
        let listed = |collapsed: &HashSet<String>, folders: &[Folder]| {
            let listed = Listed {
                rows: &sessions,
                pinned: &pinned,
                folders,
                grouped: &grouped,
                branched: &HashMap::new(),
                collapsed,
                now: NOW,
            };
            let by = |sorting| {
                let arrangement = Arrangement {
                    grouping: Grouping::Groups,
                    sorting,
                };
                shape(&layout(&listed, arrangement, Listing::Browse), &sessions)
            };
            (by(Sorting::Newest), by(Sorting::Name))
        };

        let (newest, name) = listed(&collapsed, &folders);
        assert_eq!(newest, "[Screens 1] c [Writing 2] b a [No group 2] d e");
        // By name, ignoring case; the pin still leads.
        assert_eq!(name, "[Screens 1] c [Writing 2] b a [No group 2] d e");

        collapsed.insert("folder:f1".to_string());
        let (newest, _) = listed(&collapsed, &folders);
        assert_eq!(newest, "[Screens 1] c [Writing 2 folded] [No group 2] d e");

        // Without groups there is nothing to head: a plain list.
        let (newest, name) = listed(&collapsed, &[]);
        assert_eq!(newest, "b a c d e");
        assert_eq!(name, "b d c e a");
    }

    #[test]
    fn an_empty_group_is_listed_and_no_group_is_hidden_when_unused() {
        let sessions = [row("a", "A", 10)];
        let folders = [folder("f1", "Writing"), folder("f2", "Empty")];
        let grouped = HashMap::from([("a".to_string(), "f1".to_string())]);
        let listed = Listed {
            rows: &sessions,
            pinned: &[],
            folders: &folders,
            grouped: &grouped,
            branched: &HashMap::new(),
            collapsed: &HashSet::new(),
            now: NOW,
        };
        let rows = layout(&listed, Arrangement::default(), Listing::Browse);
        assert_eq!(shape(&rows, &sessions), "[Writing 1] a [Empty 0]");
        assert_eq!(visible(&rows, &sessions), ["a"]);
    }

    #[test]
    fn date_sections_follow_last_activity_and_skip_empty_spans() {
        let sessions = [
            row("old", "old", 400 * DAY),
            row("week", "week", 3 * DAY),
            row("today", "today", 60),
            // Milliseconds from an older row.
            SessionSearchInfo {
                activity_at: (NOW - 2 * DAY) * 1000,
                ..row("millis", "millis", 0)
            },
        ];
        let listed = Listed {
            rows: &sessions,
            pinned: &[],
            folders: &[],
            grouped: &HashMap::new(),
            branched: &HashMap::new(),
            collapsed: &HashSet::new(),
            now: NOW,
        };
        let by_date = Arrangement {
            grouping: Grouping::Date,
            sorting: Sorting::Newest,
        };
        assert_eq!(
            shape(&layout(&listed, by_date, Listing::Browse), &sessions),
            "[Last 24 hours 1] today [Last 7 days 2] millis week [Older 1] old"
        );
        // The archive is sorted but not sectioned; search keeps host order.
        assert_eq!(
            shape(&layout(&listed, by_date, Listing::Archive), &sessions),
            "today millis week old"
        );
        assert_eq!(
            shape(&layout(&listed, by_date, Listing::Search), &sessions),
            "old week today millis"
        );
    }

    #[test]
    fn a_branch_sits_under_its_source_wherever_sorting_would_put_it() {
        let sessions = [
            row("main", "Main", 50),
            // Newer than its source, and a branch of a branch after it.
            row("b1", "Branch one", 10),
            row("b2", "Branch two", 5),
            row("other", "Other", 20),
            // Its source is archived, so it is not in this list.
            row("stray", "Stray", 30),
            row("deep", "Deep", 1),
        ];
        let branched: HashMap<String, String> = [
            ("b1", "main"),
            ("b2", "main"),
            ("deep", "b1"),
            ("stray", "archived"),
        ]
        .into_iter()
        .map(|(id, source)| (id.to_string(), source.to_string()))
        .collect();
        let folders = [folder("f", "Screens")];
        let grouped = HashMap::from([("main".to_string(), "f".to_string())]);
        let pinned = ["b1".to_string()];
        let collapsed = HashSet::new();
        let flat = |branched: &HashMap<String, String>| {
            let listed = Listed {
                rows: &sessions,
                pinned: &pinned,
                folders: &[],
                grouped: &grouped,
                branched,
                collapsed: &collapsed,
                now: NOW,
            };
            shape(
                &layout(&listed, Arrangement::default(), Listing::Browse),
                &sessions,
            )
        };
        // The pinned branch leads its siblings, not the list.
        assert_eq!(flat(&branched), "other stray main >b1 >>deep >b2");

        // With groups, a family is filed where its source is, whatever
        // group its branches say.
        let filed = Listed {
            rows: &sessions,
            pinned: &pinned,
            folders: &folders,
            grouped: &grouped,
            branched: &branched,
            collapsed: &collapsed,
            now: NOW,
        };
        let rows = layout(&filed, Arrangement::default(), Listing::Browse);
        assert_eq!(
            shape(&rows, &sessions),
            "[Screens 4] main >b1 >>deep >b2 [No group 2] other stray"
        );
        assert_eq!(
            visible(&rows, &sessions),
            ["main", "b1", "deep", "b2", "other", "stray"]
        );
        // Search results are a flat ranking.
        assert_eq!(
            shape(
                &layout(&filed, Arrangement::default(), Listing::Search),
                &sessions
            ),
            "main b1 b2 other stray deep"
        );

        // Two conversations that name each other still both show.
        let ring: HashMap<String, String> = [("main", "other"), ("other", "main")]
            .into_iter()
            .map(|(id, source)| (id.to_string(), source.to_string()))
            .collect();
        let shown = flat(&ring);
        for id in ["main", "other", "b1", "stray"] {
            assert!(
                shown.split(' ').any(|word| word.trim_matches('>') == id),
                "{shown}"
            );
        }
    }

    #[test]
    fn a_group_moves_one_place_and_stops_at_the_ends() {
        let folders = [folder("a", "A"), folder("b", "B"), folder("c", "C")];
        assert_eq!(moved(&folders, "b", true).unwrap(), ["b", "a", "c"]);
        assert_eq!(moved(&folders, "b", false).unwrap(), ["a", "c", "b"]);
        assert_eq!(moved(&folders, "a", true), None);
        assert_eq!(moved(&folders, "c", false), None);
        assert_eq!(moved(&folders, "missing", true), None);
    }

    #[test]
    fn picks_toggle_extend_over_a_range_and_follow_the_list() {
        let sessions: Vec<_> = ["a", "b", "c", "d", "e"]
            .into_iter()
            .map(|id| row(id, id, 0))
            .collect();
        let shown: Vec<String> = sessions.iter().map(|row| row.id.clone()).collect();
        let mut picks = Picks::default();

        // Shift-click with no anchor is a plain click.
        picks.extend("b", &shown);
        assert_eq!(picks.ordered(&sessions), ["b"]);
        picks.extend("d", &shown);
        assert_eq!(picks.ordered(&sessions), ["b", "c", "d"]);
        // Backwards from the same anchor.
        picks.extend("a", &shown);
        assert_eq!(picks.ordered(&sessions), ["a", "b", "c", "d"]);
        picks.toggle("c");
        assert_eq!(picks.ordered(&sessions), ["a", "b", "d"]);
        assert!(!picks.contains("c") && picks.len() == 3);

        assert!(!picks.covers(&shown));
        picks.toggle_all(&shown);
        assert!(picks.covers(&shown));
        picks.toggle_all(&shown);
        assert!(picks.is_empty());
        assert!(!picks.covers(&[]));

        // A conversation that left the list is no longer picked.
        picks.toggle("a");
        picks.toggle("e");
        picks.keep(&sessions[..3]);
        assert_eq!(picks.ordered(&sessions), ["a"]);
        // Its anchor went with it, so the next shift-click starts afresh.
        picks.extend("c", &shown);
        assert_eq!(picks.ordered(&sessions), ["a", "c"]);
    }

    #[test]
    fn a_batch_report_counts_what_changed_and_says_why_the_rest_did_not() {
        assert_eq!(report("Archived", 1, &[]), "Archived 1 conversation.");
        assert_eq!(report("Deleted", 3, &[]), "Deleted 3 conversations.");
        let busy = ("a".to_string(), "Stop the running turn first".to_string());
        assert_eq!(
            report("Deleted", 2, &[busy.clone()]),
            "Deleted 2 conversations. 1 was left as it is: Stop the running turn first."
        );
        assert_eq!(
            report("Deleted", 1, &[busy.clone(), busy.clone()]),
            "Deleted 1 conversation. 2 were left as they are: Stop the running turn first."
        );
        assert_eq!(
            report("Deleted", 0, &[busy]),
            "Nothing changed: Stop the running turn first."
        );
    }

    #[test]
    fn an_exported_conversation_gets_a_name_a_file_can_have() {
        assert_eq!(export_name("  Align the reads  "), "Align the reads.md");
        // What a path or a shell would trip over is replaced, not dropped.
        assert_eq!(export_name("a/b\\c: d? \"e\""), "a-b-c- d- -e-.md");
        assert_eq!(export_name("two\nlines\there"), "two lines here.md");
        // A long title is cut; a title with nothing usable still has a name.
        let long = export_name(&"word ".repeat(40));
        assert!(long.chars().count() <= 83 && long.ends_with(".md"));
        assert_eq!(export_name("   "), "conversation.md");
        assert_eq!(export_name("..."), "conversation.md");
        assert_eq!(export_name("请总结 CRISPR 筛选"), "请总结 CRISPR 筛选.md");
    }
}
