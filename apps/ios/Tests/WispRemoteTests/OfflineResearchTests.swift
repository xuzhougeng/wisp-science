import XCTest
@testable import WispRemote

final class OfflineResearchTests: XCTestCase {
    private let id = "e91f72ed-402e-41d7-b271-057858fdb956"
    private func item(_ revision: Int64, _ title: String = "host", _ status: String = "open", project: String? = nil) -> RemoteValue {
        .object(["id": .string(id), "title": .string(title), "day": .string("2026-10-14"), "status": .string(status),
                 "project_id": project.map(RemoteValue.string) ?? .null, "session_id": .null, "revision": .integer(revision)])
    }
    private func reply(items: [RemoteValue] = [], outcomes: [RemoteValue] = [], reset: Bool = false, epoch: String = "epoch", visible: [String] = []) throws -> ChecklistReply {
        try ChecklistReply(.object(["schema": .string("wisp.research-checklist.v1"), "epoch": .string(epoch), "cursor": .integer(5),
            "visibility": .string("v"), "visible_project_ids": .array(visible.map(RemoteValue.string)), "reset": .bool(reset),
            "has_more": .bool(false), "items": .array(items), "outcomes": .array(outcomes)]))
    }
    private func outcome(_ mutation: String, item: RemoteValue, status: String = "applied") -> RemoteValue {
        .object(["id": .string(id), "mutation_id": .string(mutation), "status": .string(status), "item": item])
    }
    private func fields(_ title: String) -> ChecklistFields { ChecklistFields(title: title, day: "2026-10-14") }

    func testSharedContract() throws {
        let value = try fixture("research-checklist/v1/sync.json")
        let reply = try ChecklistReply(value["result"])
        XCTAssertEqual(reply.items.first?.id, value["args"]["changes"].array[0]["id"].string)
        XCTAssertEqual(reply.outcomes.first?.status, "applied")
        XCTAssertThrowsError(try ChecklistReply(.object([:])))
    }

    func testOfflineEditsPersistAndAcknowledgementDoesNotEraseNewerEdit() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = dir.appendingPathComponent("checklist.json")
        var replica = ChecklistReplica()
        try replica.edit(id: id, fields: fields("first"))
        try replica.merge(reply(reset: true))
        let mutation = try XCTUnwrap(replica.entries[id]?.mutationID)
        try replica.edit(id: id, fields: fields("newer"))
        try replica.merge(reply(items: [item(1, "first")], outcomes: [outcome(mutation, item: item(1, "first"))]))
        XCTAssertEqual(replica.entries[id]?.fields.title, "newer")
        XCTAssertEqual(replica.entries[id]?.baseRevision, 1)
        XCTAssertFalse(replica.entries[id]!.conflict)
        XCTAssertEqual(replica.pendingCount, 1)
        try OfflineResearchStorage.save(replica, to: file)
        let restored = try XCTUnwrap(OfflineResearchStorage.load(ChecklistReplica.self, from: file))
        XCTAssertEqual(restored.request, replica.request)
        try Data("broken".utf8).write(to: file)
        XCTAssertThrowsError(try OfflineResearchStorage.load(ChecklistReplica.self, from: file))
        XCTAssertEqual(try Data(contentsOf: file), Data("broken".utf8))
    }

    func testConcurrentEditsRequireResolutionAndDroppedItemsStayDropped() throws {
        var replica = ChecklistReplica()
        try replica.merge(reply(items: [item(1)], reset: true))
        try replica.edit(id: id, fields: fields("phone"))
        let mutation = replica.entries[id]!.mutationID!
        try replica.merge(reply(items: [item(2, "desktop", "done")], outcomes: [outcome(mutation, item: item(2, "desktop", "done"), status: "conflict")]))
        XCTAssertTrue(replica.entries[id]!.conflict)
        XCTAssertEqual(replica.entries[id]?.fields.title, "phone")
        XCTAssertEqual(replica.pendingCount, 0)
        replica.resolve(id: id, keepLocal: true)
        XCTAssertEqual(replica.entries[id]?.baseRevision, 2)
        let retry = replica.entries[id]!.mutationID!
        try replica.merge(reply(items: [item(3, "phone")], outcomes: [outcome(retry, item: item(3, "phone"))]))
        try replica.merge(reply(items: [item(4, "phone", "dropped")]))
        XCTAssertEqual(replica.entries[id]?.fields.status, "dropped")
        XCTAssertEqual(replica.pendingCount, 0)
    }

    func testPrivacyAndDatabaseReplacementPreserveLocalIntent() throws {
        var replica = ChecklistReplica()
        try replica.merge(reply(items: [item(1, project: "private")], reset: true, visible: ["private"]))
        XCTAssertTrue(replica.visible(replica.entries[id]!))
        try replica.edit(id: id, fields: fields("offline"))
        try replica.merge(reply())
        XCTAssertFalse(replica.visible(replica.entries[id]!))
        try replica.merge(reply(reset: true, epoch: "new"))
        XCTAssertFalse(replica.visible(replica.entries[id]!), "A reset must not reveal cached private project data")
        XCTAssertTrue(replica.entries[id]!.conflict)
        XCTAssertEqual(replica.entries[id]?.fields.title, "offline")
        XCTAssertEqual(replica.pendingCount, 0)
        replica.resolve(id: id, keepLocal: false)
        XCTAssertNil(replica.entries[id])
        XCTAssertThrowsError(try replica.edit(fields: ChecklistFields(title: "sample", day: "2026-02-30")))
    }

    func testNotebookOnlySerializesSelectedPagesAndOptionalText() throws {
        var record = NotebookRecord(title: "实验 A", day: "2026-10-14", notes: "private notes")
        record.photos = (0..<5).map { NotebookPhoto(text: "page-\($0)") }
        let selected: Set<String> = [record.photos[1].id]
        let message = try record.message(selected: selected, includeNotes: false, includeText: true)
        XCTAssertTrue(message.contains("page-1"))
        XCTAssertFalse(message.contains("page-0")); XCTAssertFalse(message.contains("private notes"))
        XCTAssertFalse(try record.message(selected: selected, includeNotes: false, includeText: false).contains("page-1"))
        XCTAssertThrowsError(try record.message(selected: Set(record.photos.map(\.id)), includeNotes: true, includeText: true))
        XCTAssertThrowsError(try record.message(selected: ["missing"], includeNotes: true, includeText: true))
        record.notes = ""
        XCTAssertThrowsError(try record.message(selected: [], includeNotes: true, includeText: true))
    }
}
