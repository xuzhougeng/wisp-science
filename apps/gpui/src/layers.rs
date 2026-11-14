//! Window-level Escape stack, ordered from the bottom surface to the topmost.
//! The stack is the single source of truth for which dismissible layers are
//! open, so one Escape press can only ever close the topmost one. It holds no
//! graphics state and is exercised without a window.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// Project file browser docked beside the transcript.
    FilesPanel,
    /// File preview shown over the browser's directory listing.
    FilePreview,
    /// Settings, shown in place of the transcript.
    Settings,
    /// Inline title editor for the open conversation.
    Rename,
    /// Confirmation before a conversation is permanently deleted.
    ConfirmDelete,
    /// Confirmation on the home page before a project leaves Wisp.
    ConfirmRemoveProject,
    /// Command palette, drawn over everything else.
    Palette,
    /// Question before a conversation stops asking for approvals.
    ConfirmFullAccess,
    /// Reasoning-effort choices above the composer.
    EffortMenu,
}

impl Layer {
    /// The surface this one is drawn inside, which it cannot outlive.
    fn parent(self) -> Option<Layer> {
        match self {
            Layer::FilePreview => Some(Layer::FilesPanel),
            _ => None,
        }
    }

    fn within(self, ancestor: Layer) -> bool {
        self == ancestor || self.parent().is_some_and(|parent| parent.within(ancestor))
    }
}

#[derive(Default)]
pub struct EscapeStack(Vec<Layer>);

impl EscapeStack {
    /// Opens `layer` as the topmost surface. Reopening moves it to the top.
    pub fn open(&mut self, layer: Layer) {
        self.0.retain(|open| *open != layer);
        self.0.push(layer);
    }

    /// Closes `layer` and the surfaces drawn inside it, wherever they sit in
    /// the stack. Unrelated layers stay open. Returns whether it was open.
    pub fn close(&mut self, layer: Layer) -> bool {
        let open = self.0.contains(&layer);
        self.0.retain(|other| !other.within(layer));
        open
    }

    /// Closes only the topmost surface and reports which one it was.
    pub fn dismiss_top(&mut self) -> Option<Layer> {
        self.0.pop()
    }

    pub fn contains(&self, layer: Layer) -> bool {
        self.0.contains(&layer)
    }
}

/// A bare Escape press. Modified chords stay available to platform shortcuts.
pub fn is_escape(key: &str, modified: bool) -> bool {
    key == "escape" && !modified
}

/// Window-wide commands that work wherever focus is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    NewConversation,
    Settings,
    Palette,
}

/// `primary` is Cmd on macOS and Ctrl elsewhere, held alone: adding Shift,
/// Alt or the other platform key leaves the chord to whoever else wants it.
pub fn shortcut(key: &str, primary: bool, others: bool) -> Option<Shortcut> {
    if !primary || others {
        return None;
    }
    match key {
        "n" => Some(Shortcut::NewConversation),
        "," => Some(Shortcut::Settings),
        "k" => Some(Shortcut::Palette),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_escape_closes_only_the_topmost_layer() {
        let mut stack = EscapeStack::default();
        assert_eq!(stack.dismiss_top(), None);
        stack.open(Layer::FilesPanel);
        stack.open(Layer::FilePreview);
        assert_eq!(stack.dismiss_top(), Some(Layer::FilePreview));
        assert!(stack.contains(Layer::FilesPanel));
        assert!(!stack.contains(Layer::FilePreview));
        assert_eq!(stack.dismiss_top(), Some(Layer::FilesPanel));
        assert!(!stack.contains(Layer::FilesPanel));
        assert_eq!(stack.dismiss_top(), None);
    }

    #[test]
    fn closing_a_parent_closes_its_children_and_reopening_does_not_duplicate() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::FilePreview);
        stack.open(Layer::FilePreview);
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::FilesPanel) && !stack.contains(Layer::FilePreview));
        assert!(!stack.close(Layer::FilesPanel));

        stack.open(Layer::FilesPanel);
        stack.open(Layer::FilePreview);
        assert!(stack.close(Layer::FilePreview));
        assert!(stack.contains(Layer::FilesPanel));
        assert_eq!(stack.dismiss_top(), Some(Layer::FilesPanel));
    }

    #[test]
    fn closing_a_layer_leaves_unrelated_layers_open_in_order() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::Rename);
        stack.open(Layer::FilePreview);
        stack.open(Layer::ConfirmDelete);
        // The panel takes its preview with it, not the conversation layers.
        assert!(stack.close(Layer::FilesPanel));
        assert!(!stack.contains(Layer::FilePreview));
        assert!(stack.contains(Layer::Rename) && stack.contains(Layer::ConfirmDelete));
        assert!(!stack.close(Layer::Settings));
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDelete));
        assert_eq!(stack.dismiss_top(), Some(Layer::Rename));
        assert_eq!(stack.dismiss_top(), None);
    }

    #[test]
    fn only_a_bare_escape_dismisses() {
        assert!(is_escape("escape", false));
        assert!(!is_escape("escape", true));
        assert!(!is_escape("enter", false));
    }

    #[test]
    fn shortcuts_need_exactly_the_platform_modifier() {
        assert_eq!(shortcut("n", true, false), Some(Shortcut::NewConversation));
        assert_eq!(shortcut(",", true, false), Some(Shortcut::Settings));
        // Typing the letter, or a longer chord, is not the command.
        assert_eq!(shortcut("n", false, false), None);
        assert_eq!(shortcut("n", true, true), None);
        assert_eq!(shortcut("k", true, false), Some(Shortcut::Palette));
        assert_eq!(shortcut("j", true, false), None);
    }

    #[test]
    fn the_palette_closes_first_and_leaves_what_is_under_it() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::Settings);
        stack.open(Layer::Palette);
        assert_eq!(stack.dismiss_top(), Some(Layer::Palette));
        assert!(stack.contains(Layer::Settings) && stack.contains(Layer::FilesPanel));
        // Reopening it over a layer opened meanwhile keeps it topmost.
        stack.open(Layer::Palette);
        stack.open(Layer::ConfirmDelete);
        stack.open(Layer::Palette);
        assert_eq!(stack.dismiss_top(), Some(Layer::Palette));
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmDelete));
    }

    #[test]
    fn composer_layers_close_one_at_a_time_and_leave_the_panel() {
        let mut stack = EscapeStack::default();
        stack.open(Layer::FilesPanel);
        stack.open(Layer::EffortMenu);
        assert_eq!(stack.dismiss_top(), Some(Layer::EffortMenu));
        stack.open(Layer::ConfirmFullAccess);
        assert_eq!(stack.dismiss_top(), Some(Layer::ConfirmFullAccess));
        assert!(stack.contains(Layer::FilesPanel));
        assert!(!stack.close(Layer::EffortMenu));
    }
}
