import AppKit
import SwiftUI
import XCTest
@testable import WispProjectBrowserUI

final class NativeMarkdownTests: XCTestCase {
    private let source = """
    # Quality report

    Sample **A** passed. `inline()` remains code.

    > Keep the original controls.

    - Parent
      - Nested
    - [x] Checked
    - [ ] Pending

    3. Third
    4. Fourth

    | Sample | Reads |
    | :--- | ---: |
    | A | 120 |
    | B | 240 |

    ```python
    first = 1
    second = first + 2
    print(second)
    ```
    """
    @MainActor func testBlocksKeepStructureCodeNewlinesAndInlineTraits() throws {
        let value = NativeMarkdownContent.render(source, saved: [], scheme: .light)
        XCTAssertTrue(value.string.hasPrefix("Quality report\nSample A passed."))
        XCTAssertTrue(value.string.contains("first = 1\nsecond = first + 2\nprint(second)\n"))
        XCTAssertTrue(value.string.contains("3. Third\n4. Fourth"))
        XCTAssertTrue(value.string.contains("[已完成] Checked"))
        XCTAssertTrue(value.string.contains("[待完成] Pending"))
        func font(_ text: String) throws -> NSFont {
            try XCTUnwrap(value.attribute(.font, at: (value.string as NSString).range(of: text).location, effectiveRange: nil) as? NSFont)
        }
        XCTAssertGreaterThan(try font("Quality report").pointSize, try font("Sample A").pointSize)
        XCTAssertTrue(try font("first =").isFixedPitch)
        XCTAssertTrue(try font("inline()").isFixedPitch)
        let bold = (value.string as NSString).range(of: "A passed").location
        XCTAssertTrue(NSFontManager.shared.traits(of: try XCTUnwrap(value.attribute(.font, at: bold, effectiveRange: nil) as? NSFont)).contains(.boldFontMask))
        let quote = try XCTUnwrap(value.attribute(.paragraphStyle, at: (value.string as NSString).range(of: "Keep").location, effectiveRange: nil) as? NSParagraphStyle)
        XCTAssertEqual(quote.textBlocks.first?.width(for: .border, edge: .minX), 3)
        let nested = try XCTUnwrap(value.attribute(.paragraphStyle, at: (value.string as NSString).range(of: "Nested").location, effectiveRange: nil) as? NSParagraphStyle)
        XCTAssertEqual(nested.headIndent, 40)
    }
    @MainActor func testTableIsARealAlignedTableAndCopyCapturesOriginalBlock() throws {
        let view = NativeMessageTextView(frame: .zero)
        view.apply(NativeMarkdownContent.render(source, saved: [], scheme: .light))
        let code = (view.string as NSString).range(of: "first = 1").location
        let cell = (view.string as NSString).range(of: "120").location
        let style = try XCTUnwrap(view.textStorage?.attribute(.paragraphStyle, at: cell, effectiveRange: nil) as? NSParagraphStyle)
        XCTAssertEqual(style.alignment, .right)
        let block = try XCTUnwrap(style.textBlocks.first as? NSTextTableBlock)
        XCTAssertEqual(block.startingRow, 1); XCTAssertEqual(block.startingColumn, 1)
        XCTAssertEqual(block.table.numberOfColumns, 2)
        var copied: [String] = []
        let codeMenu = view.blockActions(at: code, copy: { copied.append($0) })
        let tableMenu = view.blockActions(at: cell, copy: { copied.append($0) })
        XCTAssertEqual(codeMenu.map(\.title), ["复制代码"])
        XCTAssertEqual(tableMenu.map(\.title), ["复制表格"])
        view.apply(NSAttributedString(string: "later stream"))
        for item in codeMenu + tableMenu { (item.representedObject as? NativeSelectionAction)?.invoke(nil) }
        XCTAssertEqual(copied, ["first = 1\nsecond = first + 2\nprint(second)\n", "Sample\tReads\nA\t120\nB\t240"])
    }
    @MainActor func testVisibleCopyButtonsPreserveSelectionAndHaveDistinctBlockActions() throws {
        let view = NativeMessageTextView(frame: NSRect(x: 0, y: 0, width: 600, height: 1000))
        view.textContainer?.containerSize = NSSize(width: 600, height: CGFloat.greatestFiniteMagnitude)
        view.apply(NativeMarkdownContent.render(source, saved: [], scheme: .light))
        view.setSelectedRange(NSRange(location: 0, length: 14))
        var copied: [String] = []
        view.copyBlock = { copied.append($0) }
        view.layoutCopyButtons()
        let buttons = view.subviews.compactMap { $0 as? NSButton }
        XCTAssertEqual(Set(buttons.compactMap(\.toolTip)), ["复制代码", "复制表格"])
        XCTAssertEqual(buttons.count, 2)
        XCTAssertEqual(view.accessibilityChildren()?.compactMap { $0 as? NSButton }.count, 2)
        buttons.first { $0.toolTip == "复制代码" }?.performClick(nil)
        buttons.first { $0.toolTip == "复制表格" }?.performClick(nil)
        XCTAssertEqual(copied, ["first = 1\nsecond = first + 2\nprint(second)\n", "Sample\tReads\nA\t120\nB\t240"])
        XCTAssertEqual(view.selectedRange(), NSRange(location: 0, length: 14))
        view.layoutCopyButtons()
        XCTAssertEqual(view.subviews.compactMap { $0 as? NSButton }.count, 2)
        for button in buttons { XCTAssertTrue(view.bounds.contains(button.frame)) }
        view.apply(NSAttributedString(string: "plain reply"))
        view.layoutCopyButtons()
        XCTAssertTrue(view.subviews.compactMap { $0 as? NSButton }.isEmpty)
    }
    @MainActor func testIdenticalAdjacentCodeBlocksHaveIndependentStableCopyButtons() throws {
        let view = NativeMessageTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 1000))
        let source = "```\nx = 1\n```\n\n```\nx = 1\n```"
        view.apply(NativeMarkdownContent.render(source, saved: [], scheme: .light))
        view.layoutCopyButtons()
        let first = view.subviews.compactMap { $0 as? NSButton }
        XCTAssertEqual(first.count, 2)
        view.apply(NativeMarkdownContent.render(source, saved: ["x"], scheme: .light))
        view.layoutCopyButtons()
        let second = view.subviews.compactMap { $0 as? NSButton }
        XCTAssertEqual(second.count, 2)
        XCTAssertTrue(zip(first, second).allSatisfy { $0 === $1 })
    }
    @MainActor func testEmptyTableCellsKeepOneCopyActionAcrossLayoutUpdates() {
        let view = NativeMessageTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 1000))
        let source = "| A | B |\n| --- | --- |\n| | value |\n| tail | |"
        view.apply(NativeMarkdownContent.render(source, saved: [], scheme: .light))
        view.layoutCopyButtons()
        XCTAssertEqual(view.subviews.compactMap { $0 as? NSButton }.count, 1)
        view.frame.size.width = 280
        view.layoutCopyButtons()
        XCTAssertEqual(view.subviews.compactMap { $0 as? NSButton }.count, 1)
    }
    @MainActor func testMarksAndSelectionsCanSpanBlocksAndUnicode() throws {
        let view = NativeMessageTextView(frame: .zero)
        let content = NativeMarkdownContent.render("# 🧬 Title\n\n**Sample** body", saved: ["Title Sample"], revealed: "Sample", scheme: .dark)
        view.apply(content)
        let range = (view.string as NSString).range(of: "Title\nSample")
        XCTAssertNotEqual(range.location, NSNotFound)
        view.setSelectedRange(range)
        var quoted = ""
        view.quote = { quoted = $0 }
        let action = try XCTUnwrap(view.selectionActions().first?.representedObject as? NativeSelectionAction)
        action.invoke(nil)
        XCTAssertEqual(quoted, "Title\nSample")
        XCTAssertNotNil(content.attribute(.underlineStyle, at: range.location, effectiveRange: nil))
        XCTAssertNotNil(content.attribute(.backgroundColor, at: (view.string as NSString).range(of: "Sample").location, effectiveRange: nil))
    }
    @MainActor func testNarrowLayoutContainsLongCodeAndTables() throws {
        let value = NativeMarkdownContent.render(source + "\n\n```\n" + String(repeating: "x", count: 300) + "\n```", saved: [], scheme: .light)
        let storage = NSTextStorage(attributedString: value)
        let layout = NSLayoutManager(); storage.addLayoutManager(layout)
        let container = NSTextContainer(containerSize: NSSize(width: 260, height: CGFloat.greatestFiniteMagnitude))
        container.lineFragmentPadding = 0
        layout.addTextContainer(container); layout.ensureLayout(for: container)
        let bounds = layout.usedRect(for: container)
        XCTAssertLessThanOrEqual(bounds.width, 261)
        XCTAssertGreaterThan(bounds.height, 300)
    }
    @MainActor func testIncompleteStreamsAndEmptyCellsRemainRenderable() {
        for text in ["", "#", "---", "```python\n", "| A | B |\n| --- | --- |\n| | value |", "**unfinished"] {
            _ = NativeMarkdownContent.render(text, saved: [], scheme: .light)
        }
        for end in stride(from: 1, to: source.count, by: 19) {
            _ = NativeMarkdownContent.render(String(source.prefix(end)), saved: [], scheme: .dark)
        }
    }
    @MainActor func testRenderStructuredMarkdown() throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in rendering") }
        for (name, scheme, width) in [("markdown-light", ColorScheme.light, 720.0), ("markdown-dark-narrow", ColorScheme.dark, 310.0)] {
            let root = NativeSelectableMessage(text: AttributedString(""), saved: ["Sample A"], quote: { _ in }, save: { _ in }, markdown: source)
                .padding(20).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .background(WispDesign.color("bg-app", scheme)).environment(\.colorScheme, scheme)
            let view = NSHostingView(rootView: root)
            view.frame = NSRect(x: 0, y: 0, width: width, height: 860)
            view.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
        }
    }
}
