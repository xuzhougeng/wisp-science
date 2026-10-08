import AppKit
import SwiftUI
import XCTest
@testable import WispProjectBrowserUI

final class NativeCodeHighlightTests: XCTestCase {
    func testRelocatedAppLoadsGrammarWithoutEvaluatingSwiftPMFallback() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let app = directory.appendingPathComponent("Relocated SwiftUI.app")
        let contents = app.appendingPathComponent("Contents")
        let resources = contents.appendingPathComponent("Resources")
        try FileManager.default.createDirectory(at: resources, withIntermediateDirectories: true)
        let info: [String: String] = ["CFBundleIdentifier": "science.wisp-science.resource-test", "CFBundlePackageType": "APPL"]
        try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0)
            .write(to: contents.appendingPathComponent("Info.plist"))
        let packaged = resources.appendingPathComponent("WispScience_WispProjectBrowserUI.bundle")
        try FileManager.default.copyItem(at: WispDesign.resources.bundleURL, to: packaged)
        let main = try XCTUnwrap(Bundle(url: app))
        var fallbackCalled = false
        let bundle = WispDesign.resourceBundle(in: main, module: {
            fallbackCalled = true
            return .main
        })
        XCTAssertFalse(fallbackCalled)
        XCTAssertEqual(bundle.bundleURL.path, packaged.path)
        let context = try XCTUnwrap(NativeCodeHighlight.makeContext(resources: bundle))
        let tokens = try XCTUnwrap(context.objectForKeyedSubscript("wispTokens")?
            .call(withArguments: ["def example():\n    return 42\n", "python"])?.toArray() as? [[String: Any]])
        XCTAssertTrue(tokens.contains { $0["scope"] as? String == "keyword" })
        XCTAssertTrue(tokens.contains { $0["scope"] as? String == "number" })
        XCTAssertNil(context.exception)
    }

    func testCommandLineUsesLazyModuleFallbackAndMissingGrammarIsOptional() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString + ".bundle")
        defer { try? FileManager.default.removeItem(at: directory) }
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let empty = try XCTUnwrap(Bundle(url: directory))
        var fallbackCalled = false
        let bundle = WispDesign.resourceBundle(in: empty, module: {
            fallbackCalled = true
            return WispDesign.resources
        })
        XCTAssertTrue(fallbackCalled)
        XCTAssertEqual(bundle.bundleURL, WispDesign.resources.bundleURL)
        XCTAssertNil(NativeCodeHighlight.makeContext(resources: empty))
    }

    @MainActor func testDefaultHighlighterColorsCodeWithoutChangingCopyText() throws {
        let source = "def example():\n    return 42\n"
        for scheme in [ColorScheme.light, .dark] {
            let value = NSMutableAttributedString(string: source, attributes: [.foregroundColor: NSColor.labelColor])
            NativeCodeHighlight.apply(to: value, language: "python", scheme: scheme)
            XCTAssertEqual(value.string, source)
            let color = try XCTUnwrap(value.attribute(.foregroundColor, at: 0, effectiveRange: nil) as? NSColor)
            XCTAssertNotEqual(color, NSColor.labelColor)
        }
    }
}
