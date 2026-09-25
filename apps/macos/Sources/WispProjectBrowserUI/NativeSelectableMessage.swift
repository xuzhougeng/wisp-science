import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeSelectableMessage: NSViewRepresentable {
    let text: AttributedString
    let saved: [String]
    let quote: ((String) -> Void)?
    let save: ((String) -> Void)?
    var monospaced = false
    var markdown: String?
    var revealed: String?
    @Environment(\.colorScheme) private var scheme

    func makeNSView(context: Context) -> NativeMessageTextView {
        let view = NativeMessageTextView(frame: .zero)
        view.isEditable = false; view.isSelectable = true; view.isRichText = true
        view.drawsBackground = false
        view.isHorizontallyResizable = false; view.isVerticallyResizable = true
        view.textContainerInset = .zero
        view.textContainer?.lineFragmentPadding = 0
        view.textContainer?.widthTracksTextView = true
        view.textContainer?.containerSize = NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude)
        configure(view)
        return view
    }
    func updateNSView(_ view: NativeMessageTextView, context: Context) { configure(view) }
    private func configure(_ view: NativeMessageTextView) {
        view.quote = quote; view.save = save
        view.linkTextAttributes = [.foregroundColor: NSColor(WispDesign.color("clay", scheme)), .underlineStyle: NSUnderlineStyle.single.rawValue]
        view.apply(markdown.map { NativeMarkdownContent.render($0, saved: saved, revealed: revealed, scheme: scheme) }
                   ?? Self.content(text, saved: saved, scheme: scheme, monospaced: monospaced))
    }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NativeMessageTextView, context: Context) -> CGSize? {
        let width = max(1, proposal.width ?? 400)
        nsView.frame.size.width = width
        guard let container = nsView.textContainer, let layout = nsView.layoutManager else { return nil }
        container.containerSize = NSSize(width: width, height: CGFloat.greatestFiniteMagnitude)
        layout.ensureLayout(for: container)
        return CGSize(width: width, height: max(20, ceil(layout.usedRect(for: container).height)))
    }
    static func dismantleNSView(_ view: NativeMessageTextView, coordinator: ()) { view.quote = nil; view.save = nil }
    static func content(_ text: AttributedString, saved: [String], scheme: ColorScheme, monospaced: Bool = false) -> NSAttributedString {
        let value = NSMutableAttributedString(text)
        let plain = value.string
        let whole = NSRange(location: 0, length: value.length)
        let key = monospaced ? "code" : "ui"
        let size = UserDefaults.standard.double(forKey: "nativeSettings." + key + "_font_size")
        let family = UserDefaults.standard.string(forKey: "nativeSettings." + key + "_font_family") ?? ""
        let pointSize = size > 0 ? size : (monospaced ? 12 : 14)
        let fallback = monospaced ? NSFont.monospacedSystemFont(ofSize: pointSize, weight: .regular) : NSFont.systemFont(ofSize: pointSize)
        let base = NSFont(name: family, size: pointSize) ?? fallback
        value.addAttributes([.font: base, .foregroundColor: NSColor(WispDesign.color("text", scheme))], range: whole)
        func nsRange(_ range: Range<Int>) -> NSRange {
            let start = plain.index(plain.startIndex, offsetBy: range.lowerBound)
            let end = plain.index(plain.startIndex, offsetBy: range.upperBound)
            return NSRange(start..<end, in: plain)
        }
        for run in text.runs {
            let start = text.characters.distance(from: text.startIndex, to: run.range.lowerBound)
            let end = text.characters.distance(from: text.startIndex, to: run.range.upperBound)
            let range = nsRange(start..<end)
            var font = base
            if let intent = run.inlinePresentationIntent {
                if intent.contains(.code) { font = .monospacedSystemFont(ofSize: base.pointSize, weight: .regular) }
                if intent.contains(.stronglyEmphasized) { font = NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask) }
                if intent.contains(.emphasized) { font = NSFontManager.shared.convert(font, toHaveTrait: .italicFontMask) }
                if intent.contains(.strikethrough) { value.addAttribute(.strikethroughStyle, value: NSUnderlineStyle.single.rawValue, range: range) }
            }
            value.addAttribute(.font, value: font, range: range)
        }
        for excerpt in Set(saved) {
            for range in NativeSavedExcerpt.ranges(in: plain, excerpt: excerpt) {
                value.addAttributes([.underlineStyle: NSUnderlineStyle.single.rawValue, .underlineColor: NSColor(WispDesign.color("clay", scheme))], range: nsRange(range))
            }
        }
        return value
    }
}

/// Menu actions retain the selection and callback from menu creation, not live view state.
final class NativeSelectionAction: NSObject {
    private let perform: () -> Void
    init(_ perform: @escaping () -> Void) { self.perform = perform }
    @objc func invoke(_ sender: Any?) { perform() }
}
class NativeMessageTextView: NSTextView {
    var quote: ((String) -> Void)?
    var save: ((String) -> Void)?
    func apply(_ content: NSAttributedString) {
        guard let storage = textStorage, !storage.isEqual(to: content) else { return }
        let selected = selectedRange()
        storage.setAttributedString(content)
        let start = min(selected.location, storage.length)
        setSelectedRange(NSRange(location: start, length: min(selected.length, storage.length - start)))
    }
    func selectionActions() -> [NSMenuItem] {
        let range = selectedRange()
        let source = string as NSString
        guard range.location != NSNotFound, range.length > 0, range.location <= source.length,
              range.length <= source.length - range.location else { return [] }
        let selected = source.substring(with: range)
        guard !selected.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return [] }
        return [("引用到侧聊", "chat", quote), ("收藏划线", "star", save)].compactMap { title, icon, callback in
            guard let callback else { return nil }
            let action = NativeSelectionAction { callback(selected) }
            let item = NSMenuItem(title: title, action: #selector(NativeSelectionAction.invoke(_:)), keyEquivalent: "")
            item.target = action; item.representedObject = action
            let image = WispDesign.image("icon-" + icon); image.size = NSSize(width: 16, height: 16); item.image = image
            return item
        }
    }
    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = super.menu(for: event) ?? NSMenu()
        let point = convert(event.locationInWindow, from: nil)
        let actions = selectionActions() + blockActions(at: characterIndexForInsertion(at: point))
        if !actions.isEmpty { menu.addItem(.separator()); actions.forEach(menu.addItem) }
        return menu
    }
    func blockActions(at index: Int, copy: @escaping (String) -> Void = { text in
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(text, forType: .string)
    }) -> [NSMenuItem] {
        guard let storage = textStorage, index >= 0, index < storage.length else { return [] }
        return [("复制代码", NativeMarkdownContent.codeCopy), ("复制表格", NativeMarkdownContent.tableCopy)].compactMap { title, key in
            guard let text = storage.attribute(key, at: index, effectiveRange: nil) as? String else { return nil }
            let action = NativeSelectionAction { copy(text) }
            let item = NSMenuItem(title: title, action: #selector(NativeSelectionAction.invoke(_:)), keyEquivalent: "")
            item.target = action; item.representedObject = action
            return item
        }
    }
}
