import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeSideChatInputTests: XCTestCase {
    @MainActor func testPlaceholderNeverBecomesDraftAndContentHeightIsBounded() {
        let (window, editor) = editor(); defer { window.close() }
        editor.placeholder = "请输入问题…"
        let widthBefore = editor.textContainer?.containerSize.width
        let tracksBefore = editor.textContainer?.widthTracksTextView
        XCTAssertTrue(editor.showsPlaceholder)
        XCTAssertEqual(editor.string, "")
        XCTAssertEqual(editor.fittedHeight(width: 300), 64)
        editor.apply(String(repeating: "long text wraps across the composer width\n", count: 30))
        XCTAssertFalse(editor.showsPlaceholder)
        XCTAssertEqual(editor.fittedHeight(width: 300), 160)
        _ = editor.fittedHeight(width: 1)
        XCTAssertEqual(editor.textContainer?.containerSize.width, widthBefore)
        XCTAssertEqual(editor.textContainer?.widthTracksTextView, tracksBefore)
        editor.apply("")
        XCTAssertTrue(editor.showsPlaceholder)
        XCTAssertEqual(editor.fittedHeight(width: 300), 64)
    }
    @MainActor func testHostedEmptyComposerHasClickableEditorAndMeasurementDoesNotCollapseIt() throws {
        let host = NSHostingView(rootView: NativeMessageInput(text: .constant(""), canSubmit: { false }, submit: {}, placeholder: "请输入问题…", fitsContent: true)
            .fixedSize(horizontal: false, vertical: true))
        host.frame = NSRect(x: 0, y: 0, width: 380, height: 160)
        host.layoutSubtreeIfNeeded()
        func find(_ view: NSView) -> NativeComposerTextView? {
            if let editor = view as? NativeComposerTextView { return editor }
            return view.subviews.compactMap(find).first
        }
        let editor = try XCTUnwrap(find(host))
        XCTAssertGreaterThan(editor.frame.width, 300)
        XCTAssertGreaterThanOrEqual(editor.frame.height, 64)
        XCTAssertTrue(editor.textContainer?.widthTracksTextView == true)
        _ = editor.fittedHeight(width: 1)
        XCTAssertGreaterThan(editor.frame.width, 300)
        editor.insertText("sample draft", replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertEqual(editor.string, "sample draft")
    }
    @MainActor func testModifierPreferenceChangesLiveAndShiftAlwaysMakesNewline() {
        let (window, editor) = editor(); defer { window.close() }
        var sends = 0
        editor.canSubmit = { true }; editor.submit = { sends += 1 }
        editor.sendWithModifier = true
        editor.apply("first")
        editor.keyDown(with: enter(window))
        XCTAssertEqual(editor.string, "first\n"); XCTAssertEqual(sends, 0)
        editor.keyDown(with: enter(window, flags: .command))
        editor.keyDown(with: enter(window, flags: .control, keypad: true))
        XCTAssertEqual(sends, 2)
        editor.keyDown(with: enter(window, flags: [.shift, .command]))
        XCTAssertEqual(editor.string, "first\n\n"); XCTAssertEqual(sends, 2)
        editor.sendWithModifier = false
        editor.keyDown(with: enter(window))
        XCTAssertEqual(sends, 3)
        editor.isEditable = false
        editor.keyDown(with: enter(window))
        XCTAssertEqual(sends, 3)
    }
    func testAuthoritativePreferencesRefreshCachedSendingPolicyWithoutRequiringTheme() {
        let suite = "native-composer-tests-" + UUID().uuidString
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        WispDesign.apply(.object(["send_with_modifier": .bool(true)]), defaults: defaults)
        XCTAssertTrue(defaults.bool(forKey: "nativeSettings.send_with_modifier"))
        WispDesign.apply(.null, defaults: defaults)
        XCTAssertTrue(defaults.bool(forKey: "nativeSettings.send_with_modifier"))
        WispDesign.apply(.object(["send_with_modifier": .bool(false)]), defaults: defaults)
        XCTAssertFalse(defaults.bool(forKey: "nativeSettings.send_with_modifier"))
    }
    @MainActor private func editor() -> (NSWindow, NativeComposerTextView) {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 340, height: 100), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let editor = NativeComposerTextView(frame: NSRect(x: 0, y: 0, width: 340, height: 100))
        editor.isRichText = false; editor.isEditable = true; editor.isSelectable = true
        window.contentView = editor
        XCTAssertTrue(window.makeFirstResponder(editor))
        return (window, editor)
    }
    @MainActor private func enter(_ window: NSWindow, flags: NSEvent.ModifierFlags = [], keypad: Bool = false) -> NSEvent {
        NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: keypad ? 76 : 36)!
    }
    func testReturnPolicyAlwaysLetsIMEConfirmComposition() {
        XCTAssertEqual(NativeMessageReturnAction.resolve(shift: false, composing: false), .send)
        XCTAssertEqual(NativeMessageReturnAction.resolve(shift: true, composing: false), .newline)
        XCTAssertEqual(NativeMessageReturnAction.resolve(shift: false, composing: true), .composition)
        XCTAssertEqual(NativeMessageReturnAction.resolve(shift: true, composing: true), .composition)
    }
    @MainActor func testReturnSubmitsAndBusyReturnDoesNotInsertOrResubmit() {
        let (window, editor) = editor(); defer { window.close() }
        var sends = 0; var allowed = true
        editor.canSubmit = { allowed }; editor.submit = { sends += 1 }
        editor.apply("question")
        editor.keyDown(with: enter(window))
        XCTAssertEqual(sends, 1); XCTAssertEqual(editor.string, "question")
        allowed = false
        editor.keyDown(with: enter(window)); editor.keyDown(with: enter(window, keypad: true))
        XCTAssertEqual(sends, 1); XCTAssertEqual(editor.string, "question")
    }
    @MainActor func testShiftReturnInsertsNewlineAndNotifiesBinding() {
        let (window, editor) = editor(); defer { window.close() }
        var sends = 0; var changed: String?
        editor.canSubmit = { true }; editor.submit = { sends += 1 }; editor.onChange = { changed = $0 }
        editor.apply("line")
        editor.keyDown(with: enter(window, flags: .shift))
        XCTAssertEqual(sends, 0)
        XCTAssertEqual(editor.string, "line\n")
        XCTAssertEqual(changed, "line\n")
    }
    @MainActor func testIMEConfirmationAndExternalUpdatesDoNotSendOrReplaceMarkedText() {
        let (window, editor) = editor(); defer { window.close() }
        var sends = 0
        editor.canSubmit = { true }; editor.submit = { sends += 1 }
        editor.setMarkedText("候选", selectedRange: NSRange(location: 2, length: 0), replacementRange: NSRange(location: NSNotFound, length: 0))
        XCTAssertTrue(editor.hasMarkedText())
        let composing = editor.string
        editor.apply("external draft")
        XCTAssertEqual(editor.string, composing)
        editor.keyDown(with: enter(window))
        XCTAssertEqual(sends, 0)
        editor.unmarkText(); editor.apply("confirmed")
        XCTAssertEqual(editor.string, "confirmed")
    }
    @MainActor func testCommandReturnIsConsumedOnlyByFocusedSideEditor() {
        let (window, editor) = editor(); defer { window.close() }
        var sends = 0
        editor.canSubmit = { true }; editor.submit = { sends += 1 }; editor.apply("side question")
        XCTAssertTrue(editor.performKeyEquivalent(with: enter(window, flags: .command)))
        XCTAssertEqual(sends, 1)
        _ = window.makeFirstResponder(nil)
        XCTAssertFalse(editor.performKeyEquivalent(with: enter(window, flags: .command)))
        XCTAssertEqual(sends, 1)
    }
    @MainActor func testUnchangedDraftDoesNotMoveSelection() {
        let (window, editor) = editor(); defer { window.close() }
        editor.apply("abc def"); editor.setSelectedRange(NSRange(location: 1, length: 2))
        editor.apply("abc def")
        XCTAssertEqual(editor.selectedRange(), NSRange(location: 1, length: 2))
        editor.apply("")
        XCTAssertEqual(editor.string, "")
        XCTAssertEqual(editor.selectedRange().location, 0)
    }
}
