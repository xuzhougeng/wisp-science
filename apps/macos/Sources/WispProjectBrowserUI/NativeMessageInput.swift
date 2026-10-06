import AppKit
import SwiftUI
import WispProjectBrowser

/// A native editor owns Return handling, so keyboard events never escape to the
/// main conversation's send shortcut. IME composition stays with AppKit.
struct NativeMessageInput: NSViewRepresentable {
    @Binding var text: String
    let canSubmit: () -> Bool
    let submit: () -> Void
    var sendWithModifier = false
    var editable = true
    var accessibilityLabel = "侧聊问题"
    var fontSize: CGFloat = 13
    var placeholder = ""
    var fitsContent = false
    var completions: NativeComposerCompletionModel?
    var referencesAvailable: () -> Bool = { false }
    var selectReference: (NativeComposerReference) -> Bool = { _ in false }
    var completionCommands: [NativeComposerCommand] = []
    var executeCommand: (NativeComposerCommand, String) -> Void = { _, _ in }
    @Environment(\.colorScheme) private var scheme

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView()
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.drawsBackground = false
        let editor = NativeComposerTextView(frame: NSRect(x: 0, y: 0, width: 400, height: 64))
        editor.minSize = NSSize(width: 0, height: 64)
        editor.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        editor.isRichText = false
        editor.isEditable = true
        editor.isSelectable = true
        editor.allowsUndo = true
        editor.isAutomaticQuoteSubstitutionEnabled = false
        editor.isAutomaticDashSubstitutionEnabled = false
        editor.isAutomaticTextReplacementEnabled = false
        editor.isVerticallyResizable = true
        editor.isHorizontallyResizable = false
        editor.autoresizingMask = [.width]
        editor.textContainer?.widthTracksTextView = true
        editor.textContainer?.containerSize = NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        editor.textContainerInset = NSSize(width: 6, height: 6)
        scroll.documentView = editor
        configure(editor)
        return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let editor = scroll.documentView as? NativeComposerTextView else { return }
        configure(editor)
    }
    private func configure(_ editor: NativeComposerTextView) {
        editor.onChange = { text = $0 }
        editor.canSubmit = canSubmit
        editor.submit = submit
        editor.sendWithModifier = sendWithModifier
        editor.completions = completions
        completions?.editor = editor
        editor.referencesAvailable = referencesAvailable
        editor.selectReference = selectReference
        editor.completionCommands = completionCommands
        editor.executeCommand = executeCommand
        editor.isEditable = editable
        editor.setAccessibilityLabel(accessibilityLabel)
        editor.placeholder = placeholder
        editor.font = .systemFont(ofSize: fontSize)
        editor.textColor = NSColor(WispDesign.color("text", scheme))
        editor.backgroundColor = NSColor(WispDesign.color("bg-elev", scheme))
        editor.insertionPointColor = editor.textColor ?? .textColor
        editor.apply(text)
    }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NSScrollView, context: Context) -> CGSize? {
        guard fitsContent, let editor = nsView.documentView as? NativeComposerTextView else { return nil }
        let width = max(1, proposal.width ?? 400)
        return CGSize(width: width, height: editor.fittedHeight(width: width))
    }
    static func dismantleNSView(_ scroll: NSScrollView, coordinator: ()) {
        guard let editor = scroll.documentView as? NativeComposerTextView else { return }
        editor.completions?.detach(editor)
        editor.completions = nil; editor.referencesAvailable = nil; editor.selectReference = nil; editor.executeCommand = nil
        editor.onChange = nil; editor.canSubmit = nil; editor.submit = nil
    }
}

class NativeComposerTextView: NSTextView {
    var placeholder = "" { didSet { needsDisplay = true } }
    var showsPlaceholder: Bool { string.isEmpty && !hasMarkedText() }
    func fittedHeight(width: CGFloat) -> CGFloat {
        // SwiftUI probes ideal and zero widths too. Never mutate the live
        // text container while measuring; doing so can collapse the editor.
        let storage = NSTextStorage(string: string, attributes: [.font: font ?? NSFont.systemFont(ofSize: 14)])
        let layout = NSLayoutManager()
        let container = NSTextContainer(containerSize: NSSize(width: max(1, width - textContainerInset.width * 2), height: CGFloat.greatestFiniteMagnitude))
        container.lineFragmentPadding = textContainer?.lineFragmentPadding ?? 5
        storage.addLayoutManager(layout); layout.addTextContainer(container)
        layout.ensureLayout(for: container)
        return min(160, max(64, ceil(layout.usedRect(for: container).height + textContainerInset.height * 2)))
    }
    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        if showsPlaceholder && !placeholder.isEmpty {
            let rect = bounds.insetBy(dx: textContainerInset.width + (textContainer?.lineFragmentPadding ?? 0), dy: textContainerInset.height)
            (placeholder as NSString).draw(in: rect, withAttributes: [.font: font ?? NSFont.systemFont(ofSize: 14), .foregroundColor: NSColor.placeholderTextColor])
        }
    }
    var sendWithModifier = false
    var onChange: ((String) -> Void)?
    var canSubmit: (() -> Bool)?
    var submit: (() -> Void)?
    weak var completions: NativeComposerCompletionModel?
    var referencesAvailable: (() -> Bool)?
    var selectReference: ((NativeComposerReference) -> Bool)?
    var completionCommands: [NativeComposerCommand] = []
    var executeCommand: ((NativeComposerCommand, String) -> Void)?
    private var editing = false

    func apply(_ text: String) {
        guard !hasMarkedText(), string != text else { return }
        completions?.dismiss()
        string = text
        needsDisplay = true
        setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
    }
    override func didChangeText() {
        super.didChangeText(); needsDisplay = true; onChange?(string)
        if !editing { completions?.dismiss() }
    }
    override func insertText(_ insertString: Any, replacementRange: NSRange) {
        let wasEditing = editing; editing = true
        super.insertText(insertString, replacementRange: replacementRange)
        editing = wasEditing
        if !wasEditing { completions?.edited(insertion: true) }
    }
    override func deleteBackward(_ sender: Any?) {
        let wasEditing = editing; editing = true; super.deleteBackward(sender); editing = wasEditing
        if !wasEditing { completions?.edited(insertion: false) }
    }
    override func deleteForward(_ sender: Any?) {
        let wasEditing = editing; editing = true; super.deleteForward(sender); editing = wasEditing
        if !wasEditing { completions?.edited(insertion: false) }
    }
    override func paste(_ sender: Any?) {
        completions?.dismiss(); editing = true; super.paste(sender); editing = false
    }
    override func readSelection(from pboard: NSPasteboard, type: NSPasteboard.PasteboardType) -> Bool {
        // Plain-text paste, Services and drag/pasteboard insertion share this
        // AppKit boundary and must not masquerade as typed trigger keys.
        completions?.dismiss(); let wasEditing = editing; editing = true
        let result = super.readSelection(from: pboard, type: type)
        editing = wasEditing
        return result
    }
    override func setMarkedText(_ string: Any, selectedRange: NSRange, replacementRange: NSRange) {
        let wasEditing = editing; completions?.suspendComposition(); editing = true
        super.setMarkedText(string, selectedRange: selectedRange, replacementRange: replacementRange)
        editing = wasEditing
    }
    override func unmarkText() {
        let wasEditing = editing; editing = true; super.unmarkText(); editing = wasEditing
        if !wasEditing { completions?.edited(insertion: true) }
    }
    override func setSelectedRanges(_ ranges: [NSValue], affinity: NSSelectionAffinity, stillSelecting: Bool) {
        super.setSelectedRanges(ranges, affinity: affinity, stillSelecting: stillSelecting)
        if !editing { completions?.selectionChanged() }
    }
    func replaceCompletion(_ range: NSRange, with text: String) {
        editing = true
        // AppKit's edit path retains undo and puts the caret immediately after
        // the replacement, including when a token sits in the middle of text.
        insertText(text, replacementRange: range)
        editing = false
        window?.makeFirstResponder(self)
    }
    override func keyDown(with event: NSEvent) {
        if isEditable, !hasMarkedText(), completions?.isOpen == true {
            switch event.keyCode {
            case 125: completions?.move(1); return
            case 126: completions?.move(-1); return
            case 36, 76, 48: completions?.accept(); return
            case 53: completions?.dismiss(); return
            default: break
            }
        }
        guard event.keyCode == 36 || event.keyCode == 76 else { super.keyDown(with: event); return }
        switch NativeMessageReturnAction.resolve(shift: event.modifierFlags.contains(.shift), composing: hasMarkedText(), sendWithModifier: sendWithModifier, modifier: !event.modifierFlags.intersection([.command, .control]).isEmpty) {
        case .composition: super.keyDown(with: event)
        case .newline: if isEditable { insertNewline(nil) }
        case .send: if isEditable && canSubmit?() == true { submit?() }
        }
    }
    override func performKeyEquivalent(with event: NSEvent) -> Bool {
        // Return shortcuts belong only to the focused editor, including IME
        // composition; other windows and editors must never submit this draft.
        if window?.firstResponder === self, !event.modifierFlags.intersection([.command, .control]).isEmpty,
           event.keyCode == 36 || event.keyCode == 76 {
            keyDown(with: event)
            return true
        }
        return super.performKeyEquivalent(with: event)
    }
}
