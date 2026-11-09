import AppKit
import Foundation
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeShareTests: XCTestCase {
    func testPaginationPreservesEveryScanlineAndOnlyBreaksInWhitespace() throws {
        let ranges = try NativeSharePagination.ranges(height: 280, pageHeight: 100) { [90, 180].contains($0) }
        XCTAssertEqual(ranges, [0..<90, 90..<180, 180..<280])
        XCTAssertEqual(ranges.reduce(0) { $0 + $1.count }, 280)
        XCTAssertThrowsError(try NativeSharePagination.ranges(height: 200, pageHeight: 100) { _ in false })
    }
    @MainActor func testPagedPNGUsesThreeByFourPagesAtBothThemes() throws {
        let rows = Array(repeating: try fixture("share-complex"), count: 5).flatMap { $0 }
        for scheme in [ColorScheme.light, .dark] {
            if let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] {
                try NativeSharePage.png(rows: rows, width: 840, scheme: scheme).write(to: URL(fileURLWithPath: directory).appendingPathComponent("share-pages-source-\(scheme).png"))
            }
            let pages = try NativeSharePagination.pages(rows: rows, width: 840, scheme: scheme)
            XCTAssertGreaterThan(pages.count, 1)
            for (index, data) in pages.enumerated() {
                let image = try XCTUnwrap(NSBitmapImageRep(data: data))
                XCTAssertEqual(image.pixelsWide, 840); XCTAssertEqual(image.pixelsHigh, 1120)
                if let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] {
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("share-page-\(scheme)-\(index).png"))
                }
            }
        }
    }
    @MainActor func testPaginationRetainsAllRenderedPixelsInOrder() throws {
        let rows = Array(repeating: try fixture("share-complex"), count: 5).flatMap { $0 }
        func pixels(_ image: CGImage) throws -> [UInt8] {
            var bytes = [UInt8](repeating: 0, count: image.width * image.height * 4)
            try bytes.withUnsafeMutableBytes { buffer in
                let context = try XCTUnwrap(CGContext(data: buffer.baseAddress, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: image.width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
                context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
            }
            return bytes
        }
        for scheme in [ColorScheme.light, .dark] {
            let source = try XCTUnwrap(NSBitmapImageRep(data: NativeSharePage.png(rows: rows, width: 840, scheme: scheme))?.cgImage)
            let pages = try NativeSharePagination.render(image: source, scheme: scheme)
            var nextRow = 0
            for page in pages {
                XCTAssertEqual(page.source.lowerBound, nextRow)
                let expected = try XCTUnwrap(source.cropping(to: CGRect(x: 0, y: nextRow, width: source.width, height: page.source.count)))
                let decoded = try XCTUnwrap(NSBitmapImageRep(data: page.png)?.cgImage)
                let actual = try XCTUnwrap(decoded.cropping(to: CGRect(x: 0, y: 16, width: source.width, height: page.source.count)))
                let expectedBytes = try pixels(expected), actualBytes = try pixels(actual)
                XCTAssertEqual(expectedBytes.count, actualBytes.count)
                XCTAssertLessThanOrEqual(zip(expectedBytes, actualBytes).map { abs(Int($0) - Int($1)) }.max() ?? 0, 2, "Every source pixel must survive the ordered page slices")
                nextRow = page.source.upperBound
            }
            XCTAssertEqual(nextRow, source.height)
        }
    }
    func fixture(_ name: String = "share") throws -> [NativeShareRow] {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode([NativeShareRow].self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/\(name).json")))
    }
    func testSelectionAndRedactionExcludeThinkingByDefault() throws {
        let draft = NativeShare.draft(try fixture())
        XCTAssertEqual(draft.map(\.selected), [true, false, true])
        let rows = NativeShare.selected(draft, keywords: " Alice，Alice\n样本一 ")
        XCTAssertEqual(rows.count, 2)
        XCTAssertFalse(rows.map(\.text).joined().lowercased().contains("alice"))
        XCTAssertFalse(rows.map(\.text).joined().contains("样本一"))
        XCTAssertEqual(NativeShare.keywords("Alice,Alice,Ali"), ["Alice", "Ali"])
        XCTAssertTrue(draft[0].row.text.contains("Alice"), "Export must not mutate the stored transcript")
    }
    func testMarkdownBlocksPreserveCodeAndSeparateBullets() {
        let blocks = NativeShareMarkdown.blocks("# Results\n\n- first\n- second\n\n```python\n# literal\nprint(1)\n```")
        XCTAssertEqual(blocks.map(\.kind), ["heading", "bullet", "bullet", "code"])
        XCTAssertEqual(blocks.last?.text, "# literal\nprint(1)")
    }
    func testTablesPreserveEscapedAndCodePipesAndPadShortRows() {
        let blocks = NativeShareMarkdown.blocks("| Sample | Value |\n| --- | :---: |\n| A | `x|y` |\n| B | one\\|two |\n| C |")
        XCTAssertEqual(blocks.count, 1)
        XCTAssertEqual(blocks[0].kind, "table")
        XCTAssertEqual(blocks[0].alignments, ["left", "center"])
        XCTAssertEqual(blocks[0].rows, [["Sample", "Value"], ["A", "`x|y`"], ["B", "one\\|two"], ["C", ""]])
        XCTAssertEqual(NativeShareMarkdown.blocks("a | b\nnot | divider").first?.kind, "paragraph")
    }
    func testNestedListsAndLongFencesKeepTheirStructure() {
        let blocks = NativeShareMarkdown.blocks("1. first\n   - nested\n2) second\n\n````markdown\n```python\nprint(1)\n```\n````\n## Results")
        XCTAssertEqual(blocks.map(\.kind), ["ordered", "bullet", "ordered", "code", "heading"])
        XCTAssertEqual(blocks[0].marker, "1.")
        XCTAssertEqual(blocks[1].level, 1)
        XCTAssertEqual(blocks[2].marker, "2.")
        XCTAssertEqual(blocks[3].text, "```python\nprint(1)\n```")
        XCTAssertEqual(blocks[4].level, 2)
    }
    @MainActor func testComplexMarkdownPNGAtNarrowAndWideWidths() throws {
        let rows = try fixture("share-complex")
        for width in [320, 840] {
            let data = try NativeSharePage.png(rows: rows, width: width, scheme: .light)
            let image = try XCTUnwrap(NSBitmapImageRep(data: data))
            XCTAssertEqual(image.pixelsWide, width)
            XCTAssertGreaterThan(image.pixelsHigh, 300)
            if let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] {
                try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("share-complex-\(width).png"))
            }
        }
    }
    func testWidthMatchesWebViewBounds() {
        XCTAssertEqual(NativeShare.width(""), 840)
        XCTAssertEqual(NativeShare.width("bad"), 840)
        XCTAssertEqual(NativeShare.width("12"), 320)
        XCTAssertEqual(NativeShare.width("9999"), 2400)
        XCTAssertEqual(NativeShare.width("640"), 640)
    }
    @MainActor func testPNGHasRequestedPixelWidthAndCanRenderRedactedSelection() throws {
        let rows = NativeShare.selected(NativeShare.draft(try fixture()), keywords: "Alice")
        let data = try NativeSharePage.png(rows: rows, width: 640, scheme: .light)
        let image = try XCTUnwrap(NSBitmapImageRep(data: data))
        XCTAssertEqual(image.pixelsWide, 640)
        XCTAssertGreaterThan(image.pixelsHigh, 100)
        if let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] {
            try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
            try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("share.png"))
        }
    }
}
