import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor LoginTransport: NativeSettingsQuerying {
    let fixture: SettingsValue
    var calls: [(String, [String: SettingsValue], String?)] = []
    var failure: String?
    var hanging: String?
    var overrides: [String: SettingsValue] = [:]
    private var continuations: [String: CheckedContinuation<SettingsValue, Error>] = [:]
    init(_ fixture: SettingsValue) { self.fixture = fixture }
    func fail(_ command: String?) { failure = command }
    func hang(_ command: String) { hanging = command }
    func allowNewRequests() { hanging = nil }
    func pending(_ command: String) -> Bool { continuations[command] != nil }
    func respond(_ command: String, _ value: SettingsValue) { overrides[command] = value }
    func finish(_ command: String, _ value: SettingsValue) { continuations.removeValue(forKey: command)?.resume(returning: value); hanging = nil }
    func count(_ command: String) -> Int { calls.filter { $0.0 == command }.count }
    func last(_ command: String) -> (String, [String: SettingsValue], String?)? { calls.last { $0.0 == command } }
    func allGlobal() -> Bool { calls.allSatisfy { $0.2 == nil } }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String?) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if failure == command { throw ProjectBrowserError.service("fake connection lost") }
        if hanging == command { return try await withCheckedThrowingContinuation { continuations[command] = $0 } }
        if let value = overrides[command] { return value }
        switch command {
        case "codex_subscription_status": return fixture["saved_account"]
        case "start_codex_login": return fixture[args["method"]?.string ?? "browser"]
        case "codex_login_status": return fixture["pending"]
        case "submit_codex_login_redirect": return fixture["success"]
        case "save_codex_login": return fixture["saved_models"]
        case "cancel_codex_login": return .null
        default: throw ProjectBrowserError.service("unexpected command")
        }
    }
}

final class NativeCodexLoginTests: XCTestCase {
    private func fixture() throws -> SettingsValue {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return try JSONDecoder().decode(SettingsValue.self, from: Data(contentsOf: root.appendingPathComponent("contracts/native-settings/v1/codex-login.json")))
    }
    @MainActor private func waitPending(_ client: LoginTransport, _ command: String) async {
        for _ in 0..<1000 { if await client.pending(command) { return }; await Task.yield() }
        XCTFail("Request was not suspended")
    }

    @MainActor func testBrowserRedirectAndProfileSaveUseTheSharedGlobalContract() async throws {
        let data = try fixture(); let client = LoginTransport(data)
        let profile: SettingsValue = .object(["id": .string("subscription-test"), "model": .string("fixture-model"), "label": .string("My subscription"), "api_url": .string("https://chatgpt.com/backend-api")])
        let model = NativeCodexLoginModel(client: client, profile: profile, automaticallyPoll: false)
        await model.loadSavedAccount()
        XCTAssertEqual(model.savedAccount?.account_id, "account-test")
        let url = await model.start()
        XCTAssertEqual(url?.absoluteString, data["browser"]["url"].string)
        model.redirect = "http://localhost:1455/auth/callback?code=fake&state=test-state"
        let redirect = model.redirect
        await model.submitRedirect()
        XCTAssertEqual(model.phase, .success); XCTAssertTrue(model.redirect.isEmpty)
        let submitted = await client.last("submit_codex_login_redirect")
        XCTAssertEqual(submitted?.1, ["loginId": .string("browser-test"), "redirect": .string(redirect)])
        let result = await model.save()
        XCTAssertEqual(result, data["saved_models"])
        XCTAssertEqual(model.phase, .saved)
        let saved = await client.last("save_codex_login")
        XCTAssertEqual(saved?.1, ["loginId": .string("browser-test"), "model": .string("fixture-model"), "label": .string("My subscription"), "profileId": .string("subscription-test"), "apiUrl": .string("https://chatgpt.com/backend-api"), "useSaved": .bool(false)])
        _ = await model.save()
        let saves = await client.count("save_codex_login"); XCTAssertEqual(saves, 1)
        let global = await client.allGlobal(); XCTAssertTrue(global)
    }

    @MainActor func testDeviceCodePendingSuccessAndExpiredAttempts() async throws {
        let data = try fixture(); let client = LoginTransport(data)
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false); model.method = "device"
        let url = await model.start(); XCTAssertNil(url)
        XCTAssertEqual(model.challenge?.user_code, "TEST-1234")
        XCTAssertEqual(model.phase, .pending); XCTAssertFalse(model.canSave)
        await model.pollOnce(); XCTAssertEqual(model.phase, .pending)
        await client.respond("codex_login_status", data["expired"])
        await model.pollOnce()
        XCTAssertEqual(model.phase, .failed); XCTAssertFalse(model.canSave)
        XCTAssertTrue(model.error?.contains("timed out") == true)
        _ = await model.start()
        let canceled = await client.count("cancel_codex_login"); XCTAssertEqual(canceled, 1)
        await client.respond("codex_login_status", data["success"])
        await model.pollOnce()
        XCTAssertEqual(model.phase, .success); XCTAssertEqual(model.accountID, "account-test")
    }

    @MainActor func testCancelDuringStartCancelsTheLateChallengeWithoutReopening() async throws {
        let data = try fixture(); let client = LoginTransport(data)
        await client.hang("start_codex_login")
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
        let start = Task { await model.start() }
        await waitPending(client, "start_codex_login")
        _ = await model.cancel()
        await client.finish("start_codex_login", data["browser"])
        let url = await start.value
        XCTAssertNil(url); XCTAssertEqual(model.phase, .closed); XCTAssertNil(model.challenge)
        let canceled = await client.last("cancel_codex_login")
        XCTAssertEqual(canceled?.1, ["loginId": .string("browser-test")])
        let count = await client.count("cancel_codex_login"); XCTAssertEqual(count, 1)
    }

    @MainActor func testLatePollCannotRestoreClosedFormOrDowngradeManualSuccess() async throws {
        for close in [false, true] {
            let data = try fixture(); let client = LoginTransport(data)
            let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
            _ = await model.start(); await client.hang("codex_login_status")
            let polling = Task { await model.pollOnce() }
            await waitPending(client, "codex_login_status")
            if close { _ = await model.cancel() }
            else { model.redirect = "fake-code"; await model.submitRedirect() }
            await client.finish("codex_login_status", data[close ? "success" : "pending"])
            await polling.value
            XCTAssertEqual(model.phase, close ? .closed : .success)
            if close { XCTAssertNil(model.challenge); XCTAssertTrue(model.accountID.isEmpty) }
        }
    }

    @MainActor func testSavedAccountAndLostSaveDoNotStartLoginOrRetry() async throws {
        let client = LoginTransport(try fixture())
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
        await model.loadSavedAccount(); await client.fail("save_codex_login")
        let result = await model.save(useSaved: true)
        XCTAssertNil(result); XCTAssertEqual(model.phase, .failed)
        XCTAssertTrue(model.error?.contains("不会自动重试") == true)
        let saved = await client.last("save_codex_login")
        XCTAssertEqual(saved?.1["useSaved"], .bool(true)); XCTAssertEqual(saved?.1["model"], .string(""))
        XCTAssertEqual(saved?.1["profileId"], .null)
        await Task.yield()
        let writes = await client.count("save_codex_login"); XCTAssertEqual(writes, 1)
        let starts = await client.count("start_codex_login"); XCTAssertEqual(starts, 0)
    }

    @MainActor func testNewAttemptCanPollWhileAnOldAttemptHasAnOutstandingRead() async throws {
        let data = try fixture(); let client = LoginTransport(data)
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
        _ = await model.start(); await client.hang("codex_login_status")
        let oldPoll = Task { await model.pollOnce() }
        await waitPending(client, "codex_login_status")
        await client.respond("submit_codex_login_redirect", data["expired"])
        model.redirect = "expired-code"; await model.submitRedirect()
        XCTAssertEqual(model.phase, .failed)
        _ = await model.start()
        await client.allowNewRequests(); await client.respond("codex_login_status", data["success"])
        await model.pollOnce()
        XCTAssertEqual(model.phase, .success)
        await client.finish("codex_login_status", data["pending"]); await oldPoll.value
        XCTAssertEqual(model.phase, .success)
    }

    @MainActor func testInvalidAuthorizationURLIsNotOpenedAndCanBeCanceled() async throws {
        let data = try fixture(); let client = LoginTransport(data)
        var challenge = data["browser"]; challenge["url"] = .string("https://other.example/login")
        await client.respond("start_codex_login", challenge)
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
        let url = await model.start(); XCTAssertNil(url); XCTAssertEqual(model.phase, .failed)
        _ = await model.cancel()
        let count = await client.count("cancel_codex_login"); XCTAssertEqual(count, 1)
    }

    @MainActor func testSavedAccountCanSaveOnceWithoutASeparateSignInAttempt() async throws {
        let data = try fixture(); let client = LoginTransport(data)
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
        await model.loadSavedAccount()
        let result = await model.save(useSaved: true)
        XCTAssertEqual(result, data["saved_models"])
        XCTAssertEqual(model.phase, .saved); XCTAssertFalse(model.canUseSaved)
        _ = await model.save(useSaved: true)
        let writes = await client.count("save_codex_login"); XCTAssertEqual(writes, 1)
        let starts = await client.count("start_codex_login"); XCTAssertEqual(starts, 0)
    }

    @MainActor func testCancellationFailureIsReportedAndDoesNotReplay() async throws {
        let client = LoginTransport(try fixture())
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
        _ = await model.start(); await client.fail("cancel_codex_login")
        let warning = await model.cancel()
        XCTAssertNotNil(warning); XCTAssertEqual(model.phase, .closed); XCTAssertNil(model.challenge)
        _ = await model.cancel()
        let count = await client.count("cancel_codex_login"); XCTAssertEqual(count, 1)
    }

    @MainActor func testUnknownStatusAndWrongSaveResultCannotClaimSuccess() async throws {
        let client = LoginTransport(try fixture())
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
        _ = await model.start()
        await client.respond("codex_login_status", .object(["status": .string("unknown"), "message": .string(""), "account_id": .string("")]))
        await model.pollOnce(); XCTAssertEqual(model.phase, .failed)
        await model.loadSavedAccount()
        await client.respond("save_codex_login", .array([]))
        let result = await model.save(useSaved: true)
        XCTAssertNil(result); XCTAssertEqual(model.phase, .failed)
    }

    @MainActor func testImmediateEscapeClosesOnlyLoginAndCancelsItsAttempt() async throws {
        let client = LoginTransport(try fixture())
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
        _ = await model.start()
        try await exerciseEscape(model, client: client, saving: false)
    }

    @MainActor func testEscapeWhileSavingDoesNotCloseLoginOrParentOrResubmit() async throws {
        let data = try fixture(); let client = LoginTransport(data)
        let model = NativeCodexLoginModel(client: client, automaticallyPoll: false)
        await model.loadSavedAccount(); await client.hang("save_codex_login")
        let saving = Task { await model.save(useSaved: true) }
        await waitPending(client, "save_codex_login")
        try await exerciseEscape(model, client: client, saving: true)
        _ = await model.save(useSaved: true)
        let writes = await client.count("save_codex_login"); XCTAssertEqual(writes, 1)
        await client.finish("save_codex_login", data["saved_models"]); _ = await saving.value
        XCTAssertEqual(model.phase, .saved)
    }

    @MainActor private func exerciseEscape(_ model: NativeCodexLoginModel, client: LoginTransport, saving: Bool) async throws {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 610, height: 620), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        var parentClosed = false; var loginClosed = false
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed = true }
        parent.view = parentView; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeCodexLoginSheet(model: model, saved: {}, close: { _ in loginClosed = true }))
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil))
        for _ in 0..<1000 { if loginClosed || saving { break }; await Task.yield() }
        XCTAssertEqual(loginClosed, !saving); XCTAssertFalse(parentClosed)
        XCTAssertTrue(window.firstResponder === focus)
        if !saving { XCTAssertEqual(model.phase, .closed) }
    }
}
