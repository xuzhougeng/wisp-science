import XCTest
import UIKit
import WispRemote
@testable import WispScience

@MainActor final class OfflineResearchAppTests: XCTestCase {
    private func directory() -> URL { FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString) }
    private func connection() -> RemoteConnection {
        RemoteConnection(socketFactory: { _ in fatalError("Offline research tests must never create a socket") })
    }
    private func image() -> Data {
        UIGraphicsImageRenderer(size: CGSize(width: 40, height: 60)).jpegData(withCompressionQuality: 1) { context in
            UIColor.white.setFill(); context.fill(CGRect(x: 0, y: 0, width: 40, height: 60))
        }
    }
    private func configuration(_ host: String) throws -> RemoteConfiguration {
        try RemoteConfiguration(address: "https://\(host).example.test", code: "000102030405060708090a0b0c0d0e0f")
    }
    private func snapshot(_ route: ConversationRoute, readOnly: Bool = false) -> RemoteValue {
        .object(["schema": .string("wisp.native-conversations.v1"), "epoch": .string("test"), "sequence": .integer(1),
                 "project_id": .string(route.project), "session_id": .string(route.session), "items": .array([]),
                 "approvals": .array([]), "running": .bool(false), "stopping": .bool(false), "read_only": .bool(readOnly)])
    }

    func testChecklistLocalRestartScopesAndOldHostLeavePendingData() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = ChecklistModel(directory: dir)
        XCTAssertTrue(model.edit(fields: ChecklistFields(title: "sample A", day: "2026-10-14")))
        let restored = ChecklistModel(directory: dir)
        XCTAssertEqual(restored.entries.count, 1)
        let c = connection()
        c.useLocalService(name: "one", commands: []) { _, _, _ in XCTFail("Old host must not receive sync"); return .null }
        restored.configure(configuration: try configuration("one"), connection: c, demo: false)
        let one = restored.scope
        XCTAssertEqual(restored.entries.count, 1)
        await restored.sync(connection: c)
        XCTAssertEqual(restored.pendingCount, 1); XCTAssertNotNil(restored.error)
        c.useLocalService(name: "two", commands: []) { _, _, _ in XCTFail(); return .null }
        restored.configure(configuration: try configuration("two"), connection: c, demo: false)
        XCTAssertTrue(restored.entries.isEmpty)
        restored.select(one)
        XCTAssertEqual(restored.entries.first?.fields.title, "sample A")
        XCTAssertFalse(restored.canSyncSelection)
        c.disconnect()
        XCTAssertTrue(restored.edit(id: restored.entries[0].id, fields: ChecklistFields(title: "offline A", day: "2026-10-15", status: "done")))
        XCTAssertEqual(ChecklistModel(directory: dir).entries[0].fields.status, "done")
    }

    func testChecklistReconnectAdoptsLocalEntriesOnceAndPreservesExistingHostEntries() throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = ChecklistModel(directory: dir)
        let c = connection()
        let config = try configuration("host")
        c.useLocalService(name: "host", commands: []) { _, _, _ in
            XCTFail("Configuring a checklist must not send data"); return .null
        }
        model.configure(configuration: config, connection: c, demo: false)
        let hostScope = model.scope
        XCTAssertTrue(model.edit(fields: ChecklistFields(title: "host entry", day: "2026-10-14")))
        let hostID = try XCTUnwrap(model.entries.first?.id)
        model.select("local")
        XCTAssertTrue(model.edit(fields: ChecklistFields(title: "local entry", day: "2026-10-15")))
        let localID = try XCTUnwrap(model.entries.first?.id)
        let mutationID = try XCTUnwrap(model.entries.first?.mutationID)

        model.configure(configuration: config, connection: c, demo: false)
        model.configure(configuration: config, connection: c, demo: false)
        XCTAssertEqual(model.scope, hostScope)
        XCTAssertEqual(Set(model.entries.map(\.id)), Set([hostID, localID]))
        XCTAssertEqual(model.replica.entries[localID]?.mutationID, mutationID)
        XCTAssertEqual(model.pendingCount, 2)
        XCTAssertNil(model.error)

        let restored = ChecklistModel(directory: dir)
        XCTAssertEqual(restored.scope, hostScope)
        XCTAssertEqual(Set(restored.entries.map(\.id)), Set([hostID, localID]))
        restored.select("local")
        XCTAssertTrue(restored.entries.isEmpty)
        restored.configure(configuration: try configuration("other"), connection: c, demo: false)
        XCTAssertTrue(restored.entries.isEmpty, "Adopted entries must not move into another host")
        restored.select(hostScope)
        XCTAssertEqual(Set(restored.entries.map(\.id)), Set([hostID, localID]))
    }

    func testChecklistHandshakeThenSyncConvergesAndDoesNotReplayConversationCommands() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = ChecklistModel(directory: dir)
        XCTAssertTrue(model.edit(fields: ChecklistFields(title: "plan", day: "2026-10-14")))
        let c = connection(); var calls = 0
        c.useLocalService(name: "host", commands: ["native_research_checklist_sync"]) { command, project, args in
            XCTAssertEqual(command, "native_research_checklist_sync"); XCTAssertNil(project); calls += 1
            let reset = args["epoch"] == .null
            let changes = args["changes"]?.array ?? []
            let items: [RemoteValue] = reset ? [] : changes.map { value in
                .object(["id": value["id"], "title": value["title"], "day": value["day"], "status": value["status"],
                         "project_id": .null, "session_id": .null, "revision": .integer(1)])
            }
            let outcomes: [RemoteValue] = zip(changes, items).map { change, item in
                .object(["id": change["id"], "mutation_id": change["mutation_id"], "status": .string("applied"), "item": item])
            }
            return .object(["schema": .string("wisp.research-checklist.v1"), "epoch": .string("host"), "cursor": .integer(reset ? 0 : 1),
                "visibility": .string("none"), "visible_project_ids": .array([]), "reset": .bool(reset), "has_more": .bool(false),
                "items": .array(items), "outcomes": .array(outcomes)])
        }
        model.configure(configuration: try configuration("host"), connection: c, demo: false)
        await model.sync(connection: c)
        XCTAssertEqual(calls, 2); XCTAssertEqual(model.pendingCount, 0)
        XCTAssertEqual(model.entries[0].server?.revision, 1)
        XCTAssertEqual(ChecklistModel(directory: dir).pendingCount, 0)
    }

    func testNotebookOriginalAndOCRSurviveRestartAndFailedRecognition() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let data = image()
        let model = NotebookModel(directory: dir, recognizeText: { bytes in XCTAssertEqual(bytes, data); return "sample A: 0.25 µL" })
        let id = try XCTUnwrap(model.add(title: "实验", day: "2026-10-14"))
        await model.addPhoto(data, record: id)
        let photo = try XCTUnwrap(model.record(id)?.photos.first)
        XCTAssertEqual(photo.text, "sample A: 0.25 µL")
        XCTAssertEqual(try model.photoData(photo.id), data)
        XCTAssertNotNil(model.thumbnail(photo.id))
        XCTAssertTrue(model.saveText(record: id, photo: photo.id, text: "corrected A: 0.35 µL"))
        await model.recognize(record: id, photo: photo.id)
        XCTAssertEqual(model.record(id)?.photos[0].text, "corrected A: 0.35 µL", "OCR must not overwrite corrections")
        let restored = NotebookModel(directory: dir, recognizeText: { _ in throw RemoteError.service("recognition failed") })
        XCTAssertEqual(restored.record(id)?.photos[0].text, "corrected A: 0.35 µL")
        await restored.addPhoto(data, record: id)
        XCTAssertEqual(restored.record(id)?.photos.count, 2)
        XCTAssertEqual(try restored.photoData(restored.record(id)!.photos[1].id), data)
        XCTAssertNotNil(restored.error)
        XCTAssertNil(restored.record(id)?.transfer, "Import, recognition and restart must never upload")
    }

    func testNotebookOnlyUploadsSelectedPagesAndKeepsSuccessfulReceiptOnDuplicateTap() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = NotebookModel(directory: dir, recognizeText: { _ in "local OCR" })
        let id = try XCTUnwrap(model.add(title: "实验", day: "2026-10-14"))
        await model.addPhoto(image(), record: id); await model.addPhoto(image(), record: id)
        let photos = model.record(id)!.photos
        XCTAssertTrue(model.saveText(record: id, photo: photos[0].id, text: "do not send page one"))
        let selected: Set<String> = [photos[1].id]
        let c = connection(); let route = ConversationRoute(project: "p", session: "s")
        var uploads = 0; var sends = 0
        c.useLocalService(name: "host", commands: ["native_conversation_snapshot", "remote_attach_image", "native_conversation_send"]) { command, project, args in
            XCTAssertEqual(project, route.project); XCTAssertEqual(args["session_id"], .string(route.session))
            switch command {
            case "native_conversation_snapshot": return self.snapshot(route)
            case "remote_attach_image": uploads += 1; return .object(["path": .string("uploads/page.jpg")])
            case "native_conversation_send":
                sends += 1; XCTAssertEqual(args["attachments"], .array([.string("uploads/page.jpg")]))
                XCTAssertFalse(args["message"]!.string.contains("do not send page one"))
                XCTAssertTrue(args["message"]!.string.contains("local OCR")); return .null
            default: XCTFail(); return .null
            }
        }
        for _ in 0..<3 {
            await model.send(record: id, selected: selected, includeNotes: false, includeText: true, route: route,
                             scope: "host", hostName: "host", connection: c, expectedSession: c.readSessionID)
        }
        XCTAssertEqual(uploads, 1); XCTAssertEqual(sends, 1)
        XCTAssertEqual(model.record(id)?.transfer?.state, "sent")
        XCTAssertEqual(model.record(id)?.transfer?.photoIDs, Array(selected))
    }

    func testUncertainDeliveryPersistsWithoutRetryAndReadOnlyHostCannotUpload() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = NotebookModel(directory: dir, recognizeText: { _ in "" })
        let id = try XCTUnwrap(model.add(title: "实验", day: "2026-10-14"))
        XCTAssertTrue(model.saveNotes(id, title: "实验", day: "2026-10-14", notes: "observation"))
        let c = connection(); let route = ConversationRoute(project: "p", session: "s")
        var sends = 0; var readOnly = true
        c.useLocalService(name: "host", commands: ["native_conversation_snapshot", "native_conversation_send"]) { command, _, _ in
            if command == "native_conversation_snapshot" { return self.snapshot(route, readOnly: readOnly) }
            sends += 1; throw RemoteError.outcomeUnknown
        }
        await model.send(record: id, selected: [], includeNotes: true, includeText: false, route: route, scope: "host", hostName: "host", connection: c, expectedSession: c.readSessionID)
        XCTAssertEqual(sends, 0)
        readOnly = false
        await model.send(record: id, selected: [], includeNotes: true, includeText: false, route: route, scope: "host", hostName: "host", connection: c, expectedSession: c.readSessionID)
        XCTAssertEqual(sends, 1); XCTAssertEqual(model.record(id)?.transfer?.state, "uncertain")
        let restored = NotebookModel(directory: dir, recognizeText: { _ in XCTFail(); return "" })
        await restored.send(record: id, selected: [], includeNotes: true, includeText: false, route: route, scope: "host", hostName: "host", connection: c, expectedSession: c.readSessionID)
        XCTAssertEqual(sends, 1)
        restored.acknowledgeUnknown(id)
        await restored.send(record: id, selected: [], includeNotes: true, includeText: false, route: route, scope: "host", hostName: "host", connection: c, expectedSession: c.readSessionID)
        XCTAssertEqual(sends, 2)
    }

    func testCorruptDocumentsCannotBeOverwrittenAndCacheClearPreservesAuthoredData() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        let id = try XCTUnwrap(workspace.notebook.add(title: "keep", day: "2026-10-14"))
        XCTAssertTrue(workspace.checklist.edit(fields: ChecklistFields(title: "keep", day: "2026-10-14")))
        await workspace.clearCachedData()
        XCTAssertNotNil(NotebookModel(directory: dir).record(id))
        XCTAssertEqual(ChecklistModel(directory: dir).entries.count, 1)
        for name in ["notebook.json", "checklists.json"] { try Data("broken".utf8).write(to: dir.appendingPathComponent(name)) }
        XCTAssertNil(NotebookModel(directory: dir).add(title: "overwrite", day: "2026-10-14"))
        XCTAssertFalse(ChecklistModel(directory: dir).edit(fields: ChecklistFields(title: "overwrite", day: "2026-10-14")))
        for name in ["notebook.json", "checklists.json"] {
            XCTAssertEqual(try Data(contentsOf: dir.appendingPathComponent(name)), Data("broken".utf8))
        }
    }

    func testOfflineSurfacesEscapeOnlyDismissesTopmostLayer() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.page = .notebook; workspace.notebookPath = [.record("r"), .photo("r", "p")]
        XCTAssertNil(workspace.sheet)
        XCTAssertTrue(workspace.hasDismissibleSurface)
        workspace.notebookLibraryPresented = true
        workspace.dismissTopSurface()
        XCTAssertFalse(workspace.notebookLibraryPresented); XCTAssertEqual(workspace.notebookPath.count, 2)
        workspace.notebookCameraRecord = "r"
        workspace.dismissTopSurface()
        XCTAssertNil(workspace.notebookCameraRecord); XCTAssertEqual(workspace.notebookPath.count, 2)
        workspace.dismissTopSurface(); XCTAssertEqual(workspace.notebookPath.count, 1)
        workspace.dismissTopSurface(); XCTAssertTrue(workspace.notebookPath.isEmpty); XCTAssertEqual(workspace.page, .notebook)
        workspace.dismissTopSurface(); XCTAssertNil(workspace.page)
        workspace.page = .checklist; workspace.checklistPath = ["item"]
        workspace.dismissTopSurface(); XCTAssertTrue(workspace.checklistPath.isEmpty); XCTAssertEqual(workspace.page, .checklist)
        workspace.dismissTopSurface(); XCTAssertNil(workspace.page)
        XCTAssertFalse(workspace.hasDismissibleSurface)
    }

    func testModalAboveResearchPagePreservesNavigationAndSelection() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.selectedProject = "project"; workspace.selectedSession = "session"
        workspace.page = .checklist; workspace.checklistPath = ["item"]
        workspace.sheet = .connection
        workspace.dismissTopSurface()
        XCTAssertNil(workspace.sheet)
        XCTAssertEqual(workspace.page, .checklist)
        XCTAssertEqual(workspace.checklistPath, ["item"], "Closing a modal must not pop the page below it")
        workspace.dismissTopSurface()
        XCTAssertEqual(workspace.page, .checklist)
        workspace.dismissTopSurface()
        XCTAssertNil(workspace.page)
        XCTAssertEqual(workspace.route, ConversationRoute(project: "project", session: "session"))
    }

    func testLeavingResearchPageClearsDetailAndPhotoPresentationState() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.page = .notebook
        workspace.notebookPath = [.record("r"), .send("r")]
        workspace.notebookCameraRecord = "r"
        workspace.notebookLibraryPresented = true
        workspace.page = nil
        XCTAssertTrue(workspace.notebookPath.isEmpty)
        XCTAssertNil(workspace.notebookCameraRecord)
        XCTAssertFalse(workspace.notebookLibraryPresented)
        XCTAssertFalse(workspace.hasDismissibleSurface)
        workspace.page = .notebook
        XCTAssertTrue(workspace.notebookPath.isEmpty, "Reopening starts at the page root")
        workspace.page = .checklist
        workspace.checklistPath = ["item"]
        workspace.page = .research(nil)
        XCTAssertTrue(workspace.checklistPath.isEmpty)
        XCTAssertTrue(workspace.researchPath.isEmpty)
    }

    func testResearchPagesDoNotMarkTheCoveredConversationAsRead() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let connection = connection()
        let route = ConversationRoute(project: "project", session: "session")
        var seenCalls = 0
        connection.useLocalService(name: "host", commands: ["native_conversation_snapshot", "native_conversation_seen"]) { command, project, args in
            XCTAssertEqual(project, route.project)
            XCTAssertEqual(args["session_id"], .string(route.session))
            if command == "native_conversation_seen" { seenCalls += 1; return .null }
            return self.snapshot(route)
        }
        let workspace = RemoteWorkspace(connection: connection, preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.selectedProject = route.project; workspace.selectedSession = route.session
        workspace.visibleRoute = route
        let pages: [WorkspacePage] = [.checklist, .notebook, .research(nil), .research(route.project)]
        for page in pages {
            workspace.page = page
            try await workspace.refreshConversation()
            XCTAssertEqual(seenCalls, 0)
        }
        workspace.page = nil
        try await workspace.refreshConversation()
        XCTAssertEqual(seenCalls, 1, "Returning to the conversation restores read acknowledgements")
    }

    func testChangingConnectionBeforeSyncOrSendNeverTargetsTheNewHost() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let checklist = ChecklistModel(directory: dir)
        let notebook = NotebookModel(directory: dir, recognizeText: { _ in "" })
        let c = connection()
        c.useLocalService(name: "old", commands: []) { _, _, _ in XCTFail(); return .null }
        checklist.configure(configuration: try configuration("old"), connection: c, demo: false)
        XCTAssertTrue(checklist.edit(fields: ChecklistFields(title: "old host only", day: "2026-10-14")))
        let session = c.readSessionID
        let id = try XCTUnwrap(notebook.add(title: "record", day: "2026-10-14"))
        XCTAssertTrue(notebook.saveNotes(id, title: "record", day: "2026-10-14", notes: "old host only"))
        c.useLocalService(name: "new", commands: ["native_research_checklist_sync", "native_conversation_snapshot", "native_conversation_send"]) { _, _, _ in
            XCTFail("Stale selection must not send any data to the replacement connection"); return .null
        }
        await checklist.sync(connection: c)
        await notebook.send(record: id, selected: [], includeNotes: true, includeText: false,
                            route: ConversationRoute(project: "p", session: "s"), scope: "old", hostName: "old",
                            connection: c, expectedSession: session)
        XCTAssertEqual(checklist.pendingCount, 1); XCTAssertNil(notebook.record(id)?.transfer)
    }

    func testProcessExitDuringSendLoadsAsUncertainWithoutReplay() throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        struct Saved: Codable { let version: Int; let records: [NotebookRecord] }
        var record = NotebookRecord(title: "record", day: "2026-10-14", notes: "preserve")
        record.transfer = NotebookTransfer(id: UUID().uuidString, signature: "sig", host: "host", project: "p", session: "s", photoIDs: [], state: "sending")
        try OfflineResearchStorage.save(Saved(version: 1, records: [record]), to: dir.appendingPathComponent("notebook.json"))
        let model = NotebookModel(directory: dir, recognizeText: { _ in XCTFail(); return "" })
        XCTAssertEqual(model.record(record.id)?.transfer?.state, "uncertain")
        XCTAssertEqual(model.record(record.id)?.notes, "preserve")
    }
}
