import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func textPreview(_ path: String, _ text: String, truncated: Bool = false, mime: String = "text/plain") throws -> NativePanelFileContent {
    let value: SettingsValue = .object(["path": .string(path), "mime": .string(mime), "text": .string(text), "base64": .null, "truncated": .bool(truncated), "total_bytes": .integer(Int64(text.utf8.count))])
    return try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value))
}
@MainActor func imagePreview(_ size: Int = 24) throws -> NativePanelFileContent {
    let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
    let pixels = try XCTUnwrap(bitmap.bitmapData)
    for index in stride(from: 0, to: bitmap.bytesPerRow * size, by: 4) {
        pixels[index] = 24; pixels[index + 1] = 150; pixels[index + 2] = 160; pixels[index + 3] = 255
    }
    let bytes = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    let value: SettingsValue = .object(["path": .string("plot.png"), "mime": .string("image/png"), "text": .null, "base64": .string(bytes.base64EncodedString()), "truncated": .bool(false)])
    return try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value))
}
private actor PreviewImageGate {
    var held: CheckedContinuation<NativePanelFileContent, Error>?
    func load() async throws -> NativePanelFileContent { try await withCheckedThrowingContinuation { held = $0 } }
    func waiting() -> Bool { held != nil }
    func release(_ value: NativePanelFileContent) { held?.resume(returning: value); held = nil }
}

final class NativeFileTextPreviewTests: XCTestCase {
    @MainActor private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    @MainActor func testActualWindowSourcePickerSwitchesWithoutChangingFileText() throws {
        let source = "---\ntitle: QC\n---\n# Report\n\n**Sample A** passed.\n\n| Gene | Value |\n| --- | --- |\n| A | 2 |"
        let content = try textPreview("/work/report.md", source)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 650), styleMask: [.titled], backing: .buffered, defer: false)
        let host = NSHostingView(rootView: NativePanelFilePreview(content: content, close: {}, save: { _ in }))
        window.isReleasedWhenClosed = false; window.contentView = host; window.makeKeyAndOrderFront(nil)
        defer { window.orderOut(nil); window.contentView = nil }
        RunLoop.current.run(until: Date().addingTimeInterval(0.08)); host.layoutSubtreeIfNeeded()
        let picker = try XCTUnwrap(descendants(host).compactMap { $0 as? NSSegmentedControl }.first)
        XCTAssertEqual(picker.segmentCount, 2)
        XCTAssertTrue(descendants(host).compactMap { $0 as? NativeMessageTextView }.contains { $0.string.contains("Sample A passed.") && !$0.string.contains("title: QC") })
        let rendered = try XCTUnwrap(descendants(host).compactMap { $0 as? NativeMessageTextView }.first)
        XCTAssertGreaterThanOrEqual(rendered.bounds.width, 868)
        let text = rendered.string as NSString
        let gene = text.range(of: "Gene"), value = text.range(of: "Value")
        let style = try XCTUnwrap(rendered.textStorage?.attribute(.paragraphStyle, at: value.location, effectiveRange: nil) as? NSParagraphStyle)
        XCTAssertEqual(style.textBlocks.count, 1)
        let layout = try XCTUnwrap(rendered.layoutManager), container = try XCTUnwrap(rendered.textContainer)
        let first = layout.boundingRect(forGlyphRange: layout.glyphRange(forCharacterRange: gene, actualCharacterRange: nil), in: container)
        let second = layout.boundingRect(forGlyphRange: layout.glyphRange(forCharacterRange: value, actualCharacterRange: nil), in: container)
        XCTAssertEqual(first.minY, second.minY, accuracy: 2); XCTAssertGreaterThan(second.minX, first.maxX)
        picker.selectedSegment = 1; picker.sendAction(picker.action, to: picker.target)
        RunLoop.current.run(until: Date().addingTimeInterval(0.05)); host.layoutSubtreeIfNeeded()
        XCTAssertTrue(descendants(host).compactMap { $0 as? NativeMessageTextView }.contains { $0.string == source })
        picker.selectedSegment = 0; picker.sendAction(picker.action, to: picker.target)
        RunLoop.current.run(until: Date().addingTimeInterval(0.05)); host.layoutSubtreeIfNeeded()
        XCTAssertTrue(descendants(host).compactMap { $0 as? NativeMessageTextView }.contains { $0.string.contains("Sample A passed.") && !$0.string.contains("title: QC") })
        XCTAssertEqual(content.text, source)
    }
    @MainActor func testActualTableWindowBoundsRowsColumnsAndScrollsWithoutInterpretingCells() throws {
        let content = try textPreview("/work/counts.tsv", "gene\tvalue\tnote\n" + (0..<510).map { "G\($0)\t\($0)\t<script>" }.joined(separator: "\n"))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 320, height: 650), styleMask: [.titled], backing: .buffered, defer: false)
        let host = NSHostingView(rootView: NativePanelFilePreview(content: content, close: {}, quote: { _ in }))
        window.contentView = host; host.layoutSubtreeIfNeeded()
        let scroll = try XCTUnwrap(descendants(host).compactMap { $0 as? NSScrollView }.first { $0.documentView is NativeDelimitedTableControl })
        let table = try XCTUnwrap(scroll.documentView as? NativeDelimitedTableControl)
        XCTAssertEqual(table.numberOfRows, 500); XCTAssertEqual(table.tableColumns.count, 3)
        XCTAssertTrue(scroll.hasHorizontalScroller); XCTAssertTrue(scroll.hasVerticalScroller)
        XCTAssertTrue(host.bounds.contains(scroll.convert(scroll.bounds, to: host)))
        let cell = try XCTUnwrap(table.view(atColumn: 2, row: 0, makeIfNecessary: true) as? NativeDelimitedCellView)
        XCTAssertEqual(cell.content.string, "<script>")
    }
    func testDocumentKindsFollowFileExtensionsAndTextMimes() throws {
        for ext in ["md", "MD", "rmd", "qmd", "markdown"] { XCTAssertEqual(NativeFileTextDocument(try textPreview("ssh://lab/reports/notes." + ext, "# Report")).kind, .markdown) }
        for ext in ["csv", "CSV", "tsv"] { XCTAssertEqual(NativeFileTextDocument(try textPreview("/work/data." + ext, "gene,value\nA,2")).kind, .delimited) }
        XCTAssertEqual(NativeFileTextDocument(try textPreview("no-extension", "# Report", mime: "text/markdown")).kind, .markdown)
        XCTAssertEqual(NativeFileTextDocument(try textPreview("script.py", "# comment")).kind, .text)
    }
    func testFrontMatterIsHiddenOnlyForCompleteLeadingYaml() {
        XCTAssertEqual(NativeFileTextDocument.withoutFrontMatter("---\ntitle: QC\n---\n\n# Result"), "# Result")
        XCTAssertEqual(NativeFileTextDocument.withoutFrontMatter("---\r\ntitle: QC\r\n...\r\n\r\n# Result"), "# Result")
        for source in ["---\nA paragraph\n---\nBody", "---\ntitle: unfinished", "text\n---\ntitle: QC\n---"] { XCTAssertEqual(NativeFileTextDocument.withoutFrontMatter(source), source) }
    }
    func testReadingBoundsKeepUnicodeAndOriginalEditingSourceIntact() throws {
        let raw = String(repeating: "🧬", count: 300_000)
        let content = try textPreview("large.md", raw), document = NativeFileTextDocument(content)
        XCTAssertTrue(document.clipped); XCTAssertLessThanOrEqual(document.source.utf8.count, 1024 * 1024)
        XCTAssertFalse(document.source.contains("�")); XCTAssertEqual(content.text, raw)
        let many = NativeFileTextDocument(try textPreview("many.md", String(repeating: "x\n", count: 8_001)))
        XCTAssertTrue(many.clipped); XCTAssertEqual(many.source, String(repeating: "x\n", count: 8_000))
    }
    func testCsvParsesQuotedCommasEscapedQuotesMultilineUnicodeAndRaggedRows() throws {
        let source = "\u{feff}sample,value,note\r\n\"样本,A\",2,\"line 1\r\nline \"\"2\"\"\"\r\nB,,\r\nC,3\r\n\r\n"
        let table = try NativeDelimitedTable.parse(source, separator: 44)
        XCTAssertEqual(table.headers, ["sample", "value", "note"])
        XCTAssertEqual(table.rows, [["样本,A", "2", "line 1\nline \"2\""], ["B", "", ""], ["C", "3"]])
        XCTAssertEqual(table.copyText, "sample\tvalue\tnote\n样本,A\t2\tline 1 line \"2\"\nB\t\t\nC\t3")
        XCTAssertEqual(table.columnCount, 3)
    }
    func testTsvRetainsEmptyFieldsAndUsesTabsRatherThanCommas() throws {
        let document = NativeFileTextDocument(try textPreview("counts.tsv", "gene\tvalue\tnote\nA,B\t2\t\"x\ty\"\nC\t\t"))
        let table = try XCTUnwrap(document.table)
        XCTAssertEqual(table.rows, [["A,B", "2", "x\ty"], ["C", "", ""]])
        XCTAssertEqual(table.copyText, "gene\tvalue\tnote\nA,B\t2\tx y\nC\t\t")
    }
    func testEmptyHeaderOnlyAndMalformedTablesHaveExplicitResults() throws {
        XCTAssertEqual(try NativeDelimitedTable.parse(" \n\n", separator: 44).columnCount, 0)
        XCTAssertEqual(try NativeDelimitedTable.parse("a,b", separator: 44).headers, ["a", "b"])
        XCTAssertThrowsError(try NativeDelimitedTable.parse("a,b\nx,\"unfinished", separator: 44))
        XCTAssertThrowsError(try NativeDelimitedTable.parse("a,b\nx,\"closed\"bad", separator: 44))
        let document = NativeFileTextDocument(try textPreview("bad.csv", "a,b\nx,\"unfinished"))
        XCTAssertNil(document.table); XCTAssertNotNil(document.tableError); XCTAssertEqual(document.source, "a,b\nx,\"unfinished")
    }
    func testTruncatedTablesOmitPartialRecordsWithoutInventingCompleteRows() throws {
        for tail in ["C,", "C,\"unfinished", "C,3"] {
            let table = try NativeDelimitedTable.parse("a,b\nA,1\nB,2\n" + tail, separator: 44, truncated: true)
            XCTAssertEqual(table.rows, [["A", "1"], ["B", "2"]]); XCTAssertTrue(table.omittedPartialRecord)
            XCTAssertFalse(table.copyText.contains("C"))
        }
        let complete = try NativeDelimitedTable.parse("a,b\nA,1\n", separator: 44, truncated: true)
        XCTAssertEqual(complete.rows, [["A", "1"]]); XCTAssertFalse(complete.omittedPartialRecord)
    }
    func testRowDisplayCapDoesNotTruncateCopyAndNoncontiguousSelectionsRemainValid() throws {
        let table = try NativeDelimitedTable.parse("gene,value\n" + (0..<510).map { "G\($0),\($0)" }.joined(separator: "\n"), separator: 44)
        XCTAssertEqual(table.rows.count, 510); XCTAssertTrue(table.copyText.hasSuffix("G509\t509"))
        XCTAssertEqual(table.selectedText(IndexSet([0, 2, 509])), "G0\t0\nG2\t2")
        XCTAssertTrue(table.containsSelection("G0\t0\nG2\t2")); XCTAssertFalse(table.containsSelection("G0\t0\nforeign\t8"))
        let document = NativeFileTextDocument(try textPreview("many.csv", "gene,value\n" + (0..<8_100).map { "G\($0),\($0)" }.joined(separator: "\n")))
        XCTAssertEqual(document.table?.rows.count, 8_100); XCTAssertTrue(document.table?.copyText.hasSuffix("G8099\t8099") == true)
        XCTAssertTrue(document.rawDisplayClipped); XCTAssertFalse(document.clipped)
    }
    @MainActor func testRenderedMarkdownAndTableQuotesKeepSourceAndRejectForeignSelections() throws {
        let source = "---\ntitle: Report\n---\n# 🧬 Result\n\n**Sample A** passed.\n\n| Gene | Value |\n| --- | --- |\n| A | 2 |"
        let content = try textPreview("/work/report.md", source), document = NativeFileTextDocument(content)
        XCTAssertTrue(document.containsSelection("Sample A passed.")); XCTAssertTrue(document.containsSelection("🧬 Result\nSample A"))
        XCTAssertFalse(document.containsSelection("Sample B passed.")); XCTAssertFalse(document.containsSelection("   "))
        let table = NativeFileTextDocument(try textPreview("/work/counts.csv", "gene,value\n\"A,B\",2"))
        XCTAssertTrue(table.containsSelection("A,B\t2")); XCTAssertFalse(table.containsSelection("A,B\t3"))
    }
    @MainActor func testNativeTableSelectionCopyAndQuoteCaptureDisplayedLiteralRows() throws {
        let table = try NativeDelimitedTable.parse("gene,value\n<script>,2\n**B**,3", separator: 44)
        let view = NativeDelimitedTableControl(); let coordinator = NativeDelimitedTableView.Coordinator(); coordinator.table = table
        view.dataSource = coordinator; view.table = table; view.allowsMultipleSelection = true; view.reloadData(); view.selectRowIndexes(IndexSet([0, 1]), byExtendingSelection: false)
        var copied: [String] = [], quoted: [String] = []; view.copyText = { copied.append($0) }; view.quote = { quoted.append($0) }
        let actions = view.selectionActions(); XCTAssertEqual(actions.count, 2)
        view.table = try NativeDelimitedTable.parse("other\nchanged", separator: 44)
        actions.forEach { ($0.representedObject as? NativeSelectionAction)?.invoke(nil) }
        XCTAssertEqual(copied, ["<script>\t2\n**B**\t3"]); XCTAssertEqual(quoted, copied)
        coordinator.table = table
        let column = NSTableColumn(identifier: .init("0"))
        coordinator.quote = { quoted.append($0) }
        let cell = try XCTUnwrap(coordinator.tableView(view, viewFor: column, row: 0) as? NativeDelimitedCellView)
        XCTAssertEqual(cell.content.string, "<script>"); XCTAssertEqual(cell.content.toolTip, "<script>")
        cell.content.setSelectedRange(NSRange(location: 1, length: 6))
        let selection = try XCTUnwrap(cell.content.selectionActions().first?.representedObject as? NativeSelectionAction)
        selection.invoke(nil); XCTAssertEqual(quoted.last, "script")
    }
    func testImagePathsResolveAgainstDocumentWithoutLocalExpansionOfSshHome() {
        XCTAssertEqual(NativeFileImagePath.resolve("../figures/plot%20A.png", document: "/work/reports/result.md", remote: false), "/work/figures/plot A.png")
        XCTAssertEqual(NativeFileImagePath.resolve("plot%2520A.png", document: "/work/result.md", remote: false), "/work/plot%20A.png")
        XCTAssertEqual(NativeFileImagePath.resolve("../plot.png", document: "~/reports/result.md", remote: true), "~/plot.png")
        XCTAssertEqual(NativeFileImagePath.resolve("figures/plot.png", document: "/home/research/result.md", remote: true), "/home/research/figures/plot.png")
        for reference in ["https://example.com/plot.png", "data:image/png;base64,AA", "//foreign/plot.png", "file://foreign/plot.png", "plot.svg", "\nplot.png"] { XCTAssertNil(NativeFileImagePath.resolve(reference, document: "/work/result.md", remote: false), reference) }
        XCTAssertNil(NativeFileImagePath.resolve("../../plot.png", document: "~/reports/result.md", remote: true))
        XCTAssertNil(NativeFileImagePath.resolve("file:///work/plot.png", document: "/reports/result.md", remote: true))
    }
    @MainActor func testImageDecoderBoundsPixelsAndRejectsMalformedOrTruncatedBytes() throws {
        let large = try imagePreview(1100), image = try NativeFileDocumentImages.decode(large)
        XCTAssertEqual(image.size.width, 1024); XCTAssertEqual(image.size.height, 1024)
        var value = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(large))
        for (key, replacement) in [("truncated", SettingsValue.bool(true)), ("mime", .string("text/plain")), ("base64", .string("invalid"))] {
            var bad = value; bad[key] = replacement
            XCTAssertThrowsError(try NativeFileDocumentImages.decode(JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(bad))))
        }
        value["base64"] = .string(Data([0, 1, 2]).base64EncodedString())
        XCTAssertThrowsError(try NativeFileDocumentImages.decode(JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value))))
        let wide = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 8193, pixelsHigh: 1, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        value["base64"] = .string(try XCTUnwrap(wide.representation(using: .png, properties: [:])).base64EncodedString())
        XCTAssertThrowsError(try NativeFileDocumentImages.decode(JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value))))
    }
    @MainActor func testLateImageReadsCannotRestoreClearedOrReplacedDocument() async throws {
        let model = NativeFileDocumentImages(), gate = PreviewImageGate()
        let requests = NativeMessageImageRequest.requests(markdown: "![plot](plot.png)", resources: [])
        let old = Task { await model.load(requests, loader: { _ in try await gate.load() }) }
        while !(await gate.waiting()) { await Task.yield() }
        model.clear(); await gate.release(try imagePreview()); await old.value
        XCTAssertTrue(model.images.isEmpty); XCTAssertTrue(model.unavailable.isEmpty)
        await model.load(requests, loader: { _ in throw ProjectBrowserError.invalidResponse })
        XCTAssertEqual(model.unavailable, ["plot.png"])
        await model.load(requests, loader: { _ in try imagePreview() })
        XCTAssertNotNil(model.images["plot.png"]); XCTAssertTrue(model.unavailable.isEmpty)
    }
    @MainActor func testImmediateEscapeClosesRichPreviewBeforeFilesWithoutChangingFocus() throws {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        var parentClosed = 0, previewClosed = 0
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        let parent = NSHostingView(rootView: Text("Files").background(NativeSettingsEscape { parentClosed += 1 }))
        container.addSubview(parent); parent.frame = container.bounds; parent.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: NativePanelFilePreview(content: try textPreview("/work/report.md", "# Report\n\n**Result** passed."), close: { previewClosed += 1 }))
        container.addSubview(child); child.frame = container.bounds; child.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(previewClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
    }
    @MainActor func testImmediateEscapeClosesImageBeforeMarkdownDocumentWithoutMovingFocus() throws {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 320, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        var documentClosed = 0, imageClosed = 0
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        let parent = NSHostingView(rootView: NativePanelFilePreview(content: try textPreview("/work/report.md", "# Report\n\n![plot](plot.png)"), close: { documentClosed += 1 }))
        container.addSubview(parent); parent.frame = container.bounds; parent.layoutSubtreeIfNeeded()
        let child = NSHostingView(rootView: NativeMessageImageSheet(preview: .init(reference: "plot.png", image: try NativeFileDocumentImages.decode(imagePreview()))) { imageClosed += 1 })
        container.addSubview(child); child.frame = container.bounds; child.layoutSubtreeIfNeeded(); let focus = window.firstResponder
        let event = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(imageClosed, 1); XCTAssertEqual(documentClosed, 0); XCTAssertTrue(window.firstResponder === focus)
        child.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(documentClosed, 1)
    }
    @MainActor func testMarkdownSelectionAndCopyMenusUseCurrentLocale() {
        let old = UserDefaults.standard.object(forKey: "nativeSettings.locale"); defer { if let old { UserDefaults.standard.set(old, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            let view = NativeMessageTextView(); view.apply(NativeMarkdownContent.render("```python\nx = 1\n```", saved: [], scheme: .light))
            view.quote = { _ in }; view.save = { _ in }; view.setSelectedRange(NSRange(location: 0, length: 5))
            XCTAssertEqual(view.selectionActions().map(\.title), locale == "zh" ? ["引用到侧聊", "收藏划线"] : ["Quote in side chat", "Save highlight"])
            XCTAssertEqual(view.blockActions(at: 0).map(\.title), locale == "zh" ? ["复制代码"] : ["Copy code"])
        }
    }
    @MainActor func testRenderRichDocumentsInBothLocalesSchemesAndNarrowWidths() throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in native rendering") }
        let oldLocale = UserDefaults.standard.object(forKey: "nativeSettings.locale"); defer { if let oldLocale { UserDefaults.standard.set(oldLocale, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let markdown = "---\ntitle: QC report\n---\n# 🧬 QC report\n\n**样本 A / Sample A** passed.\n\n> Evidence from this run.\n\n1. Inspect reads\n2. Compare counts\n\n```python\nreads = 120\nprint(reads)\n```\n\n| Sample | Reads |\n| --- | ---: |\n| A | 120 |\n| B | 240 |"
        for locale in ["zh", "en"] { for scheme in [ColorScheme.light, .dark] { for width in [320.0, 419.0, 900.0] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for (kind, source) in [("markdown", markdown), ("csv", "sample,count,note\n\"样本,A\",120,\"first line\nsecond line\"\nB,240,pass\nC,,\nD,18,\"a,b\"")] {
                let content = try textPreview("/work/reports/QC." + (kind == "markdown" ? "md" : "csv"), source)
                let host = NSHostingView(rootView: NativePanelFilePreview(content: content, close: {}, save: { _ in }, quote: { _ in }).frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-elev", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 650), styleMask: [.titled], backing: .buffered, defer: false)
                window.isReleasedWhenClosed = false; window.appearance = host.appearance; window.contentView = host; window.makeKeyAndOrderFront(nil)
                defer { window.orderOut(nil); window.contentView = nil }
                RunLoop.current.run(until: Date().addingTimeInterval(0.3)); host.layoutSubtreeIfNeeded(); window.displayIfNeeded()
                let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                let name = "file-rich-\(kind)-\(locale)-\(scheme == .dark ? "dark" : "light")-\(Int(width)).png"
                try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent(name))
            }
        } } }
    }
}
