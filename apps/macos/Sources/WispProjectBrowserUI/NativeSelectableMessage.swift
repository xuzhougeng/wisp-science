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
    var images: [String: NSImage] = [:]
    var unavailableImages: Set<String> = []
    var openImage: ((String) -> Void)?
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
        view.openImage = openImage
        view.linkTextAttributes = [.foregroundColor: NSColor(WispDesign.color("clay", scheme)), .underlineStyle: NSUnderlineStyle.single.rawValue]
        view.apply(markdown.map { NativeMarkdownContent.render($0, saved: saved, revealed: revealed, scheme: scheme, width: view.bounds.width > 0 ? view.bounds.width : 600, images: images, unavailableImages: unavailableImages) }
                   ?? Self.content(text, saved: saved, scheme: scheme, monospaced: monospaced))
    }
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: NativeMessageTextView, context: Context) -> CGSize? {
        let width = max(1, proposal.width ?? 400)
        if nsView.frame.size.width != width { nsView.frame.size.width = width; configure(nsView) }
        guard let container = nsView.textContainer, let layout = nsView.layoutManager else { return nil }
        container.containerSize = NSSize(width: width, height: CGFloat.greatestFiniteMagnitude)
        layout.ensureLayout(for: container)
        return CGSize(width: width, height: nsView.contentHeight())
    }
    static func dismantleNSView(_ view: NativeMessageTextView, coordinator: ()) { view.quote = nil; view.save = nil; view.openImage = nil }
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
            if let image = run.imageURL { value.addAttribute(NativeImageContent.referenceKey, value: image.absoluteString, range: range) }
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
    func contentHeight() -> CGFloat {
        guard let storage = textStorage, let layout = layoutManager, let container = textContainer else { return 20 }
        layout.ensureLayout(for: container)
        var height = layout.usedRect(for: container).height
        // Markdown paragraph separators must remain selectable, but the empty
        // AppKit insertion line after a final paragraph adds no reading content.
        if storage.length > 0, storage.string.hasSuffix("\n"),
           let paragraph = storage.attribute(.paragraphStyle, at: storage.length - 1, effectiveRange: nil) as? NSParagraphStyle,
           paragraph.textBlocks.isEmpty {
            let glyphs = layout.glyphRange(forCharacterRange: NSRange(location: 0, length: max(0, storage.length - 1)), actualCharacterRange: nil)
            height = layout.boundingRect(forGlyphRange: glyphs, in: container).maxY
        }
        return max(20, ceil(height))
    }
    override func accessibilityValue() -> String? {
        textStorage.map { NativeMathContent.plainText($0) } ?? ""
    }
    override func copy(_ sender: Any?) {
        guard let textStorage, selectedRange().location != NSNotFound, selectedRange().length > 0,
              NSMaxRange(selectedRange()) <= textStorage.length else { return }
        copyBlock(NativeMathContent.plainText(textStorage.attributedSubstring(from: selectedRange())))
    }
    var copyBlock: (String) -> Void = { text in
        NSPasteboard.general.clearContents(); NSPasteboard.general.setString(text, forType: .string)
    }
    private var copyLayoutWidth: CGFloat = -1
    private var copyButtons: [NSButton] = []
    private var copyActions: [NativeSelectionAction] = []
    private var imageButtons: [NSButton] = []
    private var imageActions: [NativeSelectionAction] = []
    var openImage: ((String) -> Void)?
    override func accessibilityChildren() -> [Any]? {
        let existing = super.accessibilityChildren() ?? []
        return existing + (copyButtons + imageButtons).filter { button in !existing.contains { ($0 as? NSView) === button } }
    }
    override func layout() {
        super.layout()
        layoutCopyButtons()
    }
    /// Buttons are native children, so keyboard/VoiceOver can reach each block
    /// without replacing the selectable document with disconnected SwiftUI text.
    func layoutCopyButtons() {
        guard bounds.width != copyLayoutWidth else { return }
        copyLayoutWidth = bounds.width
        let previous = Dictionary(uniqueKeysWithValues: copyButtons.compactMap { button in button.identifier.map { ($0.rawValue, button) } })
        copyButtons = []; copyActions = []
        guard let storage = textStorage, let layout = layoutManager, let container = textContainer else { return }
        var seenBlocks: Set<String> = []
        storage.enumerateAttribute(NativeMarkdownContent.copyBlockID, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            guard let blockID = value as? String, range.length > 0, seenBlocks.insert(blockID).inserted else { return }
            let code = storage.attribute(NativeMarkdownContent.codeCopy, at: range.location, effectiveRange: nil) as? String
            let table = storage.attribute(NativeMarkdownContent.tableCopy, at: range.location, effectiveRange: nil) as? String
            let formula = storage.attribute(NativeMathContent.formulaCopy, at: range.location, effectiveRange: nil) as? String
            guard let text = code ?? table ?? formula else { return }
            let title = localized(code != nil ? "复制代码" : table != nil ? "复制表格" : "复制公式")
            let identifier = "message-block-copy-" + blockID
            let glyphs = layout.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            let rect = layout.boundingRect(forGlyphRange: glyphs, in: container)
            let action = NativeSelectionAction { [copyBlock] in copyBlock(text) }
            let button = previous[identifier] ?? NSButton(title: "", target: nil, action: nil)
            button.target = action; button.action = #selector(NativeSelectionAction.invoke(_:))
            button.image = WispDesign.image("icon-copy")
            button.imageScaling = .scaleProportionallyDown
            button.bezelStyle = .regularSquare
            button.isBordered = true
            button.toolTip = title
            button.setAccessibilityLabel(title)
            button.identifier = NSUserInterfaceItemIdentifier(identifier)
            button.frame = NSRect(x: max(0, bounds.width - 32), y: max(0, textContainerOrigin.y + rect.minY - (code != nil ? 4 : 28)), width: 24, height: 24)
            if button.superview !== self { addSubview(button) }
            copyButtons.append(button); copyActions.append(action)
        }
        for button in previous.values where !copyButtons.contains(where: { $0 === button }) { button.removeFromSuperview() }
        imageButtons.forEach { $0.removeFromSuperview() }; imageButtons = []; imageActions = []
        storage.enumerateAttribute(NativeImageContent.previewKey, in: NSRange(location: 0, length: storage.length)) { value, range, _ in
            guard let reference = value as? String else { return }
            let glyphs = layout.glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            let rect = layout.boundingRect(forGlyphRange: glyphs, in: container)
            let action = NativeSelectionAction { [weak self] in self?.openImage?(reference) }
            let button = NSButton(title: "", target: action, action: #selector(NativeSelectionAction.invoke(_:)))
            button.isBordered = false
            let name = storage.attribute(NativeMathContent.sourceKey, at: range.location, effectiveRange: nil) as? String ?? reference
            let title = localized("打开图片预览") + " · " + (name.isEmpty ? reference : name)
            button.toolTip = title; button.setAccessibilityLabel(title)
            button.identifier = NSUserInterfaceItemIdentifier("message-image-\(range.location)")
            button.frame = rect.offsetBy(dx: textContainerOrigin.x, dy: textContainerOrigin.y)
            addSubview(button); imageButtons.append(button); imageActions.append(action)
        }
    }

    var quote: ((String) -> Void)?
    var save: ((String) -> Void)?
    func apply(_ content: NSAttributedString) {
        guard let storage = textStorage, !storage.isEqual(to: content) else { return }
        let selected = selectedRange()
        let plainStart = selected.location <= storage.length ? (NativeMathContent.plainText(storage.attributedSubstring(from: NSRange(location: 0, length: selected.location))) as NSString).length : 0
        let plainLength = NSMaxRange(selected) <= storage.length ? (NativeMathContent.plainText(storage.attributedSubstring(from: selected)) as NSString).length : 0
        storage.setAttributedString(content)
        copyLayoutWidth = -1
        needsLayout = true
        if selected.length > 0, plainLength > 0, plainStart + plainLength <= (NativeMathContent.plainText(content) as NSString).length {
            setSelectedRange(NativeMathContent.renderedRange(NSRange(location: plainStart, length: plainLength), in: content))
        } else {
            let start = min(selected.location, storage.length)
            setSelectedRange(NSRange(location: start, length: min(selected.length, storage.length - start)))
        }
    }
    func selectionActions() -> [NSMenuItem] {
        let range = selectedRange()
        let source = string as NSString
        guard range.location != NSNotFound, range.length > 0, range.location <= source.length,
              range.length <= source.length - range.location else { return [] }
        let selected = textStorage.map { NativeMathContent.plainText($0.attributedSubstring(from: range)) } ?? source.substring(with: range)
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
        return [("复制代码", NativeMarkdownContent.codeCopy), ("复制表格", NativeMarkdownContent.tableCopy), ("复制公式", NativeMathContent.formulaCopy)].compactMap { title, key in
            guard let text = storage.attribute(key, at: index, effectiveRange: nil) as? String else { return nil }
            let action = NativeSelectionAction { copy(text) }
            let item = NSMenuItem(title: title, action: #selector(NativeSelectionAction.invoke(_:)), keyEquivalent: "")
            item.target = action; item.representedObject = action
            return item
        }
    }
}
