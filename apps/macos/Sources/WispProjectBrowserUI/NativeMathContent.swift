import AppKit
import SwiftMath
import SwiftUI

/// Keep source on every replacement so copy, quote and VoiceOver never receive
/// an opaque attachment character. Equations are drawn as native vectors.
enum NativeMathContent {
    static let sourceKey = NSAttributedString.Key("WispInlineSource")
    static let formulaCopy = NSAttributedString.Key("WispFormulaCopy")
    struct Formula {
        let token: String
        let source: String
        let latex: String
        let display: Bool
    }
    struct Prepared {
        let markdown: String
        let formulas: [Formula]
    }

    static func prepare(_ source: String) -> Prepared {
        let text = source as NSString
        var protected: [NSRange] = []
        var offset = 0
        var fence: (Character, Int, Int)?
        for line in source.components(separatedBy: "\n") {
            let length = (line as NSString).length
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if let first = trimmed.first, first == "`" || first == "~", trimmed.prefix(while: { $0 == first }).count >= 3 {
                let count = trimmed.prefix(while: { $0 == first }).count
                if let open = fence {
                    if first == open.0 && count >= open.1 && trimmed.dropFirst(count).trimmingCharacters(in: .whitespaces).isEmpty {
                        protected.append(NSRange(location: open.2, length: offset + length - open.2)); fence = nil
                    }
                } else { fence = (first, count, offset) }
            } else if fence == nil && (line.hasPrefix("    ") || line.hasPrefix("\t")) {
                protected.append(NSRange(location: offset, length: length))
            }
            offset += length + 1
        }
        if let fence { protected.append(NSRange(location: fence.2, length: text.length - fence.2)) }
        if let code = try? NSRegularExpression(pattern: #"(`+)([\s\S]*?)\1"#) {
            protected += code.matches(in: source, range: NSRange(location: 0, length: text.length)).map(\.range)
        }
        // Image captions and paths are Markdown data. Dollar/bracket syntax
        // in filenames must not become equation tokens before image parsing.
        for pattern in [#"!\[(?:\\.|[^\]])*\](?:\((?:<[^>\n]*>|\\.|[^)\n])*\)|\[[^\]\n]*\])"#, #"(?m)^[ \t]{0,3}\[[^\]\n]+\]:[^\n]+"#] {
            if let image = try? NSRegularExpression(pattern: pattern) {
                protected += image.matches(in: source, range: NSRange(location: 0, length: text.length)).map(\.range)
            }
        }
        let pattern = #"(?s)(?<!\\)\$\$(.+?)(?<!\\)\$\$|(?<!\\)\\\[(.+?)\\\]|(?<!\\)\\\((.+?)\\\)|(?<![\\$])\$(?![\s$])([^\n]*?\S)(?<!\\)\$(?![\d$])"#
        guard let expression = try? NSRegularExpression(pattern: pattern) else { return .init(markdown: source, formulas: []) }
        var formulas: [Formula] = []
        var replacements: [(NSRange, String)] = []
        // Stable but collision-free token namespace. No LaTeX is fed through
        // Foundation's Markdown parser (which otherwise consumes backslashes).
        var prefix = "WISPMATHTOKEN"
        while source.contains(prefix) { prefix += "X" }
        for match in expression.matches(in: source, range: NSRange(location: 0, length: text.length)) {
            guard !protected.contains(where: { NSIntersectionRange($0, match.range).length > 0 }) else { continue }
            guard let group = (1...4).first(where: { match.range(at: $0).location != NSNotFound }) else { continue }
            let token = prefix + String(formulas.count) + "END"
            let display = group <= 2
            formulas.append(.init(token: token, source: text.substring(with: match.range), latex: text.substring(with: match.range(at: group)), display: display))
            replacements.append((match.range, display ? "\n\n" + token + "\n\n" : token))
        }
        let output = NSMutableString(string: source)
        for (range, value) in replacements.reversed() { output.replaceCharacters(in: range, with: value) }
        return .init(markdown: output as String, formulas: formulas)
    }

    static func replace(in value: NSMutableAttributedString, formulas: [Formula], scheme: ColorScheme, width: CGFloat) {
        for formula in formulas {
            let range = (value.string as NSString).range(of: formula.token)
            guard range.location != NSNotFound else { continue }
            let attributes = value.attributes(at: range.location, effectiveRange: nil)
            let size = (attributes[.font] as? NSFont)?.pointSize ?? 14
            let replacement: NSMutableAttributedString
            if let cell = NativeMathCell(latex: formula.latex, display: formula.display, size: size, scheme: scheme), cell.cellSize().width <= max(100, width - 32) {
                let attachment = NSTextAttachment(); attachment.attachmentCell = cell
                replacement = NSMutableAttributedString(attributedString: NSAttributedString(attachment: attachment))
                replacement.addAttributes(attributes, range: NSRange(location: 0, length: replacement.length))
                replacement.addAttribute(sourceKey, value: formula.source, range: NSRange(location: 0, length: replacement.length))
            } else {
                // Unsupported syntax or a formula wider than the column remains
                // readable, wrapped source. A wider artifact preview can typeset it.
                replacement = NSMutableAttributedString(string: formula.source, attributes: attributes)
                replacement.addAttribute(.font, value: NSFont.monospacedSystemFont(ofSize: size, weight: .regular), range: NSRange(location: 0, length: replacement.length))
            }
            replacement.addAttribute(formulaCopy, value: formula.latex, range: NSRange(location: 0, length: replacement.length))
            if formula.display {
                replacement.addAttribute(NativeMarkdownContent.copyBlockID, value: "formula-" + formula.token, range: NSRange(location: 0, length: replacement.length))
                let paragraph = (attributes[.paragraphStyle] as? NSParagraphStyle)?.mutableCopy() as? NSMutableParagraphStyle ?? NSMutableParagraphStyle()
                paragraph.paragraphSpacingBefore = 34; paragraph.paragraphSpacing = 18
                paragraph.alignment = replacement.attribute(.attachment, at: 0, effectiveRange: nil) == nil ? .left : .center
                replacement.addAttribute(.paragraphStyle, value: paragraph, range: NSRange(location: 0, length: replacement.length))
            }
            value.replaceCharacters(in: range, with: replacement)
        }
    }

    static func plainText(_ value: NSAttributedString) -> String {
        let result = NSMutableString(string: value.string)
        var replacements: [(NSRange, String)] = []
        value.enumerateAttribute(sourceKey, in: NSRange(location: 0, length: value.length)) { source, range, _ in
            if let source = source as? String { replacements.append((range, source)) }
        }
        for (range, source) in replacements.reversed() { result.replaceCharacters(in: range, with: source) }
        return result as String
    }

    static func renderedRange(_ range: NSRange, in value: NSAttributedString) -> NSRange {
        var plainOffset = 0
        var matches: [NSRange] = []
        value.enumerateAttribute(sourceKey, in: NSRange(location: 0, length: value.length)) { source, run, _ in
            let length = (source as? String).map { ($0 as NSString).length } ?? run.length
            let intersection = NSIntersectionRange(range, NSRange(location: plainOffset, length: length))
            if intersection.length > 0 {
                matches.append(source is String ? run : NSRange(location: run.location + intersection.location - plainOffset, length: intersection.length))
            }
            plainOffset += length
        }
        guard let first = matches.first, let last = matches.last else { return NSRange(location: 0, length: 0) }
        return NSRange(location: first.location, length: NSMaxRange(last) - first.location)
    }
}

final class NativeMathCell: NSTextAttachmentCell {
    private let label: MTMathUILabel
    private let size: NSSize
    private let descent: CGFloat

    init?(latex: String, display: Bool, size pointSize: CGFloat, scheme: ColorScheme) {
        // Bound pathological inputs before the recursive TeX parser. Invalid or
        // unsupported input is displayed literally rather than swallowed.
        guard latex.utf16.count <= 8_000 else { return nil }
        var depth = 0
        for char in latex {
            if char == "{" { depth += 1 }
            if char == "}" { depth -= 1 }
            if depth > 64 { return nil }
        }
        label = MTMathUILabel(frame: .zero)
        label.fontSize = display ? pointSize * 1.2 : pointSize
        label.labelMode = display ? .display : .text
        label.textColor = NSColor(WispDesign.color("text", scheme))
        label.latex = latex
        guard label.error == nil, let list = label.mathList, Self.complete(list) else { return nil }
        size = label.fittingSize
        guard size.width.isFinite, size.height.isFinite, size.width > 0, size.height > 0 else { return nil }
        label.frame = NSRect(origin: .zero, size: size); label.layout()
        descent = label.displayList?.descent ?? 0
        super.init(textCell: "")
        setAccessibilityLabel(latex)
    }
    required init(coder: NSCoder) { fatalError("Not archived") }
    private static func complete(_ list: MTMathList) -> Bool {
        list.atoms.allSatisfy { atom in
            if let fraction = atom as? MTFraction {
                guard let numerator = fraction.numerator, !numerator.atoms.isEmpty,
                      let denominator = fraction.denominator, !denominator.atoms.isEmpty,
                      complete(numerator), complete(denominator) else { return false }
            }
            if let radical = atom as? MTRadical, let inner = radical.radicand, !complete(inner) { return false }
            if let inner = (atom as? MTInner)?.innerList, !complete(inner) { return false }
            if let sub = atom.subScript, !complete(sub) { return false }
            if let sup = atom.superScript, !complete(sup) { return false }
            return true
        }
    }
    override func cellSize() -> NSSize { size }
    override func cellBaselineOffset() -> NSPoint { NSPoint(x: 0, y: -descent) }
    override func draw(withFrame frame: NSRect, in controlView: NSView?) {
        guard let context = NSGraphicsContext.current?.cgContext else { return }
        context.saveGState()
        // TextKit leaves a flipped text matrix in its drawing context. CoreText
        // expects an identity text matrix after we establish math coordinates.
        context.textMatrix = .identity
        context.translateBy(x: frame.minX, y: controlView?.isFlipped == true ? frame.maxY : frame.minY)
        if controlView?.isFlipped == true { context.scaleBy(x: 1, y: -1) }
        label.displayList?.draw(context)
        context.restoreGState()
    }
    override func wantsToTrackMouse() -> Bool { false }
}

final class NativeTaskCell: NSTextAttachmentCell {
    private let checkbox = NSButtonCell(textCell: "")
    init(checked: Bool) {
        super.init(textCell: "")
        checkbox.setButtonType(.switch); checkbox.state = checked ? .on : .off
        checkbox.isEnabled = false
        setAccessibilityLabel(localized(checked ? "已完成" : "待完成"))
    }
    required init(coder: NSCoder) { fatalError("Not archived") }
    override func cellSize() -> NSSize { NSSize(width: 16, height: 16) }
    override func cellBaselineOffset() -> NSPoint { NSPoint(x: 0, y: -3) }
    override func draw(withFrame frame: NSRect, in controlView: NSView?) {
        if let controlView { checkbox.draw(withFrame: frame, in: controlView) }
    }
    override func wantsToTrackMouse() -> Bool { false }
}
