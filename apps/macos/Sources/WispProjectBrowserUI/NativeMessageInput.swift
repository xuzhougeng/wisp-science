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

    func apply(_ text: String) {
        guard !hasMarkedText(), string != text else { return }
        string = text
        needsDisplay = true
        setSelectedRange(NSRange(location: (text as NSString).length, length: 0))
    }
    override func didChangeText() { super.didChangeText(); needsDisplay = true; onChange?(string) }
    override func keyDown(with event: NSEvent) {
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
