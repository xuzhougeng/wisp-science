import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private let usageJSON = #"{"input":13446,"output":52,"reasoning":48,"cached":512,"ctx_tokens":13126,"max_context":128000,"context_usage":{"system_prompt":1926,"tool_definitions":1980,"rules":576,"skills":596,"mcp_dynamic_tools":7749,"subagent_definitions":193,"conversation":106}}"#

private actor UsageClient: NativeConversationQuerying {
    let value: ConversationSnapshot
    init(_ value: ConversationSnapshot) { self.value = value }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { value }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue { .null }
}

final class NativeConversationUsageTests: XCTestCase {
    private func item(_ text: String, role: String = "usage") throws -> ConversationItem {
        try JSONDecoder().decode(ConversationItem.self, from: JSONEncoder().encode(SettingsValue.object(["role": .string(role), "text": .string(text)])))
    }
    private func snapshot(_ rows: [ConversationItem]) throws -> ConversationSnapshot {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        var payload = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        payload["items"] = try JSONDecoder().decode(SettingsValue.self, from: JSONEncoder().encode(rows))
        payload["approvals"] = .array([]); payload["running"] = .bool(false)
        return try ConversationSnapshot.decode(payload, projectID: "project-a", sessionID: "session-a")
    }

    func testReportedUsageProducesReadableSummaryWithoutInternalJSON() throws {
        let summary = try XCTUnwrap(NativeConversationUsage(item(usageJSON))).summary
        for count in [13446, 52, 48, 512, 13126, 128000] { XCTAssertTrue(summary.contains(count.formatted())) }
        XCTAssertTrue(summary.contains("10.3%"))
        for key in ["{", "ctx_tokens", "max_context", "context_usage", "mcp_dynamic_tools"] { XCTAssertFalse(summary.contains(key)) }
        XCTAssertNil(NativeConversationUsage(try item(usageJSON, role: "assistant")))
    }
    func testUnknownCapacityAndMalformedNumbersDoNotInventUsage() throws {
        for text in [#"{"ctx_tokens":42}"#, #"{"ctx_tokens":42,"max_context":0}"#] {
            let summary = try XCTUnwrap(NativeConversationUsage(item(text))).summary
            XCTAssertTrue(summary.contains("42")); XCTAssertFalse(summary.contains("%"))
        }
        for text in ["invalid", "[]", "{}", #"{"input":-1}"#, #"{"input":"42"}"#, #"{"input":true}"#, #"{"input":1.5}"#] {
            XCTAssertNil(NativeConversationUsage(try item(text)))
        }
        XCTAssertNotNil(NativeConversationUsage(try item(#"{"input":0,"output":0}"#)))
        XCTAssertTrue(try XCTUnwrap(NativeConversationUsage(item(#"{"ctx_tokens":120,"max_context":100}"#))).summary.contains("120.0%"))
    }
    func testSummaryUsesSavedEnglishPreference() throws {
        let defaults = UserDefaults.standard
        let original = defaults.object(forKey: "nativeSettings.locale")
        defer { if let original { defaults.set(original, forKey: "nativeSettings.locale") } else { defaults.removeObject(forKey: "nativeSettings.locale") } }
        defaults.set("en", forKey: "nativeSettings.locale")
        let summary = try XCTUnwrap(NativeConversationUsage(item(usageJSON))).summary
        for label in ["Input", "Output", "Reasoning", "Cached", "Context Usage"] { XCTAssertTrue(summary.contains(label)) }
    }
    @MainActor func testUsageCannotCaptureExcerptNavigationOrShiftHistoryIndexes() async throws {
        let rows = try [item("hello", role: "assistant"), item(usageJSON), item("question", role: "user"), item("answer", role: "assistant")]
        let model = NativeConversationModel(client: UsageClient(try snapshot(rows)))
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        XCTAssertEqual(model.visibleItems.count, 4)
        XCTAssertEqual(NativeConversationModel.questionItemIndex(0, offset: 0, items: model.visibleItems), 2)
        model.revealExcerpt("answer"); XCTAssertEqual(model.scrollTarget, 3)
        XCTAssertEqual(NativeConversationModel.renderedText(rows[1]), "")
        model.revealExcerpt("ctx_tokens")
        XCTAssertNotNil(model.operationError)
        XCTAssertEqual(model.scrollTarget, 3)
    }
    @MainActor func testUsageNeverRendersAsASelectableAssistantMessage() async throws {
        let greeting = "Hello — I’m wisp-science. How can I help with your software or scientific computing work today?"
        let model = NativeConversationModel(client: UsageClient(try snapshot([item(greeting, role: "assistant"), item(usageJSON)])))
        await model.open(project: "project-a", session: "session-a"); defer { model.pause() }
        let view = NSHostingView(rootView: NativeConversationView(conversation: model)
            .background(WispDesign.color("bg-app", .light)).environment(\.colorScheme, .light))
        view.frame = NSRect(x: 0, y: 0, width: 700, height: 500)
        view.layoutSubtreeIfNeeded()
        func messages(_ view: NSView) -> [String] {
            (view as? NativeMessageTextView).map { [$0.string] } ?? view.subviews.flatMap(messages)
        }
        XCTAssertEqual(messages(view), [greeting + "\n"])
        if let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] {
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent("conversation-usage.png"))
        }
    }
    @MainActor func testSummaryWrapsWithinNarrowWidthAndRendersInBothThemes() throws {
        let usage = try XCTUnwrap(NativeConversationUsage(item(usageJSON)))
        for scheme in [ColorScheme.light, .dark] {
            for width in [280.0, 700.0] {
                let view = NSHostingView(rootView: NativeConversationUsageView(usage: usage)
                    .background(WispDesign.color("bg-app", scheme)).environment(\.colorScheme, scheme)
                    .frame(width: width).fixedSize(horizontal: false, vertical: true))
                let size = view.fittingSize
                XCTAssertLessThanOrEqual(size.width, width + 1)
                XCTAssertGreaterThan(size.height, 12)
                XCTAssertLessThan(size.height, 100)
                view.frame = NSRect(origin: .zero, size: size); view.layoutSubtreeIfNeeded()
                if let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] {
                    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
                    view.cacheDisplay(in: view.bounds, to: bitmap)
                    try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent("usage-\(scheme == .dark ? "dark" : "light")-\(Int(width)).png"))
                }
            }
        }
    }
}
