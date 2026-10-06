import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor WorkspaceSearchFake: NativeSettingsQuerying {
    var calls: [(String, String?)] = []
    var held: CheckedContinuation<SettingsValue, Never>?
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, projectID))
        let query = args["query"]!.string
        if query == "slow" { return await withCheckedContinuation { held = $0 } }
        if query == "offline" { throw ProjectBrowserError.service("offline") }
        return response(query, projectID)
    }
    func response(_ query: String, _ project: String?) -> SettingsValue {
        .object(["schema": .string(NativeSearchResponse.schemaID), "query": .string(query), "preferred_project_id": project.map(SettingsValue.string) ?? .null,
                 "items": .array([.object(["kind": .string("session"), "id": .string("old-session"), "project_id": .string("other-project"), "project_name": .string("Other"), "title": .string("Match: " + query), "detail": .string("Other"), "session_id": .string("old-session")])])])
    }
    func waiting() -> Bool { held != nil }
    func finish(_ project: String?) { held?.resume(returning: response("slow", project)); held = nil }
    func recorded() -> [(String, String?)] { calls }
}
private struct EmptySearchBrowser: ProjectBrowserQuerying {
    func listProjects(databaseURL: URL) async throws -> ProjectListSnapshot { ProjectListSnapshot(projects: [], activitySource: "persisted_only") }
    func listSessions(databaseURL: URL, projectID: String?) async throws -> [BrowserSession] { [] }
    func transcript(databaseURL: URL, projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> TranscriptPage { throw ProjectBrowserError.invalidResponse }
}

final class NativeWorkspaceSearchTests: XCTestCase {
    @MainActor func testRenderSearchAndReferenceShortcutInBothLocalesAndSchemesAtNarrowWidth() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render search") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let fake = WorkspaceSearchFake(), browser = ProjectBrowserModel(client: EmptySearchBrowser(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: WorkspaceSearchFake())
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                let search = NativeSearchModel(client: fake, projectID: nil); await search.search("", debounce: 0)
                let host = NSHostingView(rootView: ProjectSearchSheet(model: browser, searchModel: search, close: {}).frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme)).environment(\.colorScheme, scheme))
                host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua); host.frame = NSRect(x: 0, y: 0, width: 419, height: 460); host.layoutSubtreeIfNeeded()
                let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("workspace-search-reference-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
            }
        }
    }
    @MainActor func testCommandPaletteUsesScopeAndEnglishAliases() {
        XCTAssertEqual(NativeSearchCommand.matching(">terminal", project: true, session: true).map(\.id), ["terminal"])
        XCTAssertTrue(NativeSearchCommand.matching(">terminal", project: true, session: false).isEmpty)
        XCTAssertFalse(NativeSearchCommand.matching(">", project: false, session: false).contains { $0.project || $0.session })
        XCTAssertEqual(NativeSearchCommand.matching("preferences", project: false, session: false).map(\.id), ["settings"])
        XCTAssertEqual(NativeSearchCommand.matching(">论文", project: true, session: false).map(\.id), ["publication"])
        XCTAssertEqual(Set(NativeSearchCommand.all.map(\.icon)).count, NativeSearchCommand.all.count)
    }
    @MainActor func testSearchImmediateEscapeClosesTopSurfaceBeforeParentWithoutChangingFocus() {
        _ = NSApplication.shared
        let browser = ProjectBrowserModel(client: EmptySearchBrowser(), databaseURL: URL(fileURLWithPath: "/unused"), projectTransport: WorkspaceSearchFake())
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 600, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let container = NSView(frame: window.contentView!.bounds); window.contentView = container
        var parentClosed = 0, searchClosed = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }; parent.view = container; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: ProjectSearchSheet(model: browser, close: { searchClosed += 1 }))
        hosted.frame = container.bounds; container.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertEqual(searchClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertTrue(window.firstResponder === focus)
        hosted.removeFromSuperview()
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
    }
    func testSharedFixtureAndStrictResultOwnership() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let fixture = try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-search/v1/search.json")))
        var value = fixture["result"]
        let result = try NativeSearchResponse.decode(value, query: "needle", projectID: "research-1")
        XCTAssertEqual(result.items.first?.objectID, "session-1")
        XCTAssertThrowsError(try NativeSearchResponse.decode(value, query: "changed", projectID: "research-1"))
        XCTAssertThrowsError(try NativeSearchResponse.decode(value, query: "needle", projectID: nil))
        value["items"] = .array(value["items"].array + value["items"].array)
        XCTAssertThrowsError(try NativeSearchResponse.decode(value, query: "needle", projectID: "research-1"))
        var item = fixture["result"]["items"].array[0]; item["session_id"] = .string("wrong-owner")
        value["items"] = .array([item])
        XCTAssertThrowsError(try NativeSearchResponse.decode(value, query: "needle", projectID: "research-1"))
        item["kind"] = .string("artifact"); item["session_id"] = .null; value["items"] = .array([item])
        XCTAssertThrowsError(try NativeSearchResponse.decode(value, query: "needle", projectID: "research-1"))
    }
    @MainActor func testSearchReadsPersistedHistoryWithoutDependingOnRecentSessions() async {
        let client = WorkspaceSearchFake(); let model = NativeSearchModel(client: client, projectID: nil)
        await model.search("body needle", debounce: 0)
        XCTAssertEqual(model.items.first?.objectID, "old-session")
        XCTAssertEqual(model.items.first?.project_id, "other-project")
        let calls = await client.recorded()
        XCTAssertEqual(calls.map(\.0), ["native_workspace_search"]); XCTAssertNil(calls.first?.1)
        await model.search("offline", debounce: 0)
        XCTAssertTrue(model.items.isEmpty); XCTAssertNotNil(model.error); XCTAssertFalse(model.busy)
        await model.search("body needle", debounce: 0)
        XCTAssertNil(model.error); XCTAssertEqual(model.items.count, 1)
    }
    @MainActor func testEditedAndClosedSearchesDiscardLateResultsAndEnforceByteLimit() async {
        let client = WorkspaceSearchFake(); let model = NativeSearchModel(client: client, projectID: "preferred")
        let old = Task { await model.search("slow", debounce: 0) }
        while !(await client.waiting()) { await Task.yield() }
        await model.search("new", debounce: 0)
        await client.finish("preferred"); await old.value
        XCTAssertEqual(model.items.count, 1); XCTAssertEqual(model.items.first?.title, "Match: new"); XCTAssertNil(model.error)
        let late = Task { await model.search("slow", debounce: 0) }
        while !(await client.waiting()) { await Task.yield() }
        model.invalidate(); await client.finish("preferred"); await late.value
        XCTAssertTrue(model.items.isEmpty); XCTAssertFalse(model.busy)
        await model.search(String(repeating: "中", count: 171), debounce: 0)
        XCTAssertNotNil(model.error)
        let calls = await client.recorded(); XCTAssertEqual(calls.count, 3)
    }
}
