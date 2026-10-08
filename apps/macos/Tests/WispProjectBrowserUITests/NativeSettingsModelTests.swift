import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor SettingsFake: NativeSettingsQuerying {
    var writes: [(String, [String: SettingsValue], String?)] = []
    var persisted: SettingsValue = .object(["locale": .string("zh"), "notifications_enabled": .bool(true), "future_option": .string("preserve")])
    var fail = false
    var network: SettingsValue = .object(["model_proxy_url": .string("none"), "subscription_proxy_url": .string(""), "mcp_proxy_url": .string("none")])
    var held: CheckedContinuation<SettingsValue, Error>?
    var holdReads = false
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        if command == "get_network_settings" { return network }
        if command == "set_network_settings" {
            writes.append((command, args, projectID))
            network = args["settings"]!; return network
        }
        if command == "get_settings" {
            if holdReads { return try await withCheckedThrowingContinuation { held = $0 } }
            return persisted
        }
        if command == "set_settings" {
            writes.append((command, args, projectID))
            if fail { throw ProjectBrowserError.service("Save rejected") }
            persisted = args["settings"]!; return .null
        }
        return .object([:])
    }
    func setFail() { fail = true }
    func writeCount() -> Int { writes.count }
    func lastWrite() -> (String, [String: SettingsValue], String?)? { writes.last }
    func hold() { holdReads = true }
    func isHeld() -> Bool { held != nil }
    func finish() { holdReads = false; held?.resume(returning: .object(["locale": .string("stale")])); held = nil }
}

final class NativeSettingsModelTests: XCTestCase {
    @MainActor func testSubscriptionProxySavePreservesOtherNetworkRoutes() async {
        let client = SettingsFake()
        let model = NativeSettingsModel(client: client, projectID: nil)
        model.section = .network
        await model.load()
        model.binding("get_network_settings", "subscription_proxy_url").wrappedValue = .string("http://localhost:7897")
        _ = await model.run("set_network_settings", ["settings": model.values["get_network_settings"]!])
        let write = await client.lastWrite()
        XCTAssertEqual(write?.0, "set_network_settings")
        XCTAssertEqual(write?.1["settings"]?["subscription_proxy_url"], .string("http://localhost:7897"))
        XCTAssertEqual(write?.1["settings"]?["model_proxy_url"], .string("none"))
        XCTAssertEqual(write?.1["settings"]?["mcp_proxy_url"], .string("none"))
        XCTAssertNil(write?.2)
        XCTAssertEqual(model.values["get_network_settings"]?["subscription_proxy_url"], .string("http://localhost:7897"))
    }
    @MainActor func testProjectStoragePreferenceSavesAndReloadsBothModes() async {
        let client = SettingsFake(); let model = NativeSettingsModel(client: client, projectID: nil)
        await model.load()
        for enabled in [true, false] {
            model.binding("get_settings", "decentralized_project_storage").wrappedValue = .bool(enabled)
            await model.saveSettings()
            let write = await client.lastWrite()
            XCTAssertEqual(write?.1["settings"]?["decentralized_project_storage"], .bool(enabled))
            await model.load()
            XCTAssertEqual(model.values["get_settings"]?["decentralized_project_storage"], .bool(enabled))
        }
    }

    @MainActor func testSessionDefaultsDoNotOverwriteUnknownFieldsAndCancelRestoresDraft() async {
        let client = SettingsFake(); let model = NativeSettingsModel(client: client, projectID: "project-a")
        model.section = .session; await model.load()
        XCTAssertEqual(model.numberText("max_iter"), "100")
        XCTAssertEqual(model.numberText("auto_continue_limit"), "10")
        XCTAssertEqual(model.numberText("semantic_compact_idle_hours"), "24")
        model.binding("get_settings", "semantic_compact_on_model_switch").wrappedValue = .bool(true)
        model.binding("get_settings", "semantic_compact_idle_hours").wrappedValue = .integer(0)
        model.binding("get_settings", "max_iter").wrappedValue = .integer(0)
        model.binding("get_settings", "auto_continue_limit").wrappedValue = .integer(1)
        await model.saveSettings()
        let write = await client.lastWrite()
        XCTAssertEqual(write?.1["settings"]?["semantic_compact_idle_hours"], .integer(0))
        XCTAssertEqual(write?.1["settings"]?["semantic_compact_on_model_switch"], .bool(true))
        XCTAssertEqual(write?.1["settings"]?["future_option"], .string("preserve"))
        XCTAssertFalse(model.hasUnsavedChanges)
        model.binding("get_settings", "semantic_compact_idle_hours").wrappedValue = .integer(12)
        model.discardDrafts()
        XCTAssertEqual(model.numberText("semantic_compact_idle_hours"), "0")
    }

    @MainActor func testInvalidSessionNumbersDoNotReachHostOrDiscardDraft() async {
        for key in ["max_iter", "auto_continue_limit", "semantic_compact_idle_hours"] {
            for invalid in [SettingsValue.null, .string("abc"), .string("1.5"), .string("9223372036854775808"), .integer(-1)] {
                let client = SettingsFake(); let model = NativeSettingsModel(client: client, projectID: nil)
                await model.load()
                model.binding("get_settings", key).wrappedValue = invalid
                await model.saveSettings()
                let count = await client.writeCount()
                XCTAssertEqual(count, 0, key)
                XCTAssertNotNil(model.error)
                XCTAssertEqual(model.values["get_settings"]?[key], invalid)
                XCTAssertTrue(model.hasUnsavedChanges)
            }
        }
        let client = SettingsFake(); let model = NativeSettingsModel(client: client, projectID: nil)
        await model.load()
        model.binding("get_settings", "auto_continue").wrappedValue = .bool(false)
        model.binding("get_settings", "auto_continue_limit").wrappedValue = .integer(0)
        await model.saveSettings()
        let count = await client.writeCount()
        XCTAssertEqual(count, 0, "Disabled control must still preserve a valid saved limit")
    }

    @MainActor func testSyncChoiceUsesStoredValueAndPreservesUnknownBackend() async {
        let model = NativeSettingsModel(client: SettingsFake(), projectID: nil)
        await model.load()
        for backend in ["folder", "relay", "future-backend"] {
            model.binding("get_settings", "sync_backend").wrappedValue = .string(backend)
            XCTAssertEqual(NativeSettingsChoice.selectedTitle(backend, choices: [("relay", "中继服务"), ("folder", "同步文件夹")]), ["folder": "同步文件夹", "relay": "中继服务"][backend] ?? backend)
            await model.saveSettings()
            XCTAssertEqual(model.values["get_settings"]?["sync_backend"].string, backend)
        }
    }

    @MainActor func testDraftSurvivesTabsAndSaveKeepsUneditedFieldsAndProject() async {
        let client = SettingsFake(); let model = NativeSettingsModel(client: client, projectID: "project-a")
        await model.load()
        model.binding("get_settings", "locale").wrappedValue = .string("en")
        XCTAssertTrue(model.hasUnsavedChanges)
        model.section = .session; await model.load()
        XCTAssertEqual(model.values["get_settings"]?["locale"], .string("en"))
        await model.saveSettings()
        let write = await client.lastWrite()
        XCTAssertEqual(write?.2, "project-a")
        XCTAssertEqual(write?.1["settings"]?["future_option"], .string("preserve"))
        XCTAssertFalse(model.hasUnsavedChanges)
        XCTAssertNil(model.error)
    }
    @MainActor func testRejectedSaveKeepsDraftWithoutRetry() async {
        let client = SettingsFake(); let model = NativeSettingsModel(client: client, projectID: nil)
        await model.load(); model.binding("get_settings", "locale").wrappedValue = .string("en")
        await client.setFail(); await model.saveSettings()
        let count = await client.writeCount(); XCTAssertEqual(count, 1)
        XCTAssertNotNil(model.error); XCTAssertTrue(model.hasUnsavedChanges)
        XCTAssertEqual(model.values["get_settings"]?["locale"], .string("en"))
        model.discardDrafts(); XCTAssertFalse(model.hasUnsavedChanges)
        XCTAssertEqual(model.values["get_settings"]?["locale"], .string("zh"))
        XCTAssertNil(model.error)
        XCTAssertNil(model.message)
    }
    @MainActor func testLeavingIgnoresLateRead() async {
        let client = SettingsFake(); let model = NativeSettingsModel(client: client, projectID: "a")
        await client.hold()
        let loading = Task { await model.load() }
        while !(await client.isHeld()) { await Task.yield() }
        model.leave(); await client.finish(); await loading.value
        XCTAssertNil(model.values["get_settings"])
    }
    @MainActor func testModelCategorySurvivesSaveRefreshAndEditorDismissal() async {
        let model = NativeSettingsModel(client: SettingsFake(), projectID: nil)
        model.section = .models
        model.modelCategory = "subscriptions"
        model.editor = SettingsEditor(title: "编辑模型", draft: .object([:]), fields: [], command: "save_model")
        _ = await model.run("save_model", ["profile": .object([:])])
        model.editor = nil
        XCTAssertEqual(model.modelCategory, "subscriptions")
    }

    func testSettingsSectionsMatchSharedNavigation() {
        XCTAssertEqual(NativeSettingsSection.allCases.count, 21)
        XCTAssertEqual(NativeSettingsSection.allCases.map(\.rawValue), ["general", "network", "session", "appearance", "pet", "models", "quick-actions", "hooks", "workflows", "specialists", "memory", "skills", "plugins", "browser", "connections", "channels", "credentials", "permissions", "environments", "storage", "usage"])
        XCTAssertEqual(NativeSettingsSection.general.reads, ["get_settings", "get_appearance_prefs", "get_bootstrap_status", "get_update_check_enabled"])
        XCTAssertEqual(NativeSettingsSection.network.reads, ["get_network_settings"])
    }
    func testSubscriptionSearchAndClassificationKeepApiProfilesSeparate() {
        XCTAssertTrue(NativeSettingsSection.models.matches("ChatGPT 登录"))
        for provider in ["openai_codex", "openai-codex", "codex", "xai_oauth", "xai-oauth", "xai"] {
            XCTAssertTrue(NativeModelSettings.isSubscription(.object(["provider": .string(provider)])))
        }
        for provider in ["openai", "openai_responses", "anthropic"] {
            XCTAssertFalse(NativeModelSettings.isSubscription(.object(["provider": .string(provider)])))
        }
    }
    func testApiReorderSkipsSubscriptionsWithoutMovingTheirSlots() {
        func row(_ id: String, _ provider: String) -> SettingsValue { .object(["id": .string(id), "provider": .string(provider)]) }
        let rows = [row("a", "openai"), row("subscription", "openai_codex"), row("b", "anthropic")]
        let result = NativeModelSettings.reorderAPIModels(rows, id: "b", offset: -1)
        XCTAssertEqual(result.map { $0["id"].string }, ["b", "subscription", "a"])
        XCTAssertEqual(NativeModelSettings.reorderAPIModels(rows, id: "a", offset: -1), rows)
    }
    func testSearchUsesWebViewAliasesAndRequiresEveryTerm() {
        XCTAssertTrue(NativeSettingsSection.channels.matches("同步"))
        XCTAssertTrue(NativeSettingsSection.models.matches("API key"))
        XCTAssertTrue(NativeSettingsSection.environments.matches(" SSH  runtime "))
        XCTAssertFalse(NativeSettingsSection.appearance.matches("theme ssh"))
        XCTAssertTrue(NativeSettingsSection.network.matches("网络"))
        XCTAssertTrue(NativeSettingsSection.network.matches("proxy"))
        XCTAssertFalse(NativeSettingsSection.general.matches("proxy"))
        XCTAssertFalse(NativeSettingsSection.models.matches("proxy"))
        XCTAssertEqual(Set(NativeSettingsSection.allCases.map(\.group)), ["基础偏好", "AI 配置", "工具与连接", "系统与资源"])
    }

    func testEveryExportedThemeHasEveryPreviewToken() {
        for (_, palette) in WispDesign.palettes {
            for token in ["bg-app", "bg-elev", "bg-sunken", "text", "text-muted", "border", "clay"] {
                XCTAssertNotNil(palette[token], "Missing preview token: \(token)")
            }
        }
        XCTAssertEqual(WispDesign.modelPresets.count, 6)
        XCTAssertTrue(WispDesign.modelPresets.allSatisfy { $0["url"]?.hasPrefix("https://") == true })
    }

}
