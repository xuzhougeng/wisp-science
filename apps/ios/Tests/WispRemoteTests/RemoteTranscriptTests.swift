import XCTest
@testable import WispRemote

final class RemoteTranscriptTests: XCTestCase {
    private func item(_ role: String, _ text: String = "", tool: String = "", ok: Bool? = nil) -> RemoteValue {
        .object(["role": .string(role), "text": .string(text), "tool_name": .string(tool),
                 "ok": ok.map(RemoteValue.bool) ?? .null])
    }

    func testToolStepsAndChangedPathsCollapseWithoutHidingTheAnswer() {
        let items = [
            item("user", "绘制表达热图"), item("reasoning", "检查数据"), item("assistant", "准备绘图"),
            item("tool", "done", tool: "shell", ok: true),
            item("file_changed", "figures/heatmap.png"), item("file_changed", "figures/heatmap.pdf"),
            item("acp_tool", "missing", tool: "read", ok: false),
            item("file_changed", "figures/heatmap.png"),
            item("assistant", "# 分析完成\n\n[查看报告](results/report.md)"), item("usage", "{}"),
        ]
        let groups = RemoteTranscript.groups(items)
        XCTAssertEqual(groups.map(\.isProcess), [false, true, false, false])
        XCTAssertEqual(groups[1].toolCount, 2)
        XCTAssertEqual(groups[1].changedFileCount, 2, "Count distinct paths, not write notifications")
        XCTAssertEqual(groups[1].failureCount, 1)
        XCTAssertEqual(groups[2].items, [items[8]])
        XCTAssertEqual(groups.flatMap(\.items), items, "Grouping must preserve every item and its order")
    }

    func testQuestionsPlansWarningsAndUnknownRolesRemainVisibleAndSeparateTurns() {
        for role in ["question", "plan", "system", "user", "queued_user", "review", "future-role"] {
            let boundary = item(role, "需要你查看")
            let items = [item("tool", tool: "read"), boundary, item("tool", tool: "edit")]
            let groups = RemoteTranscript.groups(items)
            XCTAssertEqual(groups.map(\.isProcess), [true, false, true], role)
            XCTAssertEqual(groups[1].items, [boundary], role)
        }
    }

    func testResultToolsAndAssistantResourcesAreNotFoldedAway() {
        for tool in ["attempt_completion", "monitor_run", "wisp_monitor_run", "generate_image", "generate_video"] {
            let items = [item("reasoning"), item("tool", tool: tool, ok: true), item("file_changed", "figure.png")]
            XCTAssertEqual(RemoteTranscript.groups(items).map(\.isProcess), [true, false, true], tool)
        }
        let resource: RemoteValue = .object(["role": .string("assistant"), "text": .string("![图](fig.png)"),
                                           "resources": .array([.object(["id": .string("image-a")])])])
        XCTAssertEqual(RemoteTranscript.groups([resource, item("tool", tool: "read")]).map(\.isProcess), [false, true])
    }

    func testCurrentCommentaryRemainsReadableWhilePastWorkStaysGrouped() {
        let items = [item("user", "上一轮"), item("assistant", "旧进度"), item("tool", tool: "read"),
                     item("assistant", "旧回答"), item("user", "这一轮"), item("assistant", "正在绘图"),
                     item("tool", tool: "shell"), item("file_changed", "figure.png")]
        let live = RemoteTranscript.groups(items, running: true)
        XCTAssertEqual(live.map(\.isProcess), [false, true, false, false, false, true])
        XCTAssertEqual(live[4].items, [items[5]])
        let finished = RemoteTranscript.groups(items)
        XCTAssertEqual(finished.last?.items, Array(items[5...]))
        XCTAssertTrue(RemoteTranscript.groups([], running: true).isEmpty)
    }

    func testGroupIdentitySurvivesAppendedStepsAndEarlierPages() {
        let live = [item("user", "当前问题"), item("tool", tool: "shell")]
        let original = RemoteTranscript.groups(live)
        let extended = RemoteTranscript.groups(live + [item("tool", tool: "read"), item("file_changed", "data.csv")])
        XCTAssertEqual(original.last?.id, extended.last?.id)
        let older = [item("user", "旧问题"), item("tool", tool: "read"), item("assistant", "旧回答")]
        let paged = RemoteTranscript.groups(older + live, indexOffset: -older.count)
        XCTAssertEqual(Array(paged.suffix(original.count)), original)
        XCTAssertEqual(Set(paged.map(\.id)).count, paged.count)
    }

    /// Rows in the shared contract's shape, with positions relative to `items`.
    private func contractRow(_ row: RemoteTranscript.Row, items: [RemoteValue], offset: Int) -> RemoteValue {
        func indices(_ group: RemoteTranscript.Group) -> RemoteValue {
            let start = group.id - offset
            if start >= 0 && start + group.items.count <= items.count {
                XCTAssertEqual(group.items, Array(items[start..<start + group.items.count]), "A fold keeps its items in order")
            } else { XCTFail("Fold \(group.id) is outside the items") }
            return .array(group.items.indices.map { .integer(Int64(start + $0)) })
        }
        switch row {
        case .item(let id, let item):
            XCTAssertEqual(items.indices.contains(id - offset) ? items[id - offset] : nil, item)
            return .object(["kind": .string("item"), "index": .integer(Int64(id - offset))])
        case .process(let group):
            return .object(["kind": .string("process"), "indices": indices(group), "tools": .integer(Int64(group.toolCount)),
                            "files": .integer(Int64(group.changedFileCount)), "failures": .integer(Int64(group.failureCount))])
        case .failures(let group):
            return .object(["kind": .string("failures"), "indices": indices(group), "count": .integer(Int64(group.failureCount))])
        }
    }

    func testDetailLevelsMatchTheSharedMobileContract() throws {
        let contract = try fixture("native-conversations/v1/transcript-detail.json")
        let levels = RemoteTranscript.DetailLevel.allCases
        XCTAssertEqual(levels.map(\.rawValue), contract["levels"].array.map(\.string))
        XCTAssertEqual(RemoteTranscript.DetailLevel.default.rawValue, contract["default_level"].string)
        for level in levels { XCTAssertEqual(level.label, contract["labels"][level.rawValue].string) }
        let cases = contract["cases"].array
        XCTAssertFalse(cases.isEmpty)
        for sample in cases {
            let items = sample["items"].array
            let running = sample["running"].bool
            for level in levels {
                let expected = sample["expect"][level.rawValue].array
                XCTAssertFalse(expected.isEmpty, "\(sample["name"].string) has no \(level.rawValue) rows")
                // An earlier history page shifts identities but never the rows themselves.
                for offset in [0, -3] {
                    let rows = RemoteTranscript.rows(items, level: level, running: running, indexOffset: offset)
                    XCTAssertEqual(rows.map { contractRow($0, items: items, offset: offset) }, expected,
                                   "\(sample["name"].string) · \(level.rawValue) · offset \(offset)")
                    XCTAssertEqual(Set(rows.map(\.id)).count, rows.count)
                }
            }
            let standard = RemoteTranscript.rows(items, level: .standard, running: running)
            XCTAssertEqual(standard.map(\.id), RemoteTranscript.groups(items, running: running).map(\.id),
                           "标准 keeps the existing grouping and identities")
        }
    }
}
