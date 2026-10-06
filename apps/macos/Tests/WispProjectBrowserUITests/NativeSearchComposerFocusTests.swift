import AppKit
import XCTest
@testable import WispProjectBrowserUI

final class NativeSearchComposerFocusTests: XCTestCase {
    @MainActor private func window() -> NSWindow {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 500, height: 300), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        return window
    }
    @MainActor private func editor(in window: NSWindow) -> NativeComposerTextView {
        let editor = NativeComposerTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 100))
        editor.isEditable = true; editor.string = "Unsent research question"
        window.contentView!.addSubview(editor)
        editor.setSelectedRange(NSRange(location: 7, length: 0))
        return editor
    }
    @MainActor func testSuccessfulSelectionRestoresOnlyOriginalEditorOnceWithoutEditingOrSubmitting() {
        let window = window(); defer { window.close() }
        let editor = editor(in: window); var submitted = 0, edited = 0
        editor.submit = { submitted += 1 }; editor.onChange = { _ in edited += 1 }
        let focus = NativeSearchComposerFocus(editor: editor, valid: { true })
        XCTAssertTrue(focus.restore(keyWindow: window)); XCTAssertTrue(window.firstResponder === editor)
        XCTAssertEqual(editor.string, "Unsent research question"); XCTAssertEqual(editor.selectedRange(), NSRange(location: 7, length: 0))
        XCTAssertEqual(submitted, 0); XCTAssertEqual(edited, 0)
        XCTAssertFalse(focus.restore(keyWindow: window))
    }
    @MainActor func testChangedScopeAndAnotherKeyWindowConsumeRequestWithoutLaterFocusSteal() {
        let source = window(), other = window(); defer { source.close(); other.close() }
        let editor = editor(in: source)
        var valid = true
        let changed = NativeSearchComposerFocus(editor: editor, valid: { valid })
        valid = false; XCTAssertFalse(changed.restore(keyWindow: source))
        valid = true; XCTAssertFalse(changed.restore(keyWindow: source))
        let switched = NativeSearchComposerFocus(editor: editor, valid: { true })
        XCTAssertFalse(switched.restore(keyWindow: other)); XCTAssertFalse(switched.restore(keyWindow: source))
        let inactive = NativeSearchComposerFocus(editor: editor, valid: { true })
        XCTAssertFalse(inactive.restore(keyWindow: nil))
    }
    @MainActor func testDetachedReparentedHiddenReadOnlyAndComposingEditorsCannotReceiveFocus() {
        for state in ["detached", "reparented", "hidden", "read-only", "composing"] {
            let source = window(), other = window(); defer { source.close(); other.close() }
            let editor = editor(in: source), focus = NativeSearchComposerFocus(editor: editor, valid: { true })
            switch state {
            case "detached": editor.removeFromSuperview()
            case "reparented": editor.removeFromSuperview(); other.contentView!.addSubview(editor)
            case "hidden": source.contentView!.isHidden = true
            case "read-only": editor.isEditable = false
            default: editor.setMarkedText("候选", selectedRange: NSRange(location: 0, length: 2), replacementRange: NSRange(location: NSNotFound, length: 0))
            }
            XCTAssertFalse(focus.restore(keyWindow: source), state)
        }
    }
    @MainActor func testAttachedSheetRetainsItsFocusAndAnUnmountedEditorCannotBeReplaced() {
        let source = window(), sheet = window(); defer { source.close(); sheet.close() }
        let original = editor(in: source), focus = NativeSearchComposerFocus(editor: original, valid: { true })
        source.beginSheet(sheet)
        XCTAssertTrue(source.attachedSheet === sheet)
        let sheetResponder = source.firstResponder
        XCTAssertFalse(focus.restore(keyWindow: source))
        XCTAssertTrue(source.firstResponder === sheetResponder)
        source.endSheet(sheet)
        let unmounted = NativeSearchComposerFocus(editor: original, valid: { true })
        original.removeFromSuperview(); _ = editor(in: source)
        let replacementResponder = source.firstResponder
        XCTAssertFalse(unmounted.restore(keyWindow: source))
        XCTAssertTrue(source.firstResponder === replacementResponder)
    }
}
