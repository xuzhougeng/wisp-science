//! The conversations a window has open as tabs: the ones visited in it, in
//! the order they were first opened, across projects. It holds no graphics
//! state and nothing is saved: a new window starts with none.

/// The most tabs a window keeps. One more pushes out the one that has gone
/// unvisited the longest.
pub const LIMIT: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub project: String,
    pub session: String,
    /// What the conversation was called when it was last seen; the sidebar
    /// may not list it any more once another project is open.
    pub title: String,
    /// When it was last visited, as a count that only grows.
    seen: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tabs {
    open: Vec<Tab>,
    clock: u64,
}

impl Tabs {
    pub fn open(&self) -> &[Tab] {
        &self.open
    }

    /// Notes that a conversation is being looked at: it gets a tab if it
    /// has none, at the end, and an older one makes room when there are
    /// too many.
    pub fn visit(&mut self, project: &str, session: &str) {
        self.clock += 1;
        match self.open.iter_mut().find(|tab| tab.session == session) {
            Some(tab) => {
                tab.seen = self.clock;
                tab.project = project.to_owned();
            }
            None => self.open.push(Tab {
                project: project.to_owned(),
                session: session.to_owned(),
                title: String::new(),
                seen: self.clock,
            }),
        }
        while self.open.len() > LIMIT {
            let oldest = self
                .open
                .iter()
                .enumerate()
                .min_by_key(|(_, tab)| tab.seen)
                .map(|(index, _)| index)
                .expect("more tabs than the limit");
            self.open.remove(oldest);
        }
    }

    /// Keeps a tab's title up to date with what the conversation is called.
    pub fn name(&mut self, session: &str, title: &str) {
        if let Some(tab) = self.open.iter_mut().find(|tab| tab.session == session) {
            if !title.trim().is_empty() {
                tab.title = title.trim().to_owned();
            }
        }
    }

    /// Closes a tab. When it was the one being looked at (`current`), says
    /// which to go to instead: the one to its right, else to its left.
    pub fn close(&mut self, session: &str, current: Option<&str>) -> Option<Tab> {
        let index = self.open.iter().position(|tab| tab.session == session)?;
        self.open.remove(index);
        if current != Some(session) {
            return None;
        }
        self.open
            .get(index)
            .or_else(|| index.checked_sub(1).and_then(|left| self.open.get(left)))
            .cloned()
    }

    /// Drops the tabs of a project that left Wisp.
    pub fn forget_project(&mut self, project: &str) {
        self.open.retain(|tab| tab.project != project);
    }
}

/// What a tab is labelled: its title, or a stand-in before it has one.
pub fn label(tab: &Tab) -> String {
    match tab.title.as_str() {
        "" => crate::i18n::tr("New conversation").to_owned(),
        title => title.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sessions(tabs: &Tabs) -> Vec<&str> {
        tabs.open().iter().map(|tab| tab.session.as_str()).collect()
    }

    #[test]
    fn a_visited_conversation_gets_one_tab_in_the_order_it_was_opened() {
        let mut tabs = Tabs::default();
        assert!(tabs.open().is_empty());
        // One conversation has its tab too: the window's bar always shows
        // the conversation in view.
        tabs.visit("p", "a");
        assert_eq!(sessions(&tabs), ["a"]);
        tabs.visit("p", "b");
        tabs.visit("q", "c");
        tabs.visit("p", "a");
        assert_eq!(sessions(&tabs), ["a", "b", "c"]);
        // A tab is named once its conversation is, and keeps the name.
        assert_eq!(label(&tabs.open()[1]), "New conversation");
        tabs.name("b", "  Count the reads ");
        tabs.name("b", "  ");
        tabs.name("zzz", "nobody");
        assert_eq!(label(&tabs.open()[1]), "Count the reads");
    }

    #[test]
    fn closing_the_tab_in_view_goes_to_its_neighbour() {
        let mut tabs = Tabs::default();
        for session in ["a", "b", "c"] {
            tabs.visit("p", session);
        }
        // Another tab closes without moving anywhere.
        assert_eq!(tabs.close("a", Some("b")), None);
        assert_eq!(sessions(&tabs), ["b", "c"]);
        // The one in view hands over to its right, then to its left.
        assert_eq!(tabs.close("b", Some("b")).unwrap().session, "c");
        tabs.visit("p", "d");
        assert_eq!(tabs.close("d", Some("d")).unwrap().session, "c");
        // The last one leaves nothing to go to.
        assert_eq!(tabs.close("c", Some("c")), None);
        assert!(tabs.open().is_empty());
        assert_eq!(tabs.close("gone", None), None);
    }

    #[test]
    fn the_longest_unvisited_tab_makes_room_and_a_project_takes_its_tabs_along() {
        let mut tabs = Tabs::default();
        for index in 0..LIMIT {
            tabs.visit("p", &format!("s{index}"));
        }
        // Looked at again, the first is no longer the oldest.
        tabs.visit("p", "s0");
        tabs.visit("q", "new");
        assert_eq!(tabs.open().len(), LIMIT);
        assert!(sessions(&tabs).contains(&"s0"));
        assert!(!sessions(&tabs).contains(&"s1"));
        assert_eq!(*sessions(&tabs).last().unwrap(), "new");

        tabs.forget_project("p");
        assert_eq!(sessions(&tabs), ["new"]);
    }
}
