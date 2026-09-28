import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

final class NativeSettingsAlignmentRenderTests: XCTestCase {
    @MainActor func testSettingsWideNarrowThemesAndLanguages() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in settings rendering") }
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: true)
        let defaults = UserDefaults.standard
        let old = defaults.dictionaryRepresentation().filter { $0.key.hasPrefix("nativeSettings.") || $0.key == "projectBrowser.appearance" }
        defer {
            for key in defaults.dictionaryRepresentation().keys where key.hasPrefix("nativeSettings.") || key == "projectBrowser.appearance" { defaults.removeObject(forKey: key) }
            for (key, value) in old { defaults.set(value, forKey: key) }
        }
        for locale in ["zh", "en"] {
            for dark in [false, true] {
                for narrow in [false, true] {
                    for page in ["general", "appearance", "pet", "models", "subscriptions"] {
                        let client = SettingsAlignmentFixture(locale: locale, dark: dark)
                        let model = NativeSettingsModel(client: client, projectID: "fixture-project")
                        model.section = page == "subscriptions" ? .models : NativeSettingsSection(rawValue: page)!
                        if page == "subscriptions" { model.modelCategory = "subscriptions" }
                        await model.load()
                        defaults.set(locale, forKey: "nativeSettings.locale")
                        let host = NSHostingView(rootView: NativeSettingsView(model: model).environment(\.colorScheme, dark ? .dark : .light))
                        host.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
                        host.frame = NSRect(x: 0, y: 0, width: narrow ? 680 : 1100, height: 760)
                        host.layoutSubtreeIfNeeded()
                        for _ in 0..<20 { try await Task.sleep(nanoseconds: 10_000_000) }
                        host.layoutSubtreeIfNeeded()
                        let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                        host.cacheDisplay(in: host.bounds, to: bitmap)
                        let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                        let name = "\(page)-\(locale)-\(dark ? "dark" : "light")-\(narrow ? "narrow" : "wide").png"
                        try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent(name))
                        XCTAssertFalse(model.loading)
                        XCTAssertNil(model.error)
                    }
                }
            }
        }
    }
    @MainActor func testEmptyAndFailedSettingsViews() async throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in settings rendering") }
        for mode in ["empty", "error", "disabled"] {
            for page in ["pet", "models", "subscriptions"] {
                let model = NativeSettingsModel(client: SettingsAlignmentFixture(locale: "zh", dark: false, mode: mode), projectID: nil)
                model.section = page == "pet" ? .pet : .models
                if page == "subscriptions" { model.modelCategory = "subscriptions" }
                await model.load()
                let host = NSHostingView(rootView: NativeSettingsView(model: model).environment(\.colorScheme, .light))
                host.frame = NSRect(x: 0, y: 0, width: 680, height: 760)
                host.layoutSubtreeIfNeeded()
                for _ in 0..<20 { try await Task.sleep(nanoseconds: 10_000_000) }
                host.layoutSubtreeIfNeeded()
                let bitmap = try XCTUnwrap(host.bitmapImageRepForCachingDisplay(in: host.bounds))
                host.cacheDisplay(in: host.bounds, to: bitmap)
                let data = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                try data.write(to: URL(fileURLWithPath: directory).appendingPathComponent("\(page)-\(mode).png"))
                XCTAssertFalse(model.loading)
                if mode == "error" && page != "subscriptions" { XCTAssertNotNil(model.error) }
            }
        }
    }

}

private actor SettingsAlignmentFixture: NativeSettingsQuerying {
    let locale: String
    let dark: Bool
    let sprite: String
    let mode: String
    init(locale: String, dark: Bool, mode: String = "normal") {
        self.mode = mode
        self.locale = locale; self.dark = dark
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1536, pixelsHigh: 2288, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        for y in 40..<168 { for x in 32..<160 { bitmap.setColor(NSColor(deviceRed: 0.12, green: 0.60, blue: 0.55, alpha: 1), atX: x, y: y) } }
        sprite = "data:image/png;base64," + bitmap.representation(using: .png, properties: [:])!.base64EncodedString()
    }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        if mode == "error" && ["get_pet", "codex_subscription_status"].contains(command) { throw ProjectBrowserError.service("Synthetic read failure") }
        if mode == "disabled" && command == "get_pet" { return .object(["enabled": .bool(false), "directory": .string("/fixture/pet"), "asset": .null]) }
        if mode == "empty" {
            if command == "list_models" { return .array([]) }
            if command == "get_pet" { return .object(["asset": .null]) }
            if command == "codex_subscription_status" { return .object(["signed_in": .bool(false), "account_id": .string("")]) }
        }
        if mode == "error" && command == "list_models" { throw ProjectBrowserError.service("Synthetic model list failure") }
        switch command {
        case "get_settings": return .object(["locale": .string(locale), "resume_last_session": .bool(true), "notifications_enabled": .bool(true), "pet_enabled": .bool(mode == "normal"), "pet_directory": .string("/fixture/pets/research-companion"), "workspace_dir": .string("/fixture/research/workspace")])
        case "get_appearance_prefs": return .object(["theme": .string(dark ? "dark" : "light"), "light_palette": .string("paper"), "dark_palette": .string("charcoal"), "ui_font_size": .integer(14), "code_font_size": .integer(12), "send_with_modifier": .bool(true), "selection_popup_enabled": .bool(true)])
        case "get_update_check_enabled": return .bool(true)
        case "get_pet": return .object(["enabled": .bool(true), "directory": .string("/fixture/pets/research-companion"), "asset": .object(["id": .string("research-companion"), "displayName": .string("研究伙伴 Research companion"), "description": .string("Synthetic settings fixture · saved pet metadata"), "spriteVersionNumber": .integer(2), "spritesheetDataUrl": .string(sprite)])])
        case "get_pet_runtime_status": return .object(["running": .array([.string("run-1")]), "waiting": .array([]), "reviewing": .array([])])
        case "list_acp_agents": return .array([])
        case "list_models": return .array([
            .object(["id": .string("api-1"), "label": .string("科研项目的长名称模型配置 · Research workspace primary model"), "provider": .string("openai_responses"), "model": .string("vendor/research-model-with-a-long-exact-id"), "active": .bool(true), "supports_vision": .bool(true)]),
            .object(["id": .string("api-2"), "label": .string("离线验收 Secondary model"), "provider": .string("anthropic"), "model": .string("research-secondary"), "active": .bool(false)]),
            .object(["id": .string("sub-1"), "label": .string("订阅模型 Subscription research model"), "provider": .string("openai_codex"), "model": .string("subscription-model-exact-id"), "active": .bool(false)])])
        case "codex_subscription_status": return .object(["signed_in": .bool(args["provider"] == .string("codex")), "account_id": .string(args["provider"] == .string("codex") ? "synthetic-account-1" : "")])
        default: return .object([:])
        }
    }
}
