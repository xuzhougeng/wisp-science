import Foundation
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeDecisionModelTests: XCTestCase {
    private func fixture() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-settings/v1/decision-model.json")))
    }

    func testSharedFixtureDecodesAndSaveArgumentsRoundTrip() throws {
        let fixture = try fixture()
        let settings = try NativeDecisionSettings.decode(fixture["settings"])
        XCTAssertEqual(settings.preset, "startlux_bamboo")
        XCTAssertEqual(settings.think_mode, "auto")
        XCTAssertTrue(settings.hasKey("typesafe"))
        XCTAssertEqual(try NativeDecisionSettings.decode(fixture["save_args"]["settings"]).value, fixture["save_args"]["settings"])
        XCTAssertEqual(try NativeDecisionSettings.decode(.object([:])), NativeDecisionSettings())
    }

    /// Mirrors `switching_presets_keeps_the_switch_and_stored_keys` in the WebView.
    func testSwitchingPresetsKeepsTheSwitchAndStoredKeys() throws {
        let catalog = NativeDecisionCatalog.shared
        XCTAssertEqual(catalog.presets.map(\.id), ["typesafe", "startlux_bamboo", "startlux_local", "custom"])
        XCTAssertEqual(catalog.think_modes, ["", "auto", "never", "always"])
        let current = NativeDecisionSettings(enabled: true, preset: "typesafe", base_url: "https://api.typesafe.ai", model: "jev-latest", think_mode: "", timeout_secs: 10, stored_keys: ["typesafe"])
        let bamboo = current.switched(to: try XCTUnwrap(catalog.preset("startlux_bamboo")))
        XCTAssertTrue(bamboo.enabled)
        XCTAssertEqual(bamboo.base_url, "https://api.bamboonode.dev")
        XCTAssertEqual(bamboo.model, "startlux-decision-27b-autothink")
        XCTAssertEqual(bamboo.timeout_secs, 60)
        XCTAssertEqual(bamboo.stored_keys, ["typesafe"])
        XCTAssertFalse(bamboo.usable(in: catalog), "Bamboo has no stored key yet")
        XCTAssertTrue(current.usable(in: catalog))
        XCTAssertTrue(current.switched(to: try XCTUnwrap(catalog.preset("startlux_local"))).usable(in: catalog))
    }

    func testStatusReadsLikeTheWebView() {
        let previous = UserDefaults.standard.object(forKey: "nativeSettings.locale")
        UserDefaults.standard.removeObject(forKey: "nativeSettings.locale")
        defer { if let previous { UserDefaults.standard.set(previous, forKey: "nativeSettings.locale") } }
        let catalog = NativeDecisionCatalog.shared
        var settings = NativeDecisionSettings(enabled: false, preset: "typesafe", base_url: "https://api.typesafe.ai", model: "jev-latest", think_mode: "", timeout_secs: 10, stored_keys: [])
        XCTAssertEqual(settings.status(in: catalog), catalog.text("decision.status_off"))
        settings.enabled = true
        XCTAssertEqual(settings.status(in: catalog), "已启用 TypeSafe Jev，但还没有保存 API key，暂不可用。")
        settings.stored_keys = ["typesafe"]
        XCTAssertEqual(settings.status(in: catalog), "使用中：TypeSafe Jev · jev-latest")
    }
}
