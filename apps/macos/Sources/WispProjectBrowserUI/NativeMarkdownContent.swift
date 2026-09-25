import AppKit
import SwiftUI
import WispProjectBrowser

/// Foundation parses CommonMark/GFM; AppKit keeps the result in one selectable
/// document, including table cells and code, so quote/save can span blocks.
enum NativeMarkdownContent {
    static let codeCopy = NSAttributedString.Key("WispCodeCopy")
    static let tableCopy = NSAttributedString.Key("WispTableCopy")

    private struct Block {
        var text: AttributedString
        let intents: [PresentationIntent.IntentType]
        var identity: Int? { intents.first?.identity }
        var tableID: Int? { intents.first { if case .table = $0.kind { return true }; return false }?.identity }
        var tableRow: Int { intents.compactMap { if case .tableRow(let row) = $0.kind { return row }; return nil }.first ?? 0 }
    }

    static func render(_ source: String, saved: [String], revealed: String? = nil, scheme: ColorScheme) -> NSAttributedString {
        guard let parsed = try? AttributedString(markdown: source, options: .init(interpretedSyntax: .full)) else {
            return NativeSelectableMessage.content(AttributedString(source), saved: saved, scheme: scheme)
        }
        var blocks: [Block] = []
        for run in parsed.runs {
            let intents = run.presentationIntent?.components ?? []
            let text = AttributedString(parsed[run.range])
            if let last = blocks.last, last.identity == intents.first?.identity {
                blocks[blocks.count - 1].text.append(text)
            } else {
                blocks.append(Block(text: text, intents: intents))
            }
        }
        var tableText: [Int: String] = [:]
        var lastRows: [Int: Int] = [:]
        for block in blocks {
            guard let id = block.tableID else { continue }
            let separator = lastRows[id] == nil ? "" : lastRows[id] == block.tableRow ? "\t" : "\n"
            tableText[id, default: ""] += separator + String(block.text.characters)
            lastRows[id] = block.tableRow
        }
        let result = NSMutableAttributedString(string: "")
        var tables: [Int: NSTextTable] = [:]
        var seenListItems: Set<Int> = []
        for block in blocks {
            let code = block.intents.contains { if case .codeBlock = $0.kind { return true }; return false }
            let value = NSMutableAttributedString(attributedString: NativeSelectableMessage.content(block.text, saved: [], scheme: scheme, monospaced: code))
            let paragraph = NSMutableParagraphStyle()
            paragraph.lineSpacing = 3
            paragraph.paragraphSpacing = 9
            paragraph.lineBreakMode = .byWordWrapping
            var indent = 0
            var listItem: PresentationIntent.IntentType?
            var ordered = false
            var header = false
            for component in block.intents {
                switch component.kind {
                case .header(let level):
                    let base = (value.attribute(.font, at: 0, effectiveRange: nil) as? NSFont)?.pointSize ?? 14
                    let size = base * [1.65, 1.4, 1.2, 1.1, 1, 1][min(5, max(0, level - 1))]
                    value.addAttribute(.font, value: NSFont.systemFont(ofSize: size, weight: .semibold), range: NSRange(location: 0, length: value.length))
                    paragraph.paragraphSpacingBefore = 10
                case .listItem:
                    if listItem == nil { listItem = component }
                case .orderedList, .unorderedList:
                    if indent == 0 { ordered = component.kind == .orderedList }
                    indent += 1
                case .blockQuote:
                    paragraph.headIndent += 16; paragraph.firstLineHeadIndent += 16
                    value.addAttribute(.foregroundColor, value: NSColor(WispDesign.color("text-muted", scheme)), range: NSRange(location: 0, length: value.length))
                case .tableHeaderRow: header = true
                default: break
                }
            }
            if let item = listItem, case .listItem(let ordinal) = item.kind {
                if seenListItems.insert(item.identity).inserted {
                    var prefix = ordered ? "\(ordinal). " : "• "
                    // Task status is readable and copyable, including by VoiceOver.
                    for (marker, label) in [("[x] ", "[已完成] "), ("[X] ", "[已完成] "), ("[ ] ", "[待完成] ")] where value.string.hasPrefix(marker) {
                        value.deleteCharacters(in: NSRange(location: 0, length: marker.count))
                        prefix = label
                        break
                    }
                    let attrs = value.length > 0 ? value.attributes(at: 0, effectiveRange: nil) : [:]
                    value.insert(NSAttributedString(string: prefix, attributes: attrs), at: 0)
                }
                paragraph.firstLineHeadIndent += CGFloat(max(0, indent - 1)) * 20
                paragraph.headIndent += CGFloat(indent) * 20
                paragraph.paragraphSpacing = 4
            }
            if code {
                paragraph.paragraphSpacing = 0
                value.addAttributes([codeCopy: String(block.text.characters), .backgroundColor: NSColor(WispDesign.color("bg-sunken", scheme))], range: NSRange(location: 0, length: value.length))
                paragraph.firstLineHeadIndent += 10; paragraph.headIndent += 10
            }
            if let id = block.tableID,
               let columns = block.intents.compactMap({ if case .table(let columns) = $0.kind { return columns }; return nil }).first,
               let column = block.intents.compactMap({ if case .tableCell(let column) = $0.kind { return column }; return nil }).first {
                let table = tables[id] ?? NSTextTable()
                table.numberOfColumns = columns.count
                table.layoutAlgorithm = .fixedLayoutAlgorithm
                table.collapsesBorders = true
                table.setValue(100, type: .percentageValueType, for: .width)
                tables[id] = table
                let cell = NSTextTableBlock(table: table, startingRow: block.tableRow, rowSpan: 1, startingColumn: column, columnSpan: 1)
                cell.setValue(100 / CGFloat(max(1, columns.count)), type: .percentageValueType, for: .width)
                cell.setWidth(7, type: .absoluteValueType, for: .padding)
                cell.setWidth(0.5, type: .absoluteValueType, for: .border)
                cell.setBorderColor(NSColor(WispDesign.color("border", scheme)))
                if header { cell.backgroundColor = NSColor(WispDesign.color("bg-sunken", scheme)) }
                paragraph.textBlocks = [cell]
                paragraph.paragraphSpacing = 0
                if columns.indices.contains(column) {
                    switch columns[column].alignment {
                    case .left: paragraph.alignment = .left
                    case .center: paragraph.alignment = .center
                    case .right: paragraph.alignment = .right
                    @unknown default: paragraph.alignment = .left
                    }
                }
                if header, value.length > 0, let font = value.attribute(.font, at: 0, effectiveRange: nil) as? NSFont {
                    value.addAttribute(.font, value: NSFontManager.shared.convert(font, toHaveTrait: .boldFontMask), range: NSRange(location: 0, length: value.length))
                }
                value.addAttribute(tableCopy, value: tableText[id] ?? "", range: NSRange(location: 0, length: value.length))
            }
            // A paragraph separator is essential: full Markdown parsing strips
            // block delimiters. Preserve fenced-code line breaks exactly.
            if !value.string.hasSuffix("\n") {
                let attributes = value.length > 0 ? value.attributes(at: value.length - 1, effectiveRange: nil) : [:]
                value.append(NSAttributedString(string: "\n", attributes: attributes))
            }
            value.addAttribute(.paragraphStyle, value: paragraph, range: NSRange(location: 0, length: value.length))
            result.append(value)
        }
        let plain = result.string
        func range(_ match: Range<Int>) -> NSRange {
            let start = plain.index(plain.startIndex, offsetBy: match.lowerBound)
            let end = plain.index(plain.startIndex, offsetBy: match.upperBound)
            return NSRange(start..<end, in: plain)
        }
        for excerpt in Set(saved) {
            for match in NativeSavedExcerpt.ranges(in: plain, excerpt: excerpt) {
                result.addAttributes([.underlineStyle: NSUnderlineStyle.single.rawValue, .underlineColor: NSColor(WispDesign.color("clay", scheme))], range: range(match))
            }
        }
        if let revealed, let match = NativeSavedExcerpt.range(in: plain, excerpt: revealed) {
            result.addAttribute(.backgroundColor, value: NSColor.systemYellow.withAlphaComponent(0.4), range: range(match))
        }
        return result
    }
}
