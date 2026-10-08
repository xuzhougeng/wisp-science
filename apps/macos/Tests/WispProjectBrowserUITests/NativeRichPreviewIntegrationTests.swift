import AppKit
import SwiftUI
import WebKit
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

/// Uses the packaged renderer origin and real WebKit workers/WASM, without a
/// model, external service, SSH host or document upload.
final class NativeRichPreviewIntegrationTests: XCTestCase {
    @MainActor func testOfflineOfficeScientificAndOpaqueHTMLRenderInWebKit() async throws {
        guard ProcessInfo.processInfo.environment["WISP_NATIVE_DOCUMENT_SMOKE"] == "1" else { throw XCTSkip("Opt-in real WebKit document rendering") }
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixtures = root.appendingPathComponent("ui-tests/fixtures")
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 620, height: 650), styleMask: [.titled], backing: .buffered, defer: false)
        defer { window.contentView = nil; window.orderOut(nil) }
        let cases: [(NativeDocumentKind, String, String?, String)] = [
            (.docx, "office-preview.docx", nil, "!!document.querySelector('.rp-docx table')"),
            (.xlsx, "office-preview.xlsx", nil, "!!document.querySelector('.rp-xlsx-cell') && getComputedStyle(document.querySelector('.rp-xlsx-content')).backgroundImage !== 'none' && getComputedStyle(document.querySelector('.rp-xlsx-col-head')).borderBottomStyle === 'solid' && document.querySelector('.rp-xlsx-grid').clientHeight > 400"),
            (.pptx, "office-preview.pptx", nil, "!!document.querySelector('.rp-pptx svg')"),
            (.fasta, "sequence.fa", ">sample\nACGT\n", "!!document.querySelector('.rp-fasta-hdr')"),
            (.molecule, "ethanol.smi", "CCO", "!!document.querySelector('svg.rp-molecule path')"),
            (.msa, "alignment.aln", "CLUSTAL W\n\none AC-G\ntwo ACTG\n", "document.querySelector('iframe.rp-alignment')?.contentDocument?.body.dataset.ready === 'true'"),
            (.structure, "ethanol.pdb", "HETATM    1  C1  LIG A   1       0.000   0.000   0.000  1.00 20.00           C\nEND\n", "document.querySelector('iframe.rp-3dmol')?.contentDocument?.body.dataset.ready === 'true'"),
            (.html, "report.html", "<h1>结果 🌱</h1><script>setTimeout(()=>{const r=document.createRange();r.selectNodeContents(document.querySelector('h1'));const s=getSelection();s.removeAllRanges();s.addRange(r)},100)</script>", "!!document.querySelector('iframe.rp-native-html')")
        ]
        for (kind, path, text, rendered) in cases {
            print("NativePreviewSmoke: \(path)")
            let bytes = text == nil ? try Data(contentsOf: fixtures.appendingPathComponent(path)) : nil
            let value: SettingsValue = .object(["path": .string(path), "mime": .string("text/plain"), "text": text.map(SettingsValue.string) ?? .null, "base64": bytes.map { .string($0.base64EncodedString()) } ?? .null, "truncated": .bool(false), "total_bytes": .integer(Int64(bytes?.count ?? text?.utf8.count ?? 0))])
            let content = try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value))
            let state = NativeRichPreviewState()
            var quotes: [NativeDocumentSelection] = []
            var host: NSHostingView<NativeRichPreviewSurface>? = NSHostingView(rootView: NativeRichPreviewSurface(content: content, kind: kind, html: kind == .html ? text : nil, scheme: .light, state: state, quote: { quotes.append($0); return true }))
            window.contentView = host; window.orderFront(nil); host?.layoutSubtreeIfNeeded()
            try await until { state.ready || state.error != nil }
            XCTAssertNil(state.error, path)
            if let error = state.error { throw NSError(domain: "NativeRichPreviewSmoke", code: 2, userInfo: [NSLocalizedDescriptionKey: error]) }
            let view = try XCTUnwrap(Self.webView(try XCTUnwrap(host)), path)
            do { try await until { (try? await view.evaluateJavaScript(rendered)) as? Bool == true } }
            catch { XCTFail("\(path): \(String(describing: try? await view.evaluateJavaScript("document.body.innerText")))"); throw error }
            let errors = try await view.evaluateJavaScript("document.querySelectorAll('.rp-error').length") as? Int
            XCTAssertEqual(errors, 0, path)
            if kind == .html {
                try await until { state.hasSelection }
                _ = try await view.evaluateJavaScript("window.webkit.messageHandlers.preview.postMessage({type:'quote',text:'unrequested',location:{kind:'html'}}); true")
                XCTAssertTrue(quotes.isEmpty)
                state.requestQuote?(); try await until { quotes.count == 1 }
                XCTAssertEqual(quotes.first?.text, "结果 🌱")
                XCTAssertEqual(quotes.first?.quote(from: content)?.source, "report.html · " + localized("HTML 阅读视图"))
                let sandbox = try await view.evaluateJavaScript("document.querySelector('iframe').sandbox.value") as? String
                XCTAssertEqual(sandbox, "allow-scripts")
                let width = try await view.evaluateJavaScript("document.querySelector('iframe').getBoundingClientRect().width") as? Double
                XCTAssertGreaterThan(width ?? 0, 500)
            }
            window.contentView = nil; host = nil
            try await until { state.requestQuote == nil }
        }
    }
    @MainActor private func until(_ predicate: () async throws -> Bool) async throws {
        let deadline = Date().addingTimeInterval(25)
        while Date() < deadline {
            if try await predicate() { return }
            try await Task.sleep(nanoseconds: 30_000_000)
        }
        throw NSError(domain: "NativeRichPreviewSmoke", code: 1, userInfo: [NSLocalizedDescriptionKey: "Timed out waiting for offline renderer"])
    }
    @MainActor private static func webView(_ root: NSView) -> WKWebView? {
        if let view = root as? WKWebView { return view }
        return root.subviews.lazy.compactMap(webView).first
    }
}
