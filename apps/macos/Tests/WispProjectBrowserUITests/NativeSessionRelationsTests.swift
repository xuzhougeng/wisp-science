import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private func relationshipFixture(_ name: String = "project-browser/v1/relationships") throws -> SettingsValue {
    var root = URL(fileURLWithPath: #filePath); for _ in 0..<5 { root.deleteLastPathComponent() }
    return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/\(name).json")))
}
private actor RelationshipClient: NativeConversationQuerying {
    var sends = 0
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot {
        var value = try relationshipFixture("native-conversations/v1/snapshot")
        value["project_id"] = .string(projectID); value["session_id"] = .string(sessionID); value["running"] = .bool(false)
        return try ConversationSnapshot.decode(value, projectID: projectID, sessionID: sessionID)
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        if command == "native_conversation_send" { sends += 1 }
        return .array([])
    }
}
final class NativeSessionRelationsTests: XCTestCase {
    private func sessions() throws -> [BrowserSession] {
        try JSONDecoder().decode([BrowserSession].self, from: JSONEncoder().encode(relationshipFixture()["sessions"]))
    }
    func testSharedMetadataFindsParentsSiblingsChildrenAndStates() throws {
        let rows = try sessions(), main = NativeSessionRelations(selected: rows[0], sessions: rows)
        XCTAssertNil(main.parent); XCTAssertEqual(main.branches.map(\.id), ["active", "merged"])
        XCTAssertEqual(main.subagents.map(\.id), ["child"]); XCTAssertFalse(main.canOpen("nested"))
        let branch = NativeSessionRelations(selected: rows[1], sessions: rows)
        XCTAssertEqual(branch.parent?.id, "main"); XCTAssertEqual(branch.siblings.map(\.id), ["merged"])
        XCTAssertEqual(branch.branches.map(\.id), ["nested"])
        XCTAssertTrue(branch.canOpen("main")); XCTAssertFalse(branch.canOpen("active"))
        XCTAssertEqual(NativeSessionRelations.stateLabel(rows[2]), "已合并分支")
        XCTAssertEqual(NativeSessionRelations.stateLabel(rows[3]), "来源已失效的分支")
        XCTAssertNil(NativeSessionRelations(selected: rows[3], sessions: rows).parent)
        XCTAssertEqual(NativeSessionRelations.stateLabel(rows[4]), "子代理会话")
    }
    func testMissingForeignDeletedDuplicateAndSelfLinksCannotInventNavigation() throws {
        let rows = try sessions(), selected = rows[1]
        let foreign = BrowserSession(id: "main", projectID: "foreign", title: "Foreign", ts: 1, status: "complete")
        let deleted = BrowserSession(id: "main", projectID: selected.projectID, title: "Deleted", ts: 1, status: "deleted")
        let missing = NativeSessionRelations(selected: selected, sessions: [selected, foreign, deleted])
        XCTAssertTrue(missing.missingParent); XCTAssertNil(missing.parent); XCTAssertFalse(missing.canOpen("main"))
        let absent = NativeSessionRelations(selected: selected, sessions: [rows[0]])
        XCTAssertNil(absent.source); XCTAssertFalse(absent.canOpen("main"))
        let selfLink = BrowserSession(id: "self", projectID: selected.projectID, title: "Self", ts: 1, status: "complete", branchedFrom: "self", branchState: "active")
        let cycle = NativeSessionRelations(selected: selfLink, sessions: [selfLink, selfLink])
        XCTAssertTrue(cycle.missingParent); XCTAssertNil(cycle.parent); XCTAssertFalse(cycle.canOpen("self"))
        let duplicates = NativeSessionRelations(selected: rows[0], sessions: rows + rows)
        XCTAssertEqual(duplicates.branches.count, 2)
        let invalidated = BrowserSession(id: "invalidated", projectID: selected.projectID, title: "Invalidated", ts: 1, status: "complete", branchedFrom: "main", branchState: "orphaned")
        let staleLink = NativeSessionRelations(selected: invalidated, sessions: rows + [invalidated])
        XCTAssertNil(staleLink.parent); XCTAssertTrue(staleLink.siblings.isEmpty); XCTAssertFalse(staleLink.canOpen("main"))
    }
    @MainActor func testRelatedNavigationKeepsBothDraftsWithoutSending() async throws {
        let rows = try sessions(), client = RelationshipClient(), conversation = NativeConversationModel(client: client)
        let main = NativeSessionRelations(selected: rows[0], sessions: rows)
        await conversation.open(project: rows[0].projectID, session: rows[0].id); conversation.draft = "Unsent main question"
        XCTAssertTrue(main.canOpen("active")); await conversation.open(project: rows[0].projectID, session: "active")
        conversation.draft = "Unsent branch question"
        let branch = NativeSessionRelations(selected: rows[1], sessions: rows)
        XCTAssertTrue(branch.canOpen("main")); await conversation.open(project: rows[0].projectID, session: "main")
        XCTAssertEqual(conversation.draft, "Unsent main question")
        await conversation.open(project: rows[0].projectID, session: "active"); XCTAssertEqual(conversation.draft, "Unsent branch question")
        conversation.pause(); let sends = await client.sends; XCTAssertEqual(sends, 0)
    }
    @MainActor func testImmediateEscapeClosesRelationshipsBeforeItsParentWithoutMovingFocus() throws {
        _ = NSApplication.shared; let rows = try sessions(); var parentOpen = true, childOpen = true
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 419, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let root = NSView(frame: window.contentLayoutRect); window.contentView = root
        let parent = NSView(frame: .zero); root.addSubview(parent)
        let owner = NativeSettingsEscape.Coordinator(enabled: true) { parentOpen = false }; owner.view = parent; owner.install(); defer { owner.remove() }
        let host = NSHostingView(rootView: NativeSessionRelationsSheet(selected: rows[1], sessions: rows, close: { childOpen = false }, open: { _ in XCTFail("Escape must not navigate") }))
        root.addSubview(host); host.frame = root.bounds; host.layoutSubtreeIfNeeded(); let focus = window.firstResponder
        let event = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(event, keyWindow: window, modalWindow: nil))
        XCTAssertFalse(childOpen); XCTAssertTrue(parentOpen); XCTAssertTrue(window.firstResponder === focus)
    }
    @MainActor func testRelationshipsRenderAllScopesAndStatesInBothLocalesAndSchemes() throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Set WISP_NATIVE_SNAPSHOT_DIR to render relationships") }
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") } }
        let rows = try sessions()
        for locale in ["zh", "en"] {
            UserDefaults.standard.set(locale, forKey: "nativeSettings.locale")
            for scheme in [ColorScheme.light, .dark] {
                for selected in [rows[0], rows[1], rows[3]] {
                    let host = NSHostingView(rootView: NativeSessionRelationsSheet(selected: selected, sessions: rows, close: {}, open: { _ in }).frame(maxWidth: .infinity, maxHeight: .infinity).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).environment(\.colorScheme, scheme))
                    host.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua); host.frame = NSRect(x: 0, y: 0, width: 419, height: 520); host.layoutSubtreeIfNeeded()
                    let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds)); host.cacheDisplay(in: host.bounds, to: bitmap)
                    let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:])); XCTAssertGreaterThan(data.count, 1000)
                    try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("session-relations-\(selected.id)-\(locale)-\(scheme == .dark ? "dark" : "light").png"))
                }
            }
        }
    }
}
