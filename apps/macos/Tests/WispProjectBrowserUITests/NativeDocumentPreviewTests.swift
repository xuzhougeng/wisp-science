import AppKit
import PDFKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor DocumentHost: NativeConversationQuerying {
    var content: SettingsValue
    var last: [String: SettingsValue] = [:]
    init(content: SettingsValue) { self.content = content }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue { last = args; return content }
    func arguments() -> [String: SettingsValue] { last }
}

final class NativeDocumentPreviewTests: XCTestCase {
    private func content(_ path: String, text: String? = nil, bytes: Data? = nil, truncated: Bool = false) throws -> NativePanelFileContent {
        let value: SettingsValue = .object(["path": .string(path), "mime": .string(bytes == nil ? "text/plain" : "application/pdf"), "text": text.map(SettingsValue.string) ?? .null, "base64": bytes.map { .string($0.base64EncodedString()) } ?? .null, "truncated": .bool(truncated), "total_bytes": .integer(Int64(bytes?.count ?? text?.utf8.count ?? 0))])
        return try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value))
    }
    func testDetectsCorrespondingOfflineViewerFormats() throws {
        for (ext, kind) in [("pdf", NativeDocumentKind.pdf), ("docx", .docx), ("xlsx", .xlsx), ("pptx", .pptx), ("html", .html), ("pdb", .structure), ("mmcif", .structure), ("mol2", .structure), ("smiles", .molecule), ("fa", .fasta), ("aln", .msa), ("stockholm", .msa)] {
            XCTAssertEqual(NativeDocumentKind.detect(try content("remote:ssh:CPU:/data/source." + ext)), kind)
        }
        XCTAssertNil(NativeDocumentKind.detect(try content("plot.png")))
        XCTAssertEqual(NativeDocumentKind.structure.format(path: "file.mmcif"), "cif")
        for ext in ["sdf", "mol"] { XCTAssertEqual(NativeDocumentKind.detect(try content("file." + ext)), .molecule) }
        for ext in ["fas", "ffn", "frn"] { XCTAssertEqual(NativeDocumentKind.detect(try content("file." + ext)), .fasta) }
        for (ext, format) in [("clustalw", "clustal"), ("stk", "stockholm"), ("afa", "fasta"), ("mfa", "fasta")] {
            XCTAssertEqual(NativeDocumentKind.detect(try content("file." + ext)), .msa)
            XCTAssertEqual(NativeDocumentKind.msa.format(path: "file." + ext), format)
        }
    }
    func testDocumentSelectionRejectsWrongKindsPositionsAndOversizedText() throws {
        let selection = NativeDocumentSelection(text: "结果 🌱", kind: .pdf, page: 2, endPage: 3)
        XCTAssertTrue(selection.valid)
        XCTAssertNil(selection.quote(from: try content("/data/source.docx", bytes: Data())))
        XCTAssertNil(selection.quote(from: try content("/data/source.pdf", bytes: Data(), truncated: true)))
        XCTAssertFalse(NativeDocumentSelection(text: "x", kind: .pdf, page: 0).valid)
        XCTAssertFalse(NativeDocumentSelection(text: "x", kind: .pdf, page: 3, endPage: 2).valid)
        XCTAssertFalse(NativeDocumentSelection(text: String(repeating: "水", count: 22000), kind: .pdf, page: 1).valid)
        XCTAssertTrue(NativeDocumentSelection(text: "值：3\n公式：=SUM(A1:A2)", kind: .xlsx, sheet: "QC", cells: "A3").valid)
        XCTAssertFalse(NativeDocumentSelection(text: "x", kind: .xlsx, sheet: "QC", cells: "A0").valid)
        XCTAssertFalse(NativeDocumentSelection(text: "x", kind: .xlsx, sheet: "QC\n", cells: "A3").valid)
        let parsed: SettingsValue = .object(["text": .string("Claim"), "location": .object(["kind": .string("docx"), "page": .integer(1)])])
        XCTAssertNotNil(NativeDocumentSelection.parse(parsed, expected: .docx))
        XCTAssertNil(NativeDocumentSelection.parse(parsed, expected: .pdf))
    }
    func testAssetRoutesRejectForeignTokensTraversalAndSymlinks() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("native-preview-assets-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try Data("offline".utf8).write(to: root.appendingPathComponent("viewer.mjs"))
        let outside = root.deletingLastPathComponent().appendingPathComponent("outside-" + UUID().uuidString + ".mjs")
        try Data("not approved".utf8).write(to: outside); defer { try? FileManager.default.removeItem(at: outside) }
        try FileManager.default.createSymbolicLink(at: root.appendingPathComponent("escape.mjs"), withDestinationURL: outside)
        let assets = NativePreviewAssets(presentation: root, vendor: root)
        XCTAssertEqual(assets.read("/token/vendor-runtime/viewer.mjs", token: "token")?.mime, "text/javascript")
        for path in ["/other/viewer.mjs", "/token/../viewer.mjs", "/token/%2e%2e/viewer.mjs", "/token/escape.mjs", "/token/file%5cname.mjs", "/token/file%00.mjs", "https://example.com/token/viewer.mjs"] { XCTAssertNil(assets.read(path, token: "token"), path) }
        XCTAssertNotNil(NativePreviewAssets.bundled.read("/token/index.html", token: "token"))
        XCTAssertNotNil(NativePreviewAssets.bundled.read("/token/vendor-runtime/rdkit-worker.mjs", token: "token"))
    }
    func testNavigationKeepsFragmentsInsideTheSameDocumentAndOpaqueFrames() {
        let root = URL(string: "http://127.0.0.1:1234/token/index.html")!
        XCTAssertTrue(NativePreviewNavigation.sameDocument(URL(string: root.absoluteString + "#section"), root))
        XCTAssertTrue(NativePreviewNavigation.allowed(URL(string: "about:blank"), document: root, mainFrame: false))
        XCTAssertTrue(NativePreviewNavigation.allowed(URL(string: "about:srcdoc"), document: root, mainFrame: false))
        for path in ["http://127.0.0.1:1234/token-evil/index.html", "http://127.0.0.1:1234/token/index.html?q=other", "https://example.com/", "file:///tmp/private"] {
            XCTAssertFalse(NativePreviewNavigation.allowed(URL(string: path), document: root, mainFrame: true))
        }
        XCTAssertFalse(NativePreviewNavigation.allowed(URL(string: "http://127.0.0.1:1234/token-evil/frame.html"), document: root, mainFrame: false))
    }
    @MainActor func testPDFPaginationSearchAndSelectionUseActualPagePositions() throws {
        let bytes = Self.pdf()
        let source = try content("paper.pdf", bytes: bytes)
        let model = NativePDFModel(content: source)
        XCTAssertEqual(model.document?.pageCount, 2)
        let view = PDFView(); view.document = model.document; model.view = view
        model.navigate(2); XCTAssertEqual(model.page, 2); XCTAssertTrue(view.currentPage === model.document?.page(at: 1))
        model.navigate(3); XCTAssertEqual(model.page, 2)
        model.query = "Evidence"; model.search(); XCTAssertEqual(model.matches.count, 2)
        XCTAssertEqual(model.selection?.page, 1); model.nextMatch(); XCTAssertEqual(model.selection?.page, 2)
        XCTAssertTrue(model.selection?.quote(from: source)?.source.contains("2") == true)
        XCTAssertNil(NativePDFModel(content: try content("broken.pdf", bytes: Data("bad".utf8))).document)
        XCTAssertNil(NativePDFModel(content: try content("partial.pdf", bytes: bytes, truncated: true)).document)
    }
    @MainActor func testReadRequestsPageBytesAndLateQuotesCannotTargetNewPreview() async throws {
        let pdf = try content("paper.pdf", bytes: Self.pdf())
        let host = DocumentHost(content: try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(pdf)))
        let model = NativePanelModel(client: host, projectID: "p", sessionID: "s")
        await model.readFile("paper.pdf")
        let args = await host.arguments(); XCTAssertEqual(args["render_pdf"], .bool(true)); XCTAssertEqual(args["render_office"], .bool(true))
        let identity = model.previewIdentity
        let selection = NativeDocumentSelection(text: "Evidence", kind: .pdf, page: 1)
        XCTAssertNotNil(model.documentQuote(selection, original: pdf, identity: identity))
        model.dismissPreview(); await model.readFile("paper.pdf")
        XCTAssertNil(model.documentQuote(selection, original: pdf, identity: identity))
    }
    @MainActor func testHTMLImagesUseOnlyApprovedCompleteScopedBytes() async throws {
        let image = try imagePreview()
        let source = "<img src='plot.png'><img src=\"plot.png\"><img src='missing.png'><img src='data:image/png;base64,AA=='>"
        var requested: [String] = []
        let prepared = await NativeHTMLPreviewImages.prepare(source, loader: { path in
            requested.append(path)
            if path == "plot.png" { return image }
            throw ProjectBrowserError.invalidResponse
        })
        XCTAssertEqual(requested, ["plot.png", "missing.png"])
        XCTAssertTrue(prepared.contains("data:image/png;base64," + (image.base64 ?? "")))
        XCTAssertTrue(prepared.contains("src='missing.png'"))
        let unchanged = await NativeHTMLPreviewImages.prepare(source, loader: nil)
        XCTAssertEqual(unchanged, source)
    }
    @MainActor static func pdf() -> Data {
        let bytes = NSMutableData(); let consumer = CGDataConsumer(data: bytes as CFMutableData)!
        var bounds = CGRect(x: 0, y: 0, width: 300, height: 300)
        let context = CGContext(consumer: consumer, mediaBox: &bounds, nil)!
        for page in 1...2 {
            context.beginPDFPage(nil); NSGraphicsContext.saveGraphicsState()
            NSGraphicsContext.current = NSGraphicsContext(cgContext: context, flipped: false)
            NSAttributedString(string: "Evidence page \(page)", attributes: [.font: NSFont.systemFont(ofSize: 16)]).draw(at: NSPoint(x: 30, y: 200))
            NSGraphicsContext.restoreGraphicsState(); context.endPDFPage()
        }
        context.closePDF(); return bytes as Data
    }
    @MainActor func testRenderPDFControlsInBothLocalesSchemesAndWidths() throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in native document rendering") }
        let old = UserDefaults.standard.object(forKey: "nativeSettings.locale"); defer { if let old { UserDefaults.standard.set(old, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        for locale in ["en", "zh"] { for scheme in [ColorScheme.light, .dark] { for width in [432.0, 1060.0] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            let content = try content("/project/科研证据-paper.pdf", bytes: Self.pdf())
            let host = NSHostingView(rootView: NativePanelFilePreview(content: content, close: {}, documentQuote: { _ in true }).frame(width: width, height: 700).background(WispDesign.color("bg-elev", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).font(WispDesign.font(size: 14)).buttonStyle(WispButtonStyle()).environment(\.colorScheme, scheme))
            host.frame = NSRect(x: 0, y: 0, width: width, height: 700); host.layoutSubtreeIfNeeded()
            let window = NSWindow(contentRect: host.bounds, styleMask: [.titled], backing: .buffered, defer: false)
            window.contentView = host; host.layoutSubtreeIfNeeded(); RunLoop.current.run(until: Date().addingTimeInterval(0.05))
            let image = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: image)
            let bytes = try XCTUnwrap(image.representation(using: .png, properties: [:])); try bytes.write(to: URL(fileURLWithPath: directory).appendingPathComponent("pdf-viewer-\(locale)-\(scheme)-\(Int(width)).png"))
            window.contentView = nil
        } } }
    }
}
