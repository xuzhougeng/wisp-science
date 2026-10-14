import XCTest
@testable import WispRemote

final class RemoteResearchTests: XCTestCase {
    private func calendar(_ zone: String = "UTC") -> Calendar {
        RemoteResearchClock.localCalendar(timeZone: TimeZone(identifier: zone)!)
    }
    private func date(_ year: Int, _ month: Int, _ day: Int, calendar: Calendar) -> Date {
        calendar.date(from: DateComponents(year: year, month: month, day: day, hour: 12))!
    }

    func testMondayGridLeapMonthAndYearBoundary() {
        let c = calendar()
        let september = RemoteResearchClock.cells(containing: date(2026, 9, 12, calendar: c), calendar: c)
        XCTAssertNil(september[0])
        XCTAssertEqual(c.component(.day, from: september[1]!), 1)
        XCTAssertEqual(september.compactMap { $0 }.count, 30)
        XCTAssertEqual(september.count % 7, 0)
        let leap = RemoteResearchClock.cells(containing: date(2024, 2, 1, calendar: c), calendar: c)
        XCTAssertEqual(leap.compactMap { $0 }.count, 29)
        let december = RemoteResearchClock.interval(.month, containing: date(2026, 12, 31, calendar: c), calendar: c)
        XCTAssertEqual(c.component(.year, from: december.end), 2027)
        XCTAssertEqual(c.component(.day, from: december.end), 1)
    }

    func testDSTDaysAndMonthQueriesUseLocalCalendarIntervals() {
        let c = calendar("America/Los_Angeles")
        let spring = RemoteResearchClock.interval(.day, containing: date(2026, 3, 8, calendar: c), calendar: c)
        let fall = RemoteResearchClock.interval(.day, containing: date(2026, 11, 1, calendar: c), calendar: c)
        XCTAssertEqual(spring.duration, 23 * 3600)
        XCTAssertEqual(fall.duration, 25 * 3600)
        XCTAssertEqual(RemoteResearchClock.cells(containing: fall.start, calendar: c).compactMap { $0 }.count, 30)
        XCTAssertEqual(RemoteResearchClock.args(spring)["until"], .integer(Int64(spring.end.timeIntervalSince1970)))
    }

    func testSharedContractsAndProjectIsolation() throws {
        let history = try RemoteResearchHistory(fixture("native-journey/v1/range.json")["result"])
        XCTAssertEqual(history.entries.first?.title, "a finding")
        XCTAssertTrue(history.entries.first?.manual == true)
        XCTAssertFalse(history.truncated)
        let value = try fixture("native-calendar/v1/month.json")["result"]
        let projects = try RemoteResearchProject.decode(value, requested: ["research-1"])
        XCTAssertEqual(projects[0].history.entries, history.entries)
        XCTAssertThrowsError(try RemoteResearchProject.decode(value, requested: ["other"]))
        XCTAssertThrowsError(try RemoteResearchProject.decode(.array(value.array + value.array), requested: ["research-1"]))
        XCTAssertThrowsError(try RemoteResearchHistory(.object([:])))
        // Privacy-filtered projects may be omitted; failed projects stay visible.
        XCTAssertTrue(try RemoteResearchProject.decode(.array([]), requested: ["hidden"]).isEmpty)
        let failed: RemoteValue = .object(["project_id": .string("p"), "history": .object(["entries": .array([]), "truncated": .bool(true)]), "error": .string("unavailable")])
        let row = try RemoteResearchProject.decode(.array([failed]), requested: ["p"])[0]
        XCTAssertEqual(row.error, "unavailable")
        XCTAssertTrue(row.history.truncated)
    }

    func testDeduplicationDaySessionGroupingAndDiscardedSourceNavigation() throws {
        guard case .object(var row) = try fixture("native-journey/v1/range.json")["result"]["entries"].array[0] else { return XCTFail() }
        row["frame_id"] = .string("chat-a")
        let first = RemoteValue.object(row)
        row["id"] = .string("second"); row["occurred_at"] = .integer(86401); row["frame_id"] = .string("chat-b")
        let second = RemoteValue.object(row)
        let history = try RemoteResearchHistory(.object(["entries": .array([first, second, first]), "truncated": .bool(false)]))
        XCTAssertEqual(history.entries.count, 2)
        XCTAssertEqual(history.days(calendar: calendar()).count, 2)
        XCTAssertEqual(history.days(calendar: calendar())[0].sessions[0].sessionID, "chat-b")
        XCTAssertEqual(history.entries[0].route(project: "p"), ConversationRoute(project: "p", session: "chat-b"))
        row["source_discarded"] = .bool(true)
        XCTAssertNil(try RemoteResearchEntry(.object(row)).route(project: "p"))
        row["source_discarded"] = .bool(false); row["frame_id"] = .null
        XCTAssertNil(try RemoteResearchEntry(.object(row)).route(project: "p"))
        row["occurred_at"] = .string("not-a-timestamp")
        XCTAssertThrowsError(try RemoteResearchEntry(.object(row)))
    }
}
