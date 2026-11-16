//! The windows opened for a project, kept so they open again at the next
//! start, as the WebView keeps its project windows: one is added when it
//! opens and taken off when it is closed, and quitting leaves the list as
//! it is. Each reopens in the project it was opened for.
use std::sync::Mutex;

/// The preference the list is saved under.
pub const PREFERENCE: &str = "project_windows";
/// Windows reopened at most, so a list that grew by mistake cannot flood
/// the screen.
const LIMIT: usize = 12;

/// The projects that have a window of their own now, in the order they
/// were opened.
static OPEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The saved list as it reads back: projects in order, each once, and no
/// more than [`LIMIT`]. Anything unreadable is no list.
pub fn decode(saved: &str) -> Vec<String> {
    let projects: Vec<String> = serde_json::from_str(saved).unwrap_or_default();
    let mut kept: Vec<String> = Vec::new();
    for project in projects {
        if !project.trim().is_empty() && !kept.contains(&project) {
            kept.push(project);
        }
    }
    kept.truncate(LIMIT);
    kept
}

pub fn encode(projects: &[String]) -> String {
    serde_json::to_string(projects).unwrap_or_else(|_| "[]".into())
}

/// The list with `project`'s window added or taken off. A project opened
/// in two windows is listed once; closing either takes it off.
pub fn with(mut projects: Vec<String>, project: &str, open: bool) -> Vec<String> {
    projects.retain(|listed| listed != project);
    if open {
        projects.push(project.to_owned());
    }
    projects
}

/// Notes that `project`'s window opened or closed, and returns the list to
/// save.
pub fn note(project: &str, open: bool) -> String {
    let mut list = OPEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *list = with(std::mem::take(&mut *list), project, open);
    encode(&list)
}

/// Notes that the windows of `projects` are open, in place of whatever was
/// noted, and returns the list to save.
pub fn replace(projects: &[String]) -> String {
    let mut list = OPEN.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *list = projects
        .iter()
        .fold(Vec::new(), |list, project| with(list, project, true));
    encode(&list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_windows_are_listed_once_in_the_order_they_opened() {
        let list = with(Vec::new(), "a", true);
        let list = with(list, "b", true);
        // Opened again: once, as the latest.
        let list = with(list, "a", true);
        assert_eq!(list, ["b", "a"]);
        let list = with(list, "b", false);
        assert_eq!(list, ["a"]);
        // Closing one that is not listed changes nothing.
        assert_eq!(with(list.clone(), "z", false), list);
        assert_eq!(decode(&encode(&list)), list);
    }

    #[test]
    fn a_saved_list_is_read_carefully() {
        assert!(decode("").is_empty());
        assert!(decode("not json").is_empty());
        assert!(decode("{\"a\":1}").is_empty());
        assert_eq!(decode(r#"["a", "", "a", "b"]"#), ["a", "b"]);
        let many = encode(&(0..40).map(|n| n.to_string()).collect::<Vec<_>>());
        assert_eq!(decode(&many).len(), LIMIT);
    }
}
