import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor ParitySettingsClient: NativeSettingsQuerying {
    var state = "idle"
    var lost = false
    var calls: [(String, [String: SettingsValue])] = []
    func loseNextWrite() { lost = true }
    func recorded() -> [(String, [String: SettingsValue])] { calls }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, args))
        switch command {
        case "get_toolchain_status": return .object(["state": .string(state), "installed": .array([]), "root": .string("/tmp/fake-tools")])
        case "install_toolchain", "cancel_toolchain_install":
            state = command == "install_toolchain" ? "running" : "cancelled"
            if lost { lost = false; throw ProjectBrowserError.service("lost reply") }
            return .null
        case "list_workflow_templates", "list_skills", "list_models": return .array([])
        case "channels_status": return .object(["remote": .object(["enabled": .bool(false), "relay_url": .string("https://example.test"), "state": .string("disabled")])])
        case "lab_status": return .object(["state": .string("none")])
        case "get_settings": return .object(["notification_sound_enabled": .bool(true), "notification_sound": .string("system")])
        default: return .null
        }
    }
}

private actor ParityExplorationClient: NativeConversationQuerying {
    var wrong = false
    var lost = false
    var writes: [(String, [String: SettingsValue])] = []
    func setup(wrong: Bool = false, lost: Bool = false) { self.wrong = wrong; self.lost = lost }
    func recorded() -> [(String, [String: SettingsValue])] { writes }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        let row: SettingsValue = .object(["source_frame_id": .string("source"), "exploration": .object(["id": .string("e"), "frame_id": .string("candidate"), "status": .string("failed"), "name": .string("Recovery")])])
        if command == "native_conversation_explorations" { return .object(["project_id": .string(wrong ? "other" : projectID), "explorations": .array([row])]) }
        if command == "native_conversation_exploration_preview" { return .object(["project_id": .string(projectID), "session_id": .string(wrong ? "foreign" : "candidate"), "preview": .object(["exploration": row["exploration"]])]) }
        writes.append((command, args))
        if lost { throw ProjectBrowserError.service("lost reply") }
        return .object(["project_id": .string(projectID), "session_id": wrong ? .string("foreign") : args["session_id"]!, "exploration_id": args["exploration_id"] ?? .string(""), "action": .string(String(command.dropFirst("native_conversation_exploration_".count))), "completed": .bool(true)])
    }
}

final class NativeParityCompletionTests: XCTestCase {
    @MainActor func testToolchainLostResponseRequiresReadAndAcknowledgementWithoutReplay() async {
        let client = ParitySettingsClient()
        let toolchain = NativeToolchainModel(client: client)
        await toolchain.refresh()
        await toolchain.install(["not-a-tool"])
        await client.loseNextWrite(); await toolchain.install(["python", "uv"])
        XCTAssertTrue(toolchain.uncertain); XCTAssertNil(toolchain.status)
        await toolchain.install(["python"])
        toolchain.acknowledge(); XCTAssertTrue(toolchain.uncertain)
        await toolchain.refresh(); XCTAssertTrue(toolchain.running); XCTAssertTrue(toolchain.uncertain)
        toolchain.acknowledge(); await toolchain.cancel()
        XCTAssertFalse(toolchain.running)
        XCTAssertEqual(toolchain.status?["state"], .string("cancelled"))
        let calls = await client.recorded()
        XCTAssertEqual(calls.filter { $0.0 == "install_toolchain" }.count, 1)
        XCTAssertEqual(calls.first { $0.0 == "install_toolchain" }?.1["tools"], .array([.string("python"), .string("uv")]))
    }
    @MainActor func testExplorationRejectsForeignPreviewAndUnconfirmedMutation() async {
        let client = ParityExplorationClient()
        let exploration = NativeExplorationsModel(client: client, project: "p")
        await exploration.load(); XCTAssertTrue(exploration.canWrite)
        let row = exploration.rows[0]
        XCTAssertEqual(row["exploration"]["status"], .string("failed"))
        await client.setup(wrong: true); await exploration.inspect(row)
        XCTAssertNil(exploration.preview)
        let success = await exploration.mutate("discard", args: exploration.arguments(row))
        XCTAssertFalse(success); XCTAssertTrue(exploration.uncertain)
        await exploration.mutate("discard", args: exploration.arguments(row))
        let calls = await client.recorded(); XCTAssertEqual(calls.count, 1)
        await client.setup(); await exploration.load(); exploration.acknowledge()
        let abandoned = await exploration.mutate("abandon", args: ["session_id": .string("source")])
        XCTAssertTrue(abandoned)
    }
    func testUsageModesExcludeFutureCellsAndPreserveTotals() {
        let rows: [SettingsValue] = (0..<14).map { day in .object(["date": .string("day-\(day)"), "tokens": .integer(Int64(day + 1)), "future": .bool(day >= 10)]) }
        let daily = NativeUsageDashboardData.activity(rows, mode: "daily")
        let weekly = NativeUsageDashboardData.activity(rows, mode: "weekly")
        let cumulative = NativeUsageDashboardData.activity(rows, mode: "cumulative")
        XCTAssertEqual(daily.count, 14); XCTAssertEqual(weekly.map { $0.1 }, [28, 27])
        XCTAssertEqual(cumulative[9].1, 55)
        XCTAssertTrue(daily.suffix(4).allSatisfy { $0.1 == 0 && $0.2 })
        XCTAssertEqual(NativeUsageDashboardData.total([.object(["input": .integer(10)]), .object(["input": .integer(-1)])], field: "input"), 10)
    }
    @MainActor func testSoundDraftAndRemoteCredentialsNeedExplicitSaveAndCancelClearsDraft() async {
        let client = ParitySettingsClient()
        let model = NativeSettingsModel(client: client, projectID: "p")
        await model.load()
        model.binding("get_settings", "notification_sound_enabled").wrappedValue = .bool(false)
        model.binding("get_settings", "notification_sound").wrappedValue = .string("chosen")
        XCTAssertTrue(model.hasUnsavedChanges)
        await model.saveSettings()
        let calls = await client.recorded()
        let saved = calls.first { $0.0 == "set_settings" }?.1["settings"]
        XCTAssertEqual(saved?["notification_sound_enabled"], .bool(false))
        XCTAssertEqual(saved?["notification_sound"], .string("chosen"))
        XCTAssertFalse(calls.contains { $0.0 == "preview_notification_sound" || $0.0 == "install_toolchain" })
        model.section = .channels; await model.load()
        model.remoteAccessToken = "fake-token"
        model.values["channels_status"]?["remote"]["relay_url"] = .string("https://changed.test")
        await model.load()
        XCTAssertTrue(model.hasUnsavedChanges)
        XCTAssertEqual(model.values["channels_status"]?["remote"]["relay_url"], .string("https://changed.test"))
        model.discardDrafts()
        XCTAssertEqual(model.remoteAccessToken, "")
        XCTAssertEqual(model.values["channels_status"]?["remote"]["relay_url"], .string("https://example.test"))
    }
    @MainActor func testRemoteLinkRejectsNonWebSchemesAndProducesQR() {
        XCTAssertNil(NativeRemoteAccessSettings.link(.object(["link": .string("file:///etc/hosts")])))
        XCTAssertNil(NativeRemoteAccessSettings.link(.object(["link": .string("javascript:alert(1)")])))
        let value: SettingsValue = .object(["link": .string("https://example.test/#private-code")])
        XCTAssertEqual(NativeRemoteAccessSettings.link(value)?.fragment, "private-code")
        XCTAssertNotNil(NativeRemoteAccessSettings.qr(value["link"].string))
    }
    @MainActor func testImmediateEscapeClosesExplorationsBeforeParentWithoutMovingFocus() {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 680), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        var parentClosed = 0, childClosed = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }; parent.view = container; parent.install(); defer { parent.remove() }
        let host = NSHostingView(rootView: NativeExplorationsView(client: ParityExplorationClient(), project: "p", session: "s", turns: [], close: { childClosed += 1 }, open: { _ in }))
        host.frame = container.bounds; container.addSubview(host); host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(childClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertTrue(window.firstResponder === focus)
    }
    @MainActor func testDashboardAndWorkflowStudioRenderWithoutLiveServices() async throws {
        let model = NativeSettingsModel(client: ParitySettingsClient(), projectID: "p")
        var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
        let template = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-workflows/v1/template.json")))
        model.values["list_workflow_templates"] = .array([template])
        let overview: SettingsValue = .object([
            "workspaces": .array([.object(["input": .integer(12000), "output": .integer(3000), "reasoning": .integer(1400), "cached": .integer(8000)])]),
            "days": .array((0..<91).map { day in .object(["date": .string("day-\(day)"), "tokens": .integer(Int64((day % 7) * 200)), "future": .bool(day > 87)]) }),
            "models": .array([.object(["model": .string("research-model"), "tokens": .integer(11000)]), .object(["model": .string("review-model"), "tokens": .integer(4000)])]),
            "tools": .array([.object(["kind": .string("skill"), "name": .string("literature-review"), "calls": .integer(12)])])
        ])
        for (name, view) in [("usage", AnyView(ScrollView { NativeUsageDashboard(overview: overview, profiles: []) })), ("workflow-studio", AnyView(NativeWorkflowStudio(model: model, back: {})))] {
            let host = NSHostingView(rootView: view.padding(20).background(WispDesign.color("bg-app", .light)).foregroundStyle(WispDesign.color("text", .light)).environment(\.colorScheme, .light))
            host.frame = NSRect(x: 0, y: 0, width: 1280, height: 900); host.layoutSubtreeIfNeeded()
            await Task.yield(); host.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(png.count, 1000)
            if let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] { try png.write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png")) }
        }
    }
}
