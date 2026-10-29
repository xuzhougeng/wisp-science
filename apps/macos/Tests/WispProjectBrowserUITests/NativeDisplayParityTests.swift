import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

/// Display rules the SwiftUI client copies from the WebView (2026-10-29 parity pass).
final class NativeDisplayParityTests: XCTestCase {
    private var previousLocale: Any?
    override func setUp() {
        previousLocale = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        UserDefaults.standard.removeObject(forKey: "nativeSettings.locale")
    }
    override func tearDown() {
        if let previousLocale { UserDefaults.standard.set(previousLocale, forKey: "nativeSettings.locale") } else { UserDefaults.standard.removeObject(forKey: "nativeSettings.locale") }
    }

    func testBytesUseTheWebViewBinaryUnits() {
        XCTAssertEqual(formatBytes(0), "0 B")
        XCTAssertEqual(formatBytes(1023), "1023 B")
        XCTAssertEqual(formatBytes(218_112), "213.0 KB")
        XCTAssertEqual(formatBytes(537_000_000), "512.1 MB")
        XCTAssertEqual(formatBytes(3_840_000_000), "3.6 GB")
    }

    func testDailyUsageDropsThePaddedFutureDays() {
        let days: [SettingsValue] = [
            .object(["date": .string("2026-10-29"), "tokens": .integer(12), "future": .bool(false)]),
            .object(["date": .string("2026-10-30"), "tokens": .integer(0), "future": .bool(true)]),
        ]
        let rows = NativeUsageRows.pastDays(days)
        XCTAssertEqual(rows.map(\.label), ["2026-10-29"])
        XCTAssertEqual(rows.map(\.value), [12])
    }

    func testModelUsageMergesProfilesByModelIDAndFoldsTheTail() {
        let profiles: [SettingsValue] = [
            .object(["id": .string("m1"), "label": .string("Kimi"), "model": .string("K3")]),
            .object(["id": .string("m2"), "label": .string("GLM"), "model": .string("  ")]),
        ]
        var rows: [SettingsValue] = [
            .object(["model": .string("K3"), "tokens": .integer(2)]),
            .object(["model": .string("m1"), "tokens": .integer(6)]),
            .object(["model": .string("m2"), "tokens": .integer(5)]),
            .object(["model": .string("unknown"), "tokens": .integer(4)]),
            .object(["model": .string("zero"), "tokens": .integer(0)]),
        ]
        let merged = NativeUsageRows.models(rows, profiles: profiles)
        XCTAssertEqual(merged.map(\.label), ["K3", "GLM", "未知模型"])
        XCTAssertEqual(merged.map(\.value), [8, 5, 4])

        rows = (1...9).map { .object(["model": .string("p\($0)"), "tokens": .integer(Int64(100 - $0))]) }
        let folded = NativeUsageRows.models(rows, profiles: [])
        XCTAssertEqual(folded.count, 8)
        XCTAssertEqual(folded.last?.label, "其他模型")
        XCTAssertEqual(folded.last?.value, 92 + 91)
    }

    func testStorageEntriesUseLegendNames() {
        XCTAssertEqual(NativeUsageRows.storageLabel("database"), "数据库（会话与工具结果）")
        XCTAssertEqual(NativeUsageRows.storageLabel("workspace"), "工作区文件")
        XCTAssertEqual(NativeUsageRows.storageLabel("cache"), "cache")
    }

    func testNewProjectNeedsNameAndDirectory() {
        var draft = NewProjectDraft()
        XCTAssertFalse(draft.canSubmit)
        draft.name = "  Demo "
        XCTAssertFalse(draft.canSubmit)
        draft.directory = " \n"
        XCTAssertFalse(draft.canSubmit)
        draft.directory = "/tmp/demo"
        XCTAssertTrue(draft.canSubmit)
    }

    func testTrajectoryUsageRowsReadLikeTheWebView() throws {
        let usage = try JSONDecoder().decode(NativeTrajectoryCell.self, from: Data("""
        {"kind":"usage","summary":"round 1 · 12845 in / 117 out","is_error":false,
         "usage":{"round":1,"input_tokens":12845,"output_tokens":117,"reasoning_tokens":0,"cached_input_tokens":10019}}
        """.utf8))
        XCTAssertEqual(usage.displaySummary, "第 1 轮 · 输入 12.8k · 输出 117 · 缓存 78%")
        XCTAssertEqual(usage.sourceLabel, "用量")
        let user = try JSONDecoder().decode(NativeTrajectoryCell.self, from: Data(#"{"kind":"user","summary":"hello","is_error":false}"#.utf8))
        XCTAssertEqual(user.displaySummary, "hello")
        XCTAssertEqual(user.sourceLabel, "用户")
    }

    func testComposerEffortAndBuiltinQuickActionAreLocalized() {
        XCTAssertEqual(NativeComposerEffort.label("high"), "高")
        XCTAssertEqual(NativeComposerEffort.label("xhigh"), "极高")
        XCTAssertEqual(NativeComposerEffort.label("turbo"), "turbo")
        let builtin = SettingsValue.object(["id": .string("literature_research"), "name": .string("Research literature")])
        XCTAssertEqual(NativeWorkflowSettings.displayName(builtin), "文献调研")
        let renamed = SettingsValue.object(["id": .string("literature_research"), "name": .string("我的文献")])
        XCTAssertEqual(NativeWorkflowSettings.displayName(renamed), "我的文献")
    }
}
