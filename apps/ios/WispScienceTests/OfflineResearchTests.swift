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

    func testChecklistSectionsKeepConflictsVisibleAndGroupOpenTasksByDay() throws {
        var replica = ChecklistReplica()
        for (id, day, status) in [
            ("overdue", "2026-12-18", "open"), ("today", "2026-12-19", "open"),
            ("next-year", "2027-01-01", "open"), ("tomorrow", "2026-12-20", "open"),
            ("done", "2026-12-18", "done"), ("dropped", "2026-12-19", "dropped"),
            ("conflict-done", "2026-12-19", "done"), ("conflict-dropped", "2026-12-19", "dropped"),
        ] {
            try replica.edit(id: id, fields: ChecklistFields(title: id, day: day, status: status))
        }
        var entries = Array(replica.entries.values)
        for index in entries.indices where entries[index].id.hasPrefix("conflict-") { entries[index].conflict = true }
        let sections = ChecklistSections(entries: Array(entries.reversed()), today: "2026-12-19")
        XCTAssertEqual(sections.needsReview.map(\.id), ["conflict-done", "conflict-dropped"])
        XCTAssertEqual(sections.overdue.map(\.id), ["overdue"])
        XCTAssertEqual(sections.today.map(\.id), ["today"])
        XCTAssertEqual(sections.upcoming.map(\.id), ["tomorrow", "next-year"])
        XCTAssertEqual(sections.done.map(\.id), ["done"])
        XCTAssertEqual(sections.remainingCount, 6)
        let nextDay = ChecklistSections(entries: entries, today: "2026-12-20")
        XCTAssertEqual(nextDay.overdue.map(\.id), ["overdue", "today"])
        XCTAssertEqual(nextDay.today.map(\.id), ["tomorrow"])
        XCTAssertEqual(nextDay.upcoming.map(\.id), ["next-year"])
        XCTAssertEqual(nextDay.remainingCount, 6)
    }

    func testChecklistCompletionAndReopeningMoveBetweenSectionsAndPersist() throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = ChecklistModel(directory: dir)
        XCTAssertTrue(model.edit(id: "sample", fields: ChecklistFields(title: "核对样本编号", day: "2026-10-14")))
        XCTAssertTrue(model.edit(id: "sample", fields: ChecklistFields(title: "核对样本编号", day: "2026-10-14", status: "done")))
        var sections = ChecklistSections(entries: ChecklistModel(directory: dir).entries, today: "2026-10-14")
        XCTAssertEqual(sections.remainingCount, 0)
        XCTAssertEqual(sections.done.map(\.id), ["sample"])
        XCTAssertTrue(model.edit(id: "sample", fields: ChecklistFields(title: "核对样本编号", day: "2026-10-15")))
        sections = ChecklistSections(entries: ChecklistModel(directory: dir).entries, today: "2026-10-14")
        XCTAssertTrue(sections.done.isEmpty)
        XCTAssertEqual(sections.upcoming.map(\.id), ["sample"])
        XCTAssertEqual(sections.remainingCount, 1)
    }

    func testChecklistSettingsDismissBeforeTheTabAndNeverSwipeAwayATab() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.tab = .checklist
        workspace.sheet = .checklistSettings
        XCTAssertFalse(workspace.canSwipeBackToWorkspace)
        workspace.dismissTopSurface()
        XCTAssertNil(workspace.sheet)
        XCTAssertEqual(workspace.tab, .checklist)
        XCTAssertFalse(workspace.canSwipeBackToWorkspace, "A tab root has nothing to swipe back to")
        XCTAssertFalse(workspace.hasDismissibleSurface)
        workspace.dismissTopSurface()
        XCTAssertEqual(workspace.tab, .checklist, "Escape never closes a tab")
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

    func testFirstObservationNeedsNoTitleAndPersistsInOneRecord() throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = NotebookModel(directory: dir, recognizeText: { _ in XCTFail("Text entry must not invoke OCR"); return "" })
        XCTAssertTrue(model.records.isEmpty)
        XCTAssertNil(model.createObservation(title: "  ", day: "2026-10-13", notes: "\n "))
        XCTAssertTrue(model.records.isEmpty, "An empty editor must not create a record")
        let notes = "  A03 出现沉淀，补做离心。\n下午复测浓度。"
        let id = try XCTUnwrap(model.createObservation(title: "", day: "2026-10-13", notes: notes))
        XCTAssertEqual(model.record(id)?.title, "A03 出现沉淀，补做离心。")
        let restored = NotebookModel(directory: dir)
        XCTAssertEqual(restored.records.count, 1)
        XCTAssertEqual(restored.record(id)?.notes, notes)
        XCTAssertNil(restored.record(id)?.transfer)
        XCTAssertEqual(restored.record(id)?.photos.count, 0)
    }

    func testNewObservationRejectsInvalidFieldsWithoutPartialSave() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = NotebookModel(directory: dir)
        XCTAssertNil(model.createObservation(title: "实验", day: "invalid", notes: "观察"))
        XCTAssertNil(model.createObservation(title: String(repeating: "a", count: 201), day: "2026-10-13", notes: "观察"))
        XCTAssertNil(model.createObservation(title: "实验", day: "2026-10-13", notes: String(repeating: "x", count: 256 * 1024 + 1)))
        XCTAssertTrue(model.records.isEmpty)
        XCTAssertTrue(NotebookModel(directory: dir).records.isEmpty)
    }

    func testPhotoFirstImportKeepsOriginalWhenRecognitionFails() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = NotebookModel(directory: dir, recognizeText: { _ in throw RemoteError.service("Offline OCR unavailable") })
        let data = image()
        let result = await model.createFromPhoto(data, title: "", day: "2026-10-13", notes: "")
        let imported = try XCTUnwrap(result)
        XCTAssertEqual(model.records.count, 1)
        XCTAssertEqual(model.record(imported.record)?.title, "实验记录 · 2026-10-13")
        XCTAssertEqual(try model.photoData(imported.photo), data)
        XCTAssertNotNil(model.error, "The correction page must explain that OCR did not finish")
        let restored = NotebookModel(directory: dir)
        XCTAssertEqual(restored.record(imported.record)?.photos.first?.id, imported.photo)
        XCTAssertEqual(restored.record(imported.record)?.photos.first?.text, "")
        XCTAssertNil(restored.record(imported.record)?.transfer)
    }

    func testInvalidFirstPhotoDoesNotCreateAnEmptyRecordOrRunOCR() async {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = NotebookModel(directory: dir, recognizeText: { _ in XCTFail("Invalid photos must not reach OCR"); return "" })
        let result = await model.createFromPhoto(Data("not an image".utf8), title: "", day: "2026-10-13", notes: "")
        XCTAssertNil(result)
        XCTAssertTrue(model.records.isEmpty)
        XCTAssertTrue(NotebookModel(directory: dir).records.isEmpty)
        XCTAssertNotNil(model.error)
    }

    func testPhotoFirstImportCanBeCorrectedAndExtendedWithoutDuplicatingRecord() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let model = NotebookModel(directory: dir, recognizeText: { _ in "A03: 0.25 µL" })
        let result = await model.createFromPhoto(image(), title: "提取记录", day: "2026-10-13", notes: "第一批")
        let imported = try XCTUnwrap(result)
        XCTAssertEqual(model.record(imported.record)?.photos.first?.text, "A03: 0.25 µL")
        XCTAssertTrue(model.saveText(record: imported.record, photo: imported.photo, text: "A03: 0.35 µL"))
        let second = await model.addPhoto(image(), record: imported.record)
        XCTAssertNotNil(second)
        XCTAssertEqual(model.records.count, 1)
        let restored = NotebookModel(directory: dir)
        XCTAssertEqual(restored.record(imported.record)?.notes, "第一批")
        XCTAssertEqual(restored.record(imported.record)?.photos.count, 2)
        XCTAssertEqual(restored.record(imported.record)?.photos.first?.text, "A03: 0.35 µL")
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
        workspace.tab = .notebook; workspace.notebookPath = [.record("r"), .photo("r", "p")]
        XCTAssertNil(workspace.sheet)
        XCTAssertTrue(workspace.hasDismissibleSurface)
        workspace.notebookLibraryPresented = true
        workspace.dismissTopSurface()
        XCTAssertFalse(workspace.notebookLibraryPresented); XCTAssertEqual(workspace.notebookPath.count, 2)
        workspace.notebookCameraPresented = true
        workspace.dismissTopSurface()
        XCTAssertFalse(workspace.notebookCameraPresented); XCTAssertEqual(workspace.notebookPath.count, 2)
        workspace.dismissTopSurface(); XCTAssertEqual(workspace.notebookPath.count, 1)
        workspace.dismissTopSurface(); XCTAssertTrue(workspace.notebookPath.isEmpty); XCTAssertEqual(workspace.tab, .notebook)
        XCTAssertFalse(workspace.hasDismissibleSurface)
        workspace.tab = .checklist; workspace.checklistPath = ["item"]
        workspace.dismissTopSurface(); XCTAssertTrue(workspace.checklistPath.isEmpty); XCTAssertEqual(workspace.tab, .checklist)
        XCTAssertFalse(workspace.hasDismissibleSurface)
    }

    func testTabsKeepTheirOwnStacksAndEscapeOnlyAffectsTheVisibleTab() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.tab = .checklist; workspace.checklistPath = ["item"]
        workspace.tab = .notebook; workspace.notebookPath = [.record("r")]
        workspace.tab = .conversations
        workspace.page = .journey("p"); workspace.researchPath = [.journey("p", Date())]
        workspace.calendarPath = [.journey("p", Date())]
        workspace.dismissTopSurface()
        XCTAssertTrue(workspace.researchPath.isEmpty)
        XCTAssertEqual(workspace.calendarPath.count, 1, "The journey page and the calendar tab never pop each other")
        XCTAssertEqual(workspace.checklistPath, ["item"]); XCTAssertEqual(workspace.notebookPath.count, 1)
        workspace.dismissTopSurface()
        XCTAssertNil(workspace.page)
        workspace.tab = .calendar
        XCTAssertTrue(workspace.hasDismissibleSurface)
        workspace.dismissTopSurface()
        XCTAssertTrue(workspace.calendarPath.isEmpty)
        workspace.tab = .checklist
        XCTAssertEqual(workspace.checklistPath, ["item"], "Switching tabs keeps each tool's place")
    }

    func testAvailableTabsFollowConnectionAndDemo() throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let suite = "wisp-tabs-" + UUID().uuidString
        let preferences = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { preferences.removePersistentDomain(forName: suite) }
        let offline = RemoteWorkspace(connection: connection(), credentials: TabCredentials(saved: nil), preferences: preferences,
                                      cache: nil, offlineDirectory: dir)
        XCTAssertEqual(offline.availableTabs, [.conversations, .checklist, .notebook])
        offline.enterDemo()
        XCTAssertEqual(offline.availableTabs, [.conversations, .calendar], "The demo hides the personal offline tools")
        offline.exitDemo()
        XCTAssertEqual(offline.availableTabs, [.conversations, .checklist, .notebook])
        let paired = RemoteWorkspace(connection: connection(), credentials: TabCredentials(saved: try configuration("tabs")),
                                     preferences: nil, cache: nil, offlineDirectory: dir)
        paired.setActive(false) // Restore the saved pairing without opening a socket.
        paired.restore()
        XCTAssertEqual(paired.availableTabs, WorkspaceTab.allCases)
    }

    func testIPadSidebarMapsSectionsAndProjectsWithoutLosingPlaces() throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let offline = RemoteWorkspace(connection: connection(), credentials: TabCredentials(saved: nil), preferences: nil,
                                      cache: nil, offlineDirectory: dir)
        XCTAssertEqual(offline.sidebarItem, .section(.conversations), "Before pairing the first section is the connection form")
        XCTAssertEqual(offline.sectionTitle(.conversations), "连接")
        offline.sidebarItem = .section(.calendar)
        XCTAssertEqual(offline.tab, .conversations, "The calendar needs a host")
        offline.sidebarItem = .section(.checklist)
        XCTAssertEqual(offline.sidebarItem, .section(.checklist))
        offline.checklistPath = ["item"]
        offline.sidebarItem = .section(.notebook)
        offline.sidebarItem = .section(.checklist)
        XCTAssertEqual(offline.checklistPath, ["item"], "Each section keeps the item open beside its list")

        let paired = RemoteWorkspace(connection: connection(), credentials: TabCredentials(saved: try configuration("sidebar")),
                                     preferences: nil, cache: nil, offlineDirectory: dir)
        paired.setActive(false); paired.restore()
        XCTAssertNil(paired.sidebarItem, "Paired, the projects stand for the conversations section")
        XCTAssertEqual(paired.sectionTitle(.conversations), "对话")
        paired.selectedProject = "a"; paired.selectedSession = "s"
        XCTAssertEqual(paired.sidebarItem, .project("a"))
        paired.sidebarItem = .section(.calendar)
        XCTAssertEqual(paired.tab, .calendar)
        XCTAssertEqual(paired.sidebarItem, .section(.calendar))
        paired.sidebarItem = .project("a")
        XCTAssertEqual(paired.tab, .conversations)
        XCTAssertEqual(paired.route, ConversationRoute(project: "a", session: "s"), "Returning to a project keeps its conversation")
        paired.sidebarItem = .project("b")
        XCTAssertEqual(paired.selectedProject, "b")
        XCTAssertNil(paired.selectedSession)
        paired.sidebarItem = nil
        XCTAssertEqual(paired.selectedProject, "b", "Clearing the list selection does not drop the project")
    }

    func testKeyboardCommandsFollowTheVisibleSectionAndNeverActUnderAModal() async throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let host = connection()
        let workspace = RemoteWorkspace(connection: host, credentials: TabCredentials(saved: try configuration("keys")),
                                        preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.setActive(false); workspace.restore()
        host.useLocalService(name: "host", commands: ["native_conversation_create"]) { _, _, _ in .null }
        XCTAssertFalse(workspace.canCreateConversation, "No project is selected")
        workspace.selectedProject = "p"
        XCTAssertTrue(workspace.canCreateConversation)
        XCTAssertTrue(workspace.canRefresh)
        workspace.page = .journey("p")
        XCTAssertFalse(workspace.canCreateConversation, "Not behind the journey page")
        workspace.page = nil
        workspace.showSection(.checklist)
        XCTAssertEqual(workspace.tab, .checklist)
        XCTAssertFalse(workspace.canCreateConversation)
        XCTAssertFalse(workspace.canRefresh, "The checklist syncs on its own")
        workspace.sheet = .checklistSettings
        XCTAssertFalse(workspace.canShowSection(.notebook))
        workspace.showSection(.notebook)
        XCTAssertEqual(workspace.tab, .checklist, "A shortcut never switches sections under a modal")
        workspace.sheet = nil; workspace.notebookLibraryPresented = true
        XCTAssertFalse(workspace.canShowSection(.notebook))
        workspace.notebookLibraryPresented = false
        workspace.showSection(.calendar)
        XCTAssertEqual(workspace.tab, .calendar)
        let refresh = workspace.researchCalendar.refresh
        await workspace.refreshVisible()
        XCTAssertNotEqual(workspace.researchCalendar.refresh, refresh, "Refreshing the calendar reloads its month and day")

        let demo = RemoteWorkspace(connection: connection(), credentials: TabCredentials(saved: nil), preferences: nil,
                                   cache: nil, offlineDirectory: dir)
        demo.enterDemo()
        demo.selectedProject = "demo"
        XCTAssertFalse(demo.canCreateConversation, "The demo cannot create conversations")
        XCTAssertFalse(demo.canShowSection(.checklist), "The demo hides the personal offline tools")
        XCTAssertTrue(demo.canShowSection(.calendar))
    }

    func testCalendarSelectionAndConnectionInputOutliveTheViewButNotTheHost() throws {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), credentials: TabCredentials(saved: nil), preferences: nil,
                                        cache: nil, offlineDirectory: dir)
        workspace.connectionAddress = "http://relay.example.test/remote"
        workspace.showSection(.checklist); workspace.showSection(.conversations)
        workspace.connect(address: workspace.connectionAddress, code: "")
        XCTAssertNotNil(workspace.error)
        XCTAssertEqual(workspace.connectionAddress, "http://relay.example.test/remote", "Rejected input stays for correction")
        let calendar = workspace.researchCalendar
        calendar.project = "p"; calendar.day = Date(timeIntervalSince1970: 0); calendar.month = calendar.day
        XCTAssertEqual(calendar.projectIDs([.object(["id": .string("q")]), .object(["id": .string("p")])]), ["p"])
        calendar.project = nil
        XCTAssertEqual(calendar.projectIDs([.object(["id": .string("q")]), .object(["id": .string("p")])]), ["p", "q"])
        calendar.project = "p"
        workspace.enterDemo()
        XCTAssertNil(calendar.project, "Another host's project filter does not carry over")
        XCTAssertTrue(Calendar.current.isDateInToday(calendar.day))
        XCTAssertTrue(Calendar.current.isDateInToday(calendar.month))
    }

    func testEdgeBackReturnsFromTheJourneyWithoutChangingConversationOrDraft() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        let route = ConversationRoute(project: "project", session: "session")
        workspace.selectedProject = route.project; workspace.selectedSession = route.session
        workspace.drafts[route] = "Unsent observation"
        XCTAssertFalse(workspace.beginPageBackGesture())
        workspace.page = .journey(route.project)
        XCTAssertTrue(workspace.beginPageBackGesture())
        workspace.updatePageBackGesture(translation: 160, width: 400)
        XCTAssertEqual(workspace.pageBackProgress, 0.4, accuracy: 0.001)
        workspace.finishPageBackGesture(translation: 160, velocity: 0, width: 400, cancelled: false)
        XCTAssertNil(workspace.page)
        XCTAssertEqual(workspace.pageBackProgress, 0)
        XCTAssertEqual(workspace.route, route)
        XCTAssertEqual(workspace.drafts[route], "Unsent observation")
        for tab in [WorkspaceTab.calendar, .checklist, .notebook] {
            workspace.tab = tab
            XCTAssertFalse(workspace.beginPageBackGesture(), "Tab roots have no workspace page to leave")
        }
    }

    func testEdgeBackCancellationReversalAndShortFlick() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.page = .journey("p")
        let cancelledDrags: [(CGFloat, CGFloat, Bool)] = [
            (60, 0, false), (250, -300, false), (250, 800, true), (10, 1000, false),
        ]
        for (translation, velocity, cancelled) in cancelledDrags {
            XCTAssertTrue(workspace.beginPageBackGesture())
            workspace.updatePageBackGesture(translation: translation, width: 400)
            workspace.finishPageBackGesture(translation: translation, velocity: velocity, width: 400, cancelled: cancelled)
            XCTAssertEqual(workspace.page, .journey("p"))
            XCTAssertEqual(workspace.pageBackProgress, 0)
        }
        XCTAssertTrue(workspace.beginPageBackGesture())
        workspace.finishPageBackGesture(translation: 60, velocity: 800, width: 400, cancelled: false)
        XCTAssertNil(workspace.page, "A deliberate quick edge flick also returns")
    }

    func testEdgeBackLeavesNativeDetailNavigationAndModalsInControl() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.page = .journey("p"); workspace.researchPath = [.journey("p", Date())]
        XCTAssertFalse(workspace.beginPageBackGesture())
        // A native detail pop can empty the path during the same touch. No
        // workspace gesture was begun, so its completion must not pop again.
        workspace.researchPath = []
        workspace.finishPageBackGesture(translation: 300, velocity: 800, width: 400, cancelled: false)
        XCTAssertEqual(workspace.page, .journey("p"))
        workspace.sheet = .connection
        XCTAssertFalse(workspace.beginPageBackGesture())
        workspace.sheet = nil; workspace.notebookLibraryPresented = true
        XCTAssertFalse(workspace.beginPageBackGesture())
        workspace.notebookLibraryPresented = false; workspace.notebookCameraPresented = true
        XCTAssertFalse(workspace.beginPageBackGesture())
        workspace.notebookCameraPresented = false; workspace.tab = .calendar
        XCTAssertFalse(workspace.beginPageBackGesture(), "The journey is not visible from another tab")
    }

    func testInterruptedEdgeBackCannotCloseAReplacementPageOrReachThroughAModal() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.page = .journey("a")
        XCTAssertTrue(workspace.beginPageBackGesture())
        workspace.updatePageBackGesture(translation: 150, width: 400)
        workspace.page = .journey("b")
        XCTAssertEqual(workspace.pageBackProgress, 0)
        workspace.finishPageBackGesture(translation: 300, velocity: 800, width: 400, cancelled: false)
        XCTAssertEqual(workspace.page, .journey("b"))
        XCTAssertTrue(workspace.beginPageBackGesture())
        workspace.sheet = .connection
        workspace.finishPageBackGesture(translation: 300, velocity: 800, width: 400, cancelled: false)
        XCTAssertNotNil(workspace.sheet)
        XCTAssertEqual(workspace.page, .journey("b"))
        XCTAssertEqual(workspace.pageBackProgress, 0)
        workspace.sheet = nil
        XCTAssertTrue(workspace.beginPageBackGesture())
        workspace.tab = .checklist // Switching tabs interrupts the drag.
        workspace.tab = .conversations
        workspace.finishPageBackGesture(translation: 300, velocity: 800, width: 400, cancelled: false)
        XCTAssertEqual(workspace.page, .journey("b"))
    }

    func testModalAboveAToolTabPreservesNavigationAndSelection() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.selectedProject = "project"; workspace.selectedSession = "session"
        workspace.tab = .checklist; workspace.checklistPath = ["item"]
        workspace.sheet = .connection
        workspace.dismissTopSurface()
        XCTAssertNil(workspace.sheet)
        XCTAssertEqual(workspace.checklistPath, ["item"], "Closing a modal must not pop the page below it")
        workspace.dismissTopSurface()
        XCTAssertTrue(workspace.checklistPath.isEmpty)
        XCTAssertEqual(workspace.tab, .checklist)
        XCTAssertEqual(workspace.route, ConversationRoute(project: "project", session: "session"))
    }

    func testLeavingTheJourneyClearsItsDetailStackOnly() {
        let dir = directory(); defer { try? FileManager.default.removeItem(at: dir) }
        let workspace = RemoteWorkspace(connection: connection(), preferences: nil, cache: nil, offlineDirectory: dir)
        workspace.notebookPath = [.record("r"), .send("r")]
        workspace.page = .journey("p"); workspace.researchPath = [.journey("p", Date())]
        workspace.page = nil
        XCTAssertTrue(workspace.researchPath.isEmpty)
        XCTAssertEqual(workspace.notebookPath.count, 2, "Leaving the journey does not touch the notebook tab")
    }

    func testToolTabsAndTheJourneyDoNotMarkTheCoveredConversationAsRead() async throws {
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
        for tab in [WorkspaceTab.calendar, .checklist, .notebook] {
            workspace.tab = tab
            try await workspace.refreshConversation()
            XCTAssertEqual(seenCalls, 0)
        }
        workspace.tab = .conversations
        workspace.page = .journey(route.project)
        try await workspace.refreshConversation()
        XCTAssertEqual(seenCalls, 0)
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

private final class TabCredentials: RemoteCredentialStoring {
    let saved: RemoteConfiguration?
    init(saved: RemoteConfiguration?) { self.saved = saved }
    func load() throws -> RemoteConfiguration? { saved }
    func save(_ configuration: RemoteConfiguration) throws { }
    func remove() throws { }
}
