import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeMathContentTests: XCTestCase {
    private let source = #"中文 $x_1 + \alpha$ and $\frac{a}{b}$."# + "\n\n" + #"$$\sum_{i=1}^{n} x_i = \frac{n(n+1)}{2}$$"#

    func testProtectsCodeEscapedDollarsCurrencyAndUnfinishedStreams() {
        for text in [#"`$x$`"#, "```latex\n$$x$$\n```", "~~~\n$x$\n~~~", "    $x$", #"cost \$20 and \$30"#, "$20 and $30", "$$\nx + y", #"\(x"#] {
            XCTAssertTrue(NativeMathContent.prepare(text).formulas.isEmpty, text)
        }
        let prepared = NativeMathContent.prepare(source + "\n" + #"\[a^2\] and \(b^2\)"#)
        XCTAssertEqual(prepared.formulas.count, 5)
        XCTAssertEqual(prepared.formulas.map(\.display), [false, false, true, true, false])
        XCTAssertEqual(prepared.formulas[0].latex, #"x_1 + \alpha"#)
        XCTAssertTrue(NativeMathContent.prepare("WISPMATHTOKEN $x$").markdown.contains("WISPMATHTOKENX0END"))
    }

    @MainActor func testNativeVectorsPreserveInlineBaselineAndExactCopyAcrossBlocks() throws {
        for scheme in [ColorScheme.light, .dark] {
            let rendered = NativeMarkdownContent.render(source, saved: [], scheme: scheme, width: 720)
            var cells: [NativeMathCell] = []
            rendered.enumerateAttribute(.attachment, in: NSRange(location: 0, length: rendered.length)) { attachment, _, _ in
                if let cell = (attachment as? NSTextAttachment)?.attachmentCell as? NativeMathCell { cells.append(cell) }
            }
            XCTAssertEqual(cells.count, 3)
            rendered.enumerateAttribute(NativeMarkdownContent.copyBlockID, in: NSRange(location: 0, length: rendered.length)) { block, range, _ in
                if (block as? String)?.hasPrefix("formula-") == true {
                    XCTAssertEqual((rendered.attribute(.paragraphStyle, at: range.location, effectiveRange: nil) as? NSParagraphStyle)?.alignment, .center)
                }
            }
            XCTAssertTrue(cells.allSatisfy { $0.cellSize().width > 5 && $0.cellSize().height > 5 })
            XCTAssertLessThan(cells[0].cellBaselineOffset().y, 0, "Subscripts extend below the baseline")
            let view = NativeMessageTextView(frame: NSRect(x: 0, y: 0, width: 720, height: 600))
            view.apply(rendered)
            view.setSelectedRange(NSRange(location: 0, length: rendered.length))
            var copied = ""; view.copyBlock = { copied = $0 }
            view.copy(nil)
            XCTAssertTrue(copied.contains(#"$x_1 + \alpha$"#))
            XCTAssertTrue(copied.contains(#"$$\sum_{i=1}^{n} x_i = \frac{n(n+1)}{2}$$"#))
            XCTAssertFalse(copied.contains("\u{FFFC}"))
            XCTAssertFalse(copied.contains("WISPMATHTOKEN"))
            XCTAssertEqual(view.accessibilityValue(), copied)
            var quoted = ""; view.quote = { quoted = $0 }
            (view.selectionActions().first?.representedObject as? NativeSelectionAction)?.invoke(nil)
            XCTAssertEqual(quoted, copied)
            view.layoutCopyButtons()
            let button = try XCTUnwrap(view.subviews.compactMap { $0 as? NSButton }.first { $0.toolTip == localized("复制公式") })
            let selection = view.selectedRange(); button.performClick(nil)
            XCTAssertEqual(copied, #"\sum_{i=1}^{n} x_i = \frac{n(n+1)}{2}"#)
            XCTAssertEqual(view.selectedRange(), selection)
        }
    }

    @MainActor func testInvalidAndOverflowingEquationsRemainReadableCopyableSource() {
        for source in [#"$$\unsupported{x}$$"#, #"$$\frac{a}$$"#, "$$" + String(repeating: "x + ", count: 100) + "z$$"] {
            let rendered = NativeMarkdownContent.render(source, saved: [], scheme: .light, width: 240)
            XCTAssertTrue(rendered.string.contains(source), "Expected literal fallback for \(source), got \(rendered.string)")
            XCTAssertFalse(rendered.string.contains("WISPMATHTOKEN"))
        }
        let label = NativeMathCell(latex: String(repeating: "{", count: 65) + "x" + String(repeating: "}", count: 65), display: true, size: 14, scheme: .light)
        XCTAssertNil(label)
    }

    @MainActor func testCodeHighlightingDoesNotChangeUnicodeOrCopySource() throws {
        let code = "# 中文\nvalue = 12\nprint(\"🧬 sample\")\n"
        let source = "```python\n" + code + "```"
        for scheme in [ColorScheme.light, .dark] {
            let rendered = NativeMarkdownContent.render(source, saved: [], scheme: scheme)
            XCTAssertEqual(rendered.string, code)
            let number = (rendered.string as NSString).range(of: "12").location
            let text = (rendered.string as NSString).range(of: "value").location
            XCTAssertNotEqual(rendered.attribute(.foregroundColor, at: number, effectiveRange: nil) as? NSColor, rendered.attribute(.foregroundColor, at: text, effectiveRange: nil) as? NSColor)
            XCTAssertEqual(rendered.attribute(NativeMarkdownContent.codeCopy, at: number, effectiveRange: nil) as? String, code)
        }
        XCTAssertEqual(NativeMarkdownContent.render("```unknown\n" + code + "```", saved: [], scheme: .light).string, code)
    }

    @MainActor func testReadOnlyTasksCopyMarkdownAndNeverTrackClicks() throws {
        let rendered = NativeMarkdownContent.render("- [x] 完成\n- [ ] Pending", saved: [], scheme: .light)
        XCTAssertEqual(NativeMathContent.plainText(rendered), "[x] 完成\n[ ] Pending\n")
        var count = 0
        rendered.enumerateAttribute(.attachment, in: NSRange(location: 0, length: rendered.length)) { value, _, _ in
            if let cell = (value as? NSTextAttachment)?.attachmentCell as? NativeTaskCell {
                count += 1; XCTAssertFalse(cell.wantsToTrackMouse())
            }
        }
        XCTAssertEqual(count, 2)
    }

    @MainActor func testSavedExcerptSpansFormulaAndTableCopyKeepsLatex() throws {
        let item = try JSONDecoder().decode(ConversationItem.self, from: JSONSerialization.data(withJSONObject: ["role": "assistant", "text": "# Title\n\nBefore $x_1$ after"]))
        XCTAssertEqual(NativeConversationModel.renderedText(item), "Title\nBefore $x_1$ after\n")
        let rendered = NativeMarkdownContent.render(#"Before $x_1$ after"#, saved: [#"Before $x_1$ after"#], scheme: .light)
        XCTAssertNotNil(rendered.attribute(.underlineStyle, at: 0, effectiveRange: nil))
        let formula = (rendered.string as NSString).range(of: "\u{FFFC}").location
        XCTAssertNotEqual(formula, NSNotFound)
        XCTAssertNotNil(rendered.attribute(.underlineStyle, at: formula, effectiveRange: nil))
        let table = NativeMarkdownContent.render("| Name | Formula |\n| --- | --- |\n| A | $x_1$ |", saved: [], scheme: .light)
        XCTAssertEqual(table.attribute(NativeMarkdownContent.tableCopy, at: 0, effectiveRange: nil) as? String, "Name\tFormula\nA\t$x_1$")
    }

    @MainActor func testRenderMathLightDarkNarrow() throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in rendering") }
        for (name, scheme, width) in [("math-light", ColorScheme.light, 720.0), ("math-dark", ColorScheme.dark, 720.0), ("math-narrow", ColorScheme.light, 300.0)] {
            let markdown = "# 公式与代码 / Math\n\n" + source + "\n\n- [x] 已校验\n- [ ] Follow up\n\n```python\n# 中文注释\nx = 12\nprint(\"hello\")\n```\n\n" + #"$$\unknown{x}$$"#
            let view = NSHostingView(rootView: NativeSelectableMessage(text: AttributedString(""), saved: [], quote: nil, save: nil, markdown: markdown)
                .padding(20).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .background(WispDesign.color("bg-app", scheme)).environment(\.colorScheme, scheme))
            view.frame = NSRect(x: 0, y: 0, width: width, height: 700); view.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
        }
    }
}
