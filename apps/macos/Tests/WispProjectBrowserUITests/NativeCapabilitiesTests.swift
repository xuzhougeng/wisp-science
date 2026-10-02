import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeCapabilitiesTests: XCTestCase {
    func testSkillCountsKeepDisabledBundledSkillsOut() {
        let counts = NativeCapabilityRead.skillCounts([
            CapabilitySkill(scope: "bundled", enabled: true),
            CapabilitySkill(scope: "bundled", enabled: false),
            CapabilitySkill(scope: "project", enabled: true),
            CapabilitySkill(scope: "global", enabled: true),
            CapabilitySkill(scope: "plugin", enabled: true),
        ])
        XCTAssertEqual(counts.bundled, 1)
        XCTAssertEqual(counts.project, 3)
    }

    @MainActor func testReloadUsesExistingSettingsReadsAndDoesNotProbeOrRetry() async {
        let host = CapabilityTransport()
        let capabilities = NativeCapabilitiesModel()
        capabilities.open(projectID: "research-1")
        await capabilities.reload(host)
        let commands = await host.commands()
        XCTAssertEqual(commands.map(\.command), NativeCapabilityRead.commands)
        XCTAssertTrue(commands.allSatisfy { $0.projectID == "research-1" })
        XCTAssertFalse(commands.contains { $0.command == NativeCapabilityRead.probe })
        XCTAssertEqual(commands.last?.args["project_id"], .string("research-1"))
        XCTAssertEqual(capabilities.summary.version, "1.14.0")
        XCTAssertEqual(capabilities.summary.workspace, "/tmp/research")
        XCTAssertEqual(capabilities.summary.bundledSkills, 1)
        XCTAssertEqual(capabilities.summary.projectSkills, 1)
        XCTAssertEqual(capabilities.summary.mcpConnections, 1)
        XCTAssertEqual(capabilities.summary.memoryFiles, 2)
        XCTAssertEqual(capabilities.summary.errors, ["skill index"])
        let before = commands.count
        await host.setMode("lost")
        await capabilities.reload(host)
        let after = await host.callCount()
        XCTAssertEqual(after, before + 1)
        XCTAssertTrue(capabilities.error?.contains("不会自动重试") == true)
        XCTAssertEqual(capabilities.summary.bundledSkills, 1)
        await Task.yield()
        let still = await host.callCount()
        XCTAssertEqual(still, after)
    }

    @MainActor func testSectionOpensExistingSettingsAndALateReadDoesNotOpenAProject() async {
        let list = CapabilityProjectList()
        let host = CapabilityTransport()
        let model = ProjectBrowserModel(client: list, databaseURL: URL(fileURLWithPath: "/unused/capabilities.sqlite"), projectTransport: host)
        model.capabilities.open(projectID: "research-1")
        await model.capabilities.reload(host)
        model.capabilities.openSettings("skills", in: model)
        XCTAssertTrue(model.settingsPresented)
        XCTAssertEqual(model.settingsSectionID, "skills")
        XCTAssertNil(model.projectSettingsID)
        XCTAssertFalse(model.capabilities.presented)
        let commands = await host.commands()
        XCTAssertFalse(commands.contains { $0.command == NativeCapabilityRead.probe })
        model.goHome()
        model.capabilities.open(projectID: "research-1")
        await host.setMode("late")
        let before = await host.callCount()
        let task = Task { await model.capabilities.reload(host) }
        var started = false
        for _ in 0..<200 {
            if await host.callCount() > before { started = true; break }
            await Task.yield()
        }
        XCTAssertTrue(started)
        model.goHome()
        await task.value
        XCTAssertNil(model.activeProjectID)
        XCTAssertFalse(model.capabilities.presented)
        XCTAssertEqual(model.capabilities.summary.version, "1.14.0")
    }

    @MainActor func testImmediateEscapeClosesOnlyTheCapabilitySummary() {
        _ = NSApplication.shared
        let capabilities = NativeCapabilitiesModel()
        capabilities.open(projectID: "research-1")
        var parent = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 420, height: 280), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect)
        window.contentView = root
        let parentView = NSView(frame: .zero)
        root.addSubview(parentView)
        let owner = NativeSettingsEscape.Coordinator(enabled: true) { parent = false }
        owner.view = parentView
        owner.install()
        defer { owner.remove() }
        let host = NSHostingView(rootView: CapabilityEscapeSheet(capabilities: capabilities))
        root.addSubview(host)
        host.frame = root.bounds
        host.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(capabilities.presented)
        XCTAssertTrue(parent)
        XCTAssertEqual(capabilities.projectID, "research-1")
        XCTAssertTrue(window.firstResponder === focus)
    }
}

private struct CapabilityEscapeSheet: View {
    @ObservedObject var capabilities: NativeCapabilitiesModel
    var body: some View {
        Text("能力")
            .background(NativeSettingsEscape(enabled: !capabilities.busy) { capabilities.dismiss() })
    }
}

private actor CapabilityTransport: NativeSettingsQuerying {
    private var recorded: [(command: String, args: [String: SettingsValue], projectID: String?)] = []
    private var mode = "ok"
    func setMode(_ mode: String) { self.mode = mode }
    func callCount() -> Int { recorded.count }
    func commands() -> [(command: String, args: [String: SettingsValue], projectID: String?)] { recorded }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        recorded.append((command, args, projectID))
        if mode == "lost" { throw ProjectBrowserError.service("connection reset") }
        if mode == "late" {
            await Task.yield()
            if command == "get_bootstrap_status" {
                return .object(["app_version": .string("late"), "workspace": .string("/tmp/late"), "errors": .array([])])
            }
        }
        switch command {
        case "get_bootstrap_status":
            return .object([
                "app_version": .string("1.14.0"),
                "workspace": .string("/tmp/research"),
                "skills_loaded": .integer(2),
                "mcp_catalog": .integer(1),
                "errors": .array([.string("skill index")]),
            ])
        case "list_skills":
            return .array([
                .object(["scope": .string("bundled"), "enabled": .bool(true)]),
                .object(["scope": .string("bundled"), "enabled": .bool(false)]),
                .object(["scope": .string("project"), "enabled": .bool(true)]),
            ])
        case "list_mcp_connections":
            return .object(["connections": .array([
                .object(["enabled": .bool(true)]),
                .object(["enabled": .bool(false)]),
            ])])
        case "get_memory_view":
            return .object(["files": .array([.object(["name": .string("a")]), .object(["name": .string("b")])])])
        default:
            throw ProjectBrowserError.service("unexpected \(command)")
        }
    }
}

private actor CapabilityProjectList: ProjectBrowserQuerying {
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot {
        ProjectListSnapshot(projects: [], activitySource: "persisted_only")
    }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage {
        TranscriptPage(messages: [], nextBeforeSeq: nil)
    }
}
