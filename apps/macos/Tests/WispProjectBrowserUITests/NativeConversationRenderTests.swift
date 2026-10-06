import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor RenderConversationClient: NativeConversationQuerying {
    let value: ConversationSnapshot
    init(_ value: ConversationSnapshot) { self.value = value }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { value }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        .array([.object(["id": .string("model-a"), "label": .string("Test chat model")])])
    }
}

/// Opt-in, offline visual smoke. It renders the real view without launching a
/// host, accessing a user database, or calling a model provider.
final class NativeConversationRenderTests: XCTestCase {
    @MainActor func testRenderPlanAndAcpSettingsInBothLocalesAndSchemes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render native conversation fixtures") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        for acp in [false, true] {
            for locale in ["zh", "en"] {
                for scheme in [ColorScheme.light, .dark] {
                    UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
                    var value = fixture
                    value["running"] = .bool(false); value["approvals"] = .array([]); value["plan_mode"] = acp ? .null : .bool(true)
                    value["items"] = .array([.object(["role": .string("user"), "text": .string("Plan the sample analysis")]), .object([
                        "role": .string("plan"), "text": .string("raw plan"), "proposal": .object(["source": .string(acp ? "acp" : "native"), "entries": .array([
                            .object(["content": .string("Inspect **samples** and verify metadata."), "status": .string("pending"), "priority": .string("high")]),
                            .object(["content": .string("Summarize results with the QC evidence."), "status": .string("pending"), "priority": .string("medium")])])])])])
                    if acp {
                        value["model_id"] = .string("acp:qa"); value["acp_agent_id"] = .string("qa")
                        value["acp_state"] = try JSONDecoder().decode(SettingsValue.self, from: Data(#"""
                        {"frameId":"session-a","modes":{"currentModeId":"plan","availableModes":[{"id":"plan","name":"Plan"},{"id":"agent","name":"Agent"}]},
                         "configOptions":[{"id":"thinking","name":"Thinking","type":"boolean","currentValue":true}]}
                        """#.utf8))
                    }
                    let snapshot = try ConversationSnapshot.decode(value, projectID: "project-a", sessionID: "session-a")
                    let model = NativeConversationModel(client: RenderConversationClient(snapshot)); await model.open(project: "project-a", session: "session-a")
                    model.pause(); model.draft = "Only execute the first step."
                    let view = NSHostingView(rootView: NativeConversationView(conversation: model, projectID: "project-a", sessionID: "session-a")
                        .background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme))
                        .tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                    view.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
                    view.frame = NSRect(x: 0, y: 0, width: 419, height: 680); view.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds)); view.cacheDisplay(in: view.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("plan-\(acp ? "acp" : "native")-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }
    @MainActor func testRenderConversationAtDesktopAndNarrowSizes() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else {
            throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render native conversation fixtures")
        }
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-conversations/v1/snapshot.json")))
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        for (name, width, height, scheme) in [("desktop", 859.0, 760.0, ColorScheme.light), ("narrow", 419.0, 538.0, ColorScheme.light), ("dark", 859.0, 760.0, ColorScheme.dark), ("long-preview", 419.0, 538.0, ColorScheme.light)] {
            var payload = fixture
            if name != "long-preview" {
                payload["running"] = .bool(false)
                payload["plan_mode"] = .bool(true)
                payload["fast_mode"] = .object(["enabled": .bool(true), "inherited": .bool(false)])
            }
            if name == "long-preview" {
                var approvals = payload["approvals"].array
                approvals[0]["preview"] = .string((1...30).map { "echo sample-\($0)" }.joined(separator: "\n"))
                payload["approvals"] = .array(approvals)
            }
            let value = try ConversationSnapshot.decode(payload, projectID: "project-a", sessionID: "session-a")
            let model = NativeConversationModel(client: RenderConversationClient(value))
            await model.open(project: value.project_id, session: value.session_id)
            model.pause(); model.draft = "请继续检查样本，并总结质量控制结果。"
            let view = NSHostingView(rootView: NativeConversationView(conversation: model)
                .background(WispDesign.color("bg-app", scheme))
                .foregroundStyle(WispDesign.color("text", scheme))
                .tint(WispDesign.color("clay", scheme))
                .environment(\.colorScheme, scheme))
            view.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
            view.frame = NSRect(x: 0, y: 0, width: width, height: height)
            view.layoutSubtreeIfNeeded()
            guard let bitmap = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return XCTFail("No native bitmap") }
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            XCTAssertGreaterThan(data.count, 1000)
            try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("conversation-\(name).png"))
        }
    }
}
