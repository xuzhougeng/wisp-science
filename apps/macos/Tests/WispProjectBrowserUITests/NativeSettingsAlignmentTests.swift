import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor AccountStatusFixture: NativeSettingsQuerying {
    var calls: [(String, String?)] = []
    var failCodex = true
    var held: CheckedContinuation<SettingsValue, Error>?
    var hold = false
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        XCTAssertEqual(command, "codex_subscription_status")
        calls.append((args["provider"]!.string, projectID))
        if hold { return try await withCheckedThrowingContinuation { held = $0 } }
        if args["provider"] == .string("codex") && failCodex { throw ProjectBrowserError.service("Offline fixture failure") }
        return .object(["signed_in": .bool(args["provider"] == .string("codex")), "account_id": .string("synthetic-account")])
    }
    func recover() { failCodex = false }
    func suspend() { hold = true }
    func waiting() -> Bool { held != nil }
    func finish() { hold = false; held?.resume(returning: .object(["signed_in": .bool(true), "account_id": .string("stale")])); held = nil }
    func history() -> [(String, String?)] { calls }
}

final class NativeSettingsAlignmentTests: XCTestCase {
    @MainActor func testAccountFailureIsNotSignedOutAndRetryIsGlobal() async {
        let client = AccountStatusFixture()
        let state = NativeSubscriptionAccounts()
        await state.load(client)
        XCTAssertNotNil(state.errors["codex"])
        XCTAssertNil(state.accounts["codex"])
        XCTAssertEqual(state.accounts["xai"]?.signed_in, false)
        XCTAssertFalse(state.loading)
        await client.recover()
        await state.load(client)
        XCTAssertTrue(state.errors.isEmpty)
        XCTAssertEqual(state.accounts["codex"]?.account_id, "synthetic-account")
        XCTAssertEqual(state.accounts["codex"]?.signed_in, true)
        let calls = await client.history()
        XCTAssertEqual(calls.map(\.0), ["codex", "xai", "codex", "xai"])
        XCTAssertTrue(calls.allSatisfy { $0.1 == nil })
    }

    @MainActor func testLeavingDiscardsLateAccountStatusAndDoesNotStartNextRead() async {
        let client = AccountStatusFixture()
        await client.suspend()
        let state = NativeSubscriptionAccounts()
        let task = Task { await state.load(client) }
        while !(await client.waiting()) { await Task.yield() }
        XCTAssertTrue(state.loading)
        state.invalidate()
        await client.finish()
        await task.value
        XCTAssertTrue(state.accounts.isEmpty)
        XCTAssertTrue(state.errors.isEmpty)
        XCTAssertFalse(state.loading)
        let calls = await client.history()
        XCTAssertEqual(calls.count, 1)
    }

    @MainActor func testPetSavePreservesAppearanceDraftAndCancelRestoresBothDocuments() async {
        let client = SettingsDocumentsFixture()
        let model = NativeSettingsModel(client: client, projectID: nil)
        await model.load()
        model.binding("get_settings", "pet_enabled").wrappedValue = .bool(true)
        model.binding("get_appearance_prefs", "ui_font_family").wrappedValue = .string("Menlo")
        await model.saveSettings()
        XCTAssertEqual(model.values["get_settings"]?["pet_enabled"], .bool(true))
        XCTAssertEqual(model.values["get_settings"]?["future_option"], .string("preserved"))
        XCTAssertEqual(model.values["get_appearance_prefs"]?["ui_font_family"], .string("Menlo"))
        XCTAssertTrue(model.hasUnsavedChanges)
        _ = await model.run("set_appearance_prefs", ["prefs": model.values["get_appearance_prefs"]!])
        XCTAssertFalse(model.hasUnsavedChanges)
        XCTAssertEqual(model.values["get_appearance_prefs"]?["future_palette"], .string("preserved"))
        model.binding("get_settings", "pet_directory").wrappedValue = .string("/unsaved")
        model.binding("get_appearance_prefs", "theme").wrappedValue = .string("dark")
        model.discardDrafts()
        XCTAssertEqual(model.values["get_settings"]?["pet_directory"], .string("/fixture/pet"))
        XCTAssertEqual(model.values["get_appearance_prefs"]?["theme"], .string("light"))
        XCTAssertFalse(model.hasUnsavedChanges)
        let writes = await client.writeCount()
        XCTAssertEqual(writes, 2, "Cancel must not write or compensate persisted settings")
    }

    @MainActor func testAppearanceSaveFailureKeepsPreviewDraftForCorrection() async {
        let client = SettingsDocumentsFixture()
        let model = NativeSettingsModel(client: client, projectID: nil)
        model.section = .appearance
        await model.load()
        model.binding("get_appearance_prefs", "theme").wrappedValue = .string("dark")
        await client.reject()
        let result = await model.run("set_appearance_prefs", ["prefs": model.values["get_appearance_prefs"]!])
        XCTAssertNil(result)
        XCTAssertNotNil(model.error)
        XCTAssertTrue(model.hasUnsavedChanges)
        XCTAssertEqual(model.values["get_appearance_prefs"]?["theme"], .string("dark"))
        model.discardDrafts()
        XCTAssertEqual(model.values["get_appearance_prefs"]?["theme"], .string("light"))
    }

    @MainActor func testPetPreviewRequiresValidatedSpriteDimensionsAndUsesOneFrame() throws {
        XCTAssertNil(NativePetSettings.firstFrame("https://example.invalid/pet.png"))
        XCTAssertNil(NativePetSettings.firstFrame("data:image/png;base64,invalid"))
        let bitmap = try XCTUnwrap(NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 1536, pixelsHigh: 2288, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        let bytes = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        let frame = try XCTUnwrap(NativePetSettings.firstFrame("data:image/png;base64," + bytes.base64EncodedString()))
        XCTAssertEqual(frame.size, NSSize(width: 192, height: 208))
    }
}

private actor SettingsDocumentsFixture: NativeSettingsQuerying {
    var settings: SettingsValue = .object(["pet_enabled": .bool(false), "pet_directory": .string("/fixture/pet"), "future_option": .string("preserved")])
    var appearance: SettingsValue = .object(["theme": .string("light"), "ui_font_family": .string(""), "future_palette": .string("preserved")])
    var writes = 0
    var fail = false
    func reject() { fail = true }
    func writeCount() -> Int { writes }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        switch command {
        case "get_settings": return settings
        case "get_appearance_prefs": return appearance
        case "set_settings": writes += 1; settings = args["settings"]!; return .null
        case "set_appearance_prefs":
            writes += 1
            if fail { throw ProjectBrowserError.service("Offline fixture rejected save") }
            appearance = args["prefs"]!; return appearance
        default: return .object([:])
        }
    }
}
