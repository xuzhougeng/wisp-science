import AppKit

/// One successful search selection may return focus to its original editor,
/// once the search sheet has finished dismissing. Never activate a window.
@MainActor
final class NativeSearchComposerFocus {
    private weak var editor: NativeComposerTextView?
    private weak var window: NSWindow?
    private let valid: () -> Bool
    private var consumed = false

    init(editor: NativeComposerTextView?, valid: @escaping () -> Bool) {
        self.editor = editor
        window = editor?.window
        self.valid = valid
    }

    @discardableResult
    func restore(keyWindow: NSWindow?) -> Bool {
        guard !consumed else { return false }
        consumed = true
        guard valid(), let editor, let window, editor.window === window,
              keyWindow === window, window.attachedSheet == nil,
              editor.isEditable, !editor.isHiddenOrHasHiddenAncestor,
              !editor.hasMarkedText() else { return false }
        return window.makeFirstResponder(editor)
    }
}
