import AppKit
import SwiftUI
import WebKit
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func repository() -> URL {
    var root = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { root.deleteLastPathComponent() }
    return root
}
private func reviewFixture() throws -> SettingsValue {
    try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: repository().appendingPathComponent("contracts/native-files/v1/docx-review.json")))
}
private func content(_ value: SettingsValue) throws -> NativePanelFileContent {
    try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value["content"]))
}
private actor ReviewFake: NativeConversationQuerying {
    var value: SettingsValue
    var calls: [(String, String, [String: SettingsValue])] = []
    var failure = "", hold = ""
    var held: CheckedContinuation<Void, Never>?
    init(_ value: SettingsValue) { self.value = value }
    func configure(failure: String = "", hold: String = "") { self.failure = failure; self.hold = hold }
    func recorded() -> [(String, String, [String: SettingsValue])] { calls }
    func waiting() -> Bool { held != nil }
    func release() { held?.resume(); held = nil; hold = "" }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((projectID, command, args))
        if command.hasSuffix("_readfile") || command.hasSuffix("_readartifact") { return value["content"] }
        let failure = failure
        if command.hasSuffix("docx_resolve") {
            guard args["source_sha256"] == value["review"]["source_sha256"] else { throw ProjectBrowserError.service("Stale hash") }
            let selection = args["selection"]!
            var review = value["review"]
            if failure != "conflict" {
                review["changes"] = .array(review["changes"].array.filter { change in
                    change[selection["type"] == .string("kind") ? "kind" : "id"] != selection["value"]
                })
            }
            review["source_sha256"] = .string(String(repeating: String(calls.count % 10), count: 64))
            value["review"] = review
            if review["changes"].array.isEmpty {
                let bytes = try Data(contentsOf: repository().appendingPathComponent("ui-tests/fixtures/office-preview.docx"))
                var file = value["content"]; file["base64"] = .string(bytes.base64EncodedString()); file["total_bytes"] = .integer(Int64(bytes.count)); value["content"] = file
            }
            self.failure = ""
        }
        var result = value
        if failure == "owner" { result["project_id"] = .string("foreign") }
        if command.hasSuffix(hold), !hold.isEmpty { await withCheckedContinuation { held = $0 } }
        if command.hasSuffix("docx_resolve"), ["conflict", "lost"].contains(failure) { throw ProjectBrowserError.service(failure) }
        return result
    }
}

final class NativeDocxReviewTests: XCTestCase {
    @MainActor private func model(_ fake: ReviewFake, _ fixture: SettingsValue, readOnly: Bool = false, session: String? = "session-1") throws -> NativeDocxReviewModel {
        NativeDocxReviewModel(content: try content(fixture), target: NativeDocxReviewTarget(client: fake, projectID: "project-1", sessionID: session, readOnly: readOnly))
    }
    func testSharedContractRejectsWrongOwnerPathAndMalformedSnapshots() throws {
        let fixture = try reviewFixture()
        func decode(_ value: SettingsValue) throws -> NativeDocxReview {
            try NativeDocxReview.decode(value, project: "project-1", session: "session-1", path: "/project/review.docx")
        }
        XCTAssertEqual(try decode(fixture).review.changes.map(\.kind), [.rewrite, .deletion, .insertion])
        for key in ["project_id", "session_id", "path", "schema"] {
            var bad = fixture; bad[key] = .string("foreign"); XCTAssertThrowsError(try decode(bad))
        }
        var bad = fixture, review = fixture["review"]
        review["source_sha256"] = .string("invalid"); bad["review"] = review; XCTAssertThrowsError(try decode(bad))
        review = fixture["review"]; review["changes"] = .array([review["changes"].array[0], review["changes"].array[0]]); bad["review"] = review
        XCTAssertThrowsError(try decode(bad))
        XCTAssertEqual(DocxReviewSelection.kind(.rewrite).value, .object(["type": .string("kind"), "value": .string("rewrite")]))
    }
    @MainActor func testIndividualRejectionAndTypeBatchesUseLatestHashAndRefreshDocument() async throws {
        let fixture = try reviewFixture(), fake = ReviewFake(fixture), model = try model(fake, fixture)
        await model.refresh()
        let first = try XCTUnwrap(model.snapshot)
        await model.resolve(.kind(.deletion), action: .accept)
        XCTAssertEqual(model.snapshot?.changes.map(\.kind), [.rewrite, .insertion])
        await model.resolve(.change(first.changes[0].id), action: .reject)
        XCTAssertEqual(model.snapshot?.changes.map(\.kind), [.insertion])
        await model.resolve(.change(first.changes[2].id), action: .accept)
        XCTAssertTrue(try XCTUnwrap(model.snapshot).changes.isEmpty)
        XCTAssertNotEqual(model.content.base64, try content(fixture).base64)
        let calls = await fake.recorded()
        XCTAssertEqual(calls.count, 4)
        XCTAssertTrue(calls.allSatisfy { $0.0 == "project-1" && $0.2["session_id"] == .string("session-1") && $0.2["path"] == .string("/project/review.docx") })
        XCTAssertEqual(calls[1].2["source_sha256"], .string(first.source_sha256))
        XCTAssertEqual(calls[2].2["source_sha256"], .string(String(repeating: "2", count: 64)))
        XCTAssertEqual(calls[2].2["action"], .string("reject"))
        XCTAssertFalse(model.canResolve(.kind(.rewrite)))
        // Separate project-scoped journey accepts rewrites, retaining deletions.
        var project = fixture; project["session_id"] = .null
        let projectFake = ReviewFake(project), projectModel = try self.model(projectFake, project, session: nil)
        await projectModel.refresh(); await projectModel.resolve(.kind(.rewrite), action: .accept)
        XCTAssertEqual(projectModel.snapshot?.changes.map(\.kind), [.deletion, .insertion])
        let projectCalls = await projectFake.recorded()
        XCTAssertEqual(projectCalls.map { $0.1 }, ["native_project_file_docx_review", "native_project_file_docx_resolve"])
        XCTAssertEqual(projectCalls.last?.2["session_id"], .null)
    }
    @MainActor func testConflictAndLostReplyDisableWritesUntilExplicitRefresh() async throws {
        for failure in ["conflict", "lost"] {
            let fixture = try reviewFixture(), fake = ReviewFake(fixture), model = try model(fake, fixture)
            await model.refresh(); await fake.configure(failure: failure)
            await model.resolve(.kind(.rewrite), action: .accept)
            XCTAssertTrue(model.stale); XCTAssertNotNil(model.error)
            await model.resolve(.kind(.deletion), action: .accept)
            let calls = await fake.recorded(); XCTAssertEqual(calls.count, 2)
            await model.refresh(); XCTAssertFalse(model.stale); XCTAssertNil(model.error)
            XCTAssertEqual(model.snapshot?.changes.count, failure == "lost" ? 2 : 3)
            await model.resolve(.kind(.deletion), action: .accept)
            XCTAssertNil(model.error)
        }
    }
    @MainActor func testReadOnlyReferencesAndUnsupportedSelectionsNeverWrite() async throws {
        let fixture = try reviewFixture()
        for localReadOnly in [true, false] {
            var value = fixture, review = fixture["review"]; review["writable"] = .bool(localReadOnly); value["review"] = review
            let fake = ReviewFake(value), model = try model(fake, value, readOnly: localReadOnly)
            await model.refresh(); await model.resolve(.kind(.rewrite), action: .accept)
            XCTAssertFalse(model.canResolve); let calls = await fake.recorded(); XCTAssertEqual(calls.count, 1)
        }
        for path in ["ssh://host/review.docx", "artifact-version:review.docx", "/project/review.docx"] {
            var value = fixture, file = fixture["content"]; file["path"] = .string(path); value["content"] = file
            let model = NativeDocxReviewModel(content: try content(value), target: nil)
            await model.refresh(); XCTAssertFalse(model.canResolve); XCTAssertNil(model.snapshot)
        }
        let fake = ReviewFake(fixture), model = try model(fake, fixture)
        await model.refresh(); await model.resolve(.change("foreign"), action: .accept)
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 1)
    }
    @MainActor func testClosedPreviewIgnoresLateReadsAndSaveRepliesAndDoesNotDoubleSubmit() async throws {
        for operation in ["docx_review", "docx_resolve"] {
            let fixture = try reviewFixture(), fake = ReviewFake(fixture), model = try model(fake, fixture)
            if operation == "docx_resolve" { await model.refresh() }
            await fake.configure(hold: operation)
            let pending = Task { if operation == "docx_review" { await model.refresh() } else { await model.resolve(.kind(.rewrite), action: .accept) } }
            try await until { await fake.waiting() }
            let old = model.snapshot?.source_sha256
            await model.resolve(.kind(.deletion), action: .accept) // Busy: no second mutation.
            model.close(); await fake.release(); await pending.value
            XCTAssertEqual(model.snapshot?.source_sha256, old); XCTAssertFalse(model.busy); XCTAssertFalse(model.canResolve)
            let calls = await fake.recorded(); XCTAssertEqual(calls.count, operation == "docx_review" ? 1 : 2)
        }
    }
    @MainActor func testForeignResponseFailsClosed() async throws {
        let fixture = try reviewFixture(), fake = ReviewFake(fixture), model = try model(fake, fixture)
        await fake.configure(failure: "owner"); await model.refresh()
        XCTAssertNil(model.snapshot); XCTAssertTrue(model.stale); XCTAssertNotNil(model.error)
        await model.resolve(.kind(.rewrite), action: .accept)
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 1)
    }

    @MainActor func testLocalFileReviewCapabilityDoesNotCarryOverToArtifactPreview() async throws {
        let fixture = try reviewFixture(), fake = ReviewFake(fixture)
        let panel = NativePanelModel(client: fake, projectID: "project-1", sessionID: "session-1")
        await panel.readFile("/project/review.docx")
        XCTAssertTrue(panel.previewIsLocalFile); XCTAssertNotNil(panel.preview?.base64)
        await panel.readArtifact("artifact-1")
        XCTAssertFalse(panel.previewIsLocalFile)
        await panel.readFile("/project/review.docx"); panel.dismissPreview()
        XCTAssertFalse(panel.previewIsLocalFile)
    }

    @MainActor func testNativeWebKitShowsRevisionMarkupAndRefreshesExistingSurfaceAfterSave() async throws {
        guard ProcessInfo.processInfo.environment["WISP_NATIVE_DOCUMENT_SMOKE"] == "1" else { throw XCTSkip("Opt-in DOCX WebKit rendering") }
        let fixture = try reviewFixture(), state = NativeRichPreviewState()
        let enhanced = NSAccessibility.Attribute(rawValue: "AXEnhancedUserInterface")
        let previousAccessibility = NSApplication.shared.accessibilityAttributeValue(enhanced)
        NSApplication.shared.accessibilitySetValue(true, forAttribute: enhanced)
        defer { NSApplication.shared.accessibilitySetValue(previousAccessibility ?? false, forAttribute: enhanced) }
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 720, height: 680), styleMask: [.titled], backing: .buffered, defer: false)
        defer { window.contentView = nil; window.orderOut(nil) }
        let host = NSHostingView(rootView: NativeRichPreviewSurface(content: try content(fixture), kind: .docx, html: nil, scheme: .light, state: state, quote: nil))
        window.contentView = host; window.orderFront(nil); host.layoutSubtreeIfNeeded()
        try await until { state.ready || state.error != nil }
        XCTAssertNil(state.error)
        let web = try XCTUnwrap(Self.webView(host))
        try await until { (try? await web.evaluateJavaScript("document.querySelector('.rp-docx del')?.textContent === 'Old conclusion' && document.querySelector('.rp-docx ins')?.textContent === 'Revised conclusion'")) as? Bool == true }
        var plain = fixture, file = fixture["content"]
        file["base64"] = .string(try Data(contentsOf: repository().appendingPathComponent("ui-tests/fixtures/office-preview.docx")).base64EncodedString()); plain["content"] = file
        host.rootView = NativeRichPreviewSurface(content: try content(plain), kind: .docx, html: nil, scheme: .light, state: state, quote: nil)
        try await until { (try? await web.evaluateJavaScript("document.querySelector('.rp-docx')?.textContent.includes('Word local fixture') && !document.querySelector('.rp-docx del, .rp-docx ins')")) as? Bool == true }
        XCTAssertTrue(Self.webView(host) === web)
        // Also render the actual SwiftUI controls alongside their WebKit body.
        let fake = ReviewFake(fixture)
        var closed = false
        let reviewHost = NSHostingView(rootView: NativePanelFilePreview(content: try content(fixture), close: { closed = true }, docxTarget: NativeDocxReviewTarget(client: fake, projectID: "project-1", sessionID: "session-1", readOnly: false)).background(Color(NSColor.windowBackgroundColor)))
        window.contentView = reviewHost; window.makeKeyAndOrderFront(nil); reviewHost.layoutSubtreeIfNeeded()
        try await until { guard let web = Self.webView(reviewHost) else { return false }; return (try? await web.evaluateJavaScript("!!document.querySelector('.rp-docx ins')")) as? Bool == true }
        if let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] {
            try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
            let bitmap = try XCTUnwrap(reviewHost.bitmapImageRepForCachingDisplay(in: reviewHost.bounds))
            reviewHost.cacheDisplay(in: reviewHost.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent("docx-review-swiftui.png"))
        }
        // Exercise the actual SwiftUI buttons, not only the model methods.
        let changes = fixture["review"]["changes"].array
        for (identifier, index) in [("docx-accept-deletions", 1), ("docx-accept-rewrites", 0), ("docx-reject-" + changes[2]["id"].string, 2)] {
            try await until("enabled " + identifier) { Self.accessibility(reviewHost, identifier: identifier).map { Self.accessibilityBool($0, "isAccessibilityEnabled") } == true }
            XCTAssertTrue(Self.accessibilityBool(try XCTUnwrap(Self.accessibility(reviewHost, identifier: identifier)), "accessibilityPerformPress"))
            try await until("removed " + identifier) { Self.accessibility(reviewHost, identifier: "docx-accept-" + changes[index]["id"].string) == nil }
        }
        try await until {
            guard let web = Self.webView(reviewHost) else { return false }
            return (try? await web.evaluateJavaScript("document.querySelector('.rp-docx')?.textContent.includes('Word local fixture') && !document.querySelector('.rp-docx del, .rp-docx ins')")) as? Bool == true
        }
        let calls = await fake.recorded(); XCTAssertEqual(calls.count, 4)
        XCTAssertEqual(calls.last?.2["action"], .string("reject"))
        let escape = try XCTUnwrap(NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53))
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        XCTAssertTrue(closed)
    }
    @MainActor private func until(_ label: String = "DOCX review", _ predicate: () async throws -> Bool) async throws {
        let deadline = Date().addingTimeInterval(25)
        while Date() < deadline { if try await predicate() { return }; try await Task.sleep(nanoseconds: 30_000_000) }
        XCTFail("Timed out waiting for " + label)
        throw ProjectBrowserError.invalidResponse
    }
    @MainActor private static func webView(_ root: NSView) -> WKWebView? {
        if let view = root as? WKWebView { return view }
        return root.subviews.lazy.compactMap(webView).first
    }
    // SwiftUI.AccessibilityNode implements public AX selectors without declaring
    // the full NSAccessibilityProtocol. Dispatch those selectors just as AppKit
    // does; casting to the protocol silently removes these nodes from the tree.
    @MainActor private static func accessibility(_ root: NSObject, identifier: String) -> NSObject? {
        if accessibilityValue(root, "accessibilityIdentifier") as? String == identifier { return root }
        for child in accessibilityValue(root, "accessibilityChildren") as? [NSObject] ?? [] {
            if let found = accessibility(child, identifier: identifier) { return found }
        }
        return nil
    }
    @MainActor private static func accessibilityValue(_ object: NSObject, _ name: String) -> Any? {
        let selector = NSSelectorFromString(name)
        guard object.responds(to: selector) else { return nil }
        return object.perform(selector)?.takeUnretainedValue()
    }
    @MainActor private static func accessibilityBool(_ object: NSObject, _ name: String) -> Bool {
        let selector = NSSelectorFromString(name)
        guard object.responds(to: selector) else { return false }
        typealias Method = @convention(c) (AnyObject, Selector) -> Bool
        return unsafeBitCast(object.method(for: selector), to: Method.self)(object, selector)
    }
}
