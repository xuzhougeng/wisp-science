import Foundation
import SwiftUI
import WispProjectBrowser

@MainActor
final class NativeCodexLoginModel: ObservableObject {
    enum Phase: Equatable { case idle, starting, pending, success, failed, saving, saved, closed }
    @Published var method = "browser"
    @Published var modelID: String
    @Published var label: String
    @Published var redirect = ""
    @Published private(set) var phase: Phase = .idle
    @Published private(set) var busy = false
    @Published private(set) var checkingSaved = false
    @Published private(set) var challenge: NativeCodexLoginChallenge?
    @Published private(set) var savedAccount: NativeCodexSubscriptionStatus?
    @Published private(set) var accountID = ""
    @Published private(set) var message = ""
    @Published private(set) var error: String?
    private let client: any NativeSettingsQuerying
    private let profileID: String
    private let apiURL: String
    private let automaticallyPoll: Bool
    private var generation = UUID()
    private var pollingGeneration: UUID?
    private var pollTask: Task<Void, Never>?
    var canClose: Bool { phase != .saving }
    var canSave: Bool { phase == .success && !busy }
    var canUseSaved: Bool { savedAccount?.signed_in == true && !busy && phase != .closed && phase != .saved }

    init(client: any NativeSettingsQuerying, profile: SettingsValue = .null, automaticallyPoll: Bool = true) {
        self.client = client; self.automaticallyPoll = automaticallyPoll
        profileID = profile["id"].string; apiURL = profile["api_url"].string
        modelID = profile["model"].string; label = profile["label"].string
    }
    deinit { pollTask?.cancel() }

    func loadSavedAccount() async {
        guard !checkingSaved, phase != .closed else { return }
        checkingSaved = true
        defer { checkingSaved = false }
        let current = generation
        do {
            let value = try await client.invoke("codex_subscription_status", args: [:], projectID: nil)
            let status = try decode(NativeCodexSubscriptionStatus.self, value)
            guard current == generation else { return }
            savedAccount = status
        } catch {
            if current == generation { self.error = error.localizedDescription }
        }
    }

    /// Returns only a validated browser URL; the view owns opening the browser.
    func start() async -> URL? {
        guard !busy, phase != .pending, phase != .closed else { return nil }
        let current = UUID(); generation = current
        busy = true; phase = .starting; error = nil; redirect = ""; accountID = ""; message = ""
        pollTask?.cancel(); pollTask = nil
        defer { if generation == current { busy = false } }
        do {
            if let previous = challenge {
                _ = try await client.invoke("cancel_codex_login", args: ["loginId": .string(previous.login_id)], projectID: nil)
                guard current == generation else { return nil }
                challenge = nil
            }
            let value = try await client.invoke("start_codex_login", args: ["method": .string(method)], projectID: nil)
            let next = try decode(NativeCodexLoginChallenge.self, value)
            guard !next.login_id.isEmpty else { throw ProjectBrowserError.invalidResponse }
            guard current == generation else {
                _ = try? await client.invoke("cancel_codex_login", args: ["loginId": .string(next.login_id)], projectID: nil)
                return nil
            }
            challenge = next
            guard next.method == method, Self.authorizationURL(next) != nil,
                  next.method != "device" || !next.user_code.isEmpty else { throw ProjectBrowserError.invalidResponse }
            message = next.message; phase = .pending
            if automaticallyPoll { schedulePolling() }
            return next.method == "browser" ? Self.authorizationURL(next) : nil
        } catch {
            if current == generation { self.error = error.localizedDescription; phase = .failed }
            return nil
        }
    }

    static func authorizationURL(_ challenge: NativeCodexLoginChallenge) -> URL? {
        let raw = challenge.method == "device" ? challenge.verification_uri : challenge.url
        guard let url = URL(string: raw), url.scheme == "https", url.host == "auth.openai.com",
              url.user == nil, url.password == nil else { return nil }
        return url
    }

    private func schedulePolling() {
        pollTask?.cancel()
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(nanoseconds: 1_200_000_000) } catch { return }
                guard let self, self.phase == .pending else { return }
                await self.pollOnce()
            }
        }
    }

    func pollOnce() async {
        guard phase == .pending, !busy, pollingGeneration != generation, let challenge else { return }
        let current = generation
        pollingGeneration = current
        defer { if pollingGeneration == current { pollingGeneration = nil } }
        do {
            let value = try await client.invoke("codex_login_status", args: ["loginId": .string(challenge.login_id)], projectID: nil)
            guard current == generation, phase == .pending, !busy else { return }
            try applySnapshot(value)
        } catch {
            if current == generation, phase == .pending, !busy {
                self.error = error.localizedDescription; phase = .failed; pollTask?.cancel()
            }
        }
    }

    func submitRedirect() async {
        guard !busy, phase == .pending, let challenge, challenge.method == "browser", !redirect.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        let current = generation; busy = true; error = nil
        defer { if current == generation { busy = false } }
        do {
            let value = try await client.invoke("submit_codex_login_redirect", args: ["loginId": .string(challenge.login_id), "redirect": .string(redirect)], projectID: nil)
            guard current == generation else { return }
            try applySnapshot(value)
            if phase == .success { redirect = "" }
        } catch {
            if current == generation { self.error = error.localizedDescription }
        }
    }

    private func applySnapshot(_ value: SettingsValue) throws {
        let snapshot = try decode(NativeCodexLoginSnapshot.self, value)
        switch snapshot.status {
        case "pending": phase = .pending
        case "success": phase = .success; pollTask?.cancel(); redirect = ""
        case "error": phase = .failed; pollTask?.cancel(); error = snapshot.message
        default: throw ProjectBrowserError.invalidResponse
        }
        message = snapshot.message; accountID = snapshot.account_id
    }

    /// A save is never retried automatically; an uncertain reply may have persisted.
    func save(useSaved: Bool = false) async -> SettingsValue? {
        guard useSaved ? canUseSaved : canSave else { return nil }
        let current = generation
        busy = true; phase = .saving; error = nil
        pollTask?.cancel(); pollTask = nil
        defer { if current == generation { busy = false } }
        do {
            let value = try await client.invoke("save_codex_login", args: [
                "loginId": .string(challenge?.login_id ?? ""), "model": .string(modelID), "label": .string(label),
                "profileId": profileID.isEmpty ? .null : .string(profileID), "apiUrl": apiURL.isEmpty ? .null : .string(apiURL),
                "useSaved": .bool(useSaved)
            ], projectID: nil)
            guard current == generation else { return nil }
            guard case .array(let rows) = value, rows.contains(where: { $0["provider"].string == "openai_codex" && (profileID.isEmpty || $0["id"].string == profileID) }) else { throw ProjectBrowserError.invalidResponse }
            if let challenge { _ = try? await client.invoke("cancel_codex_login", args: ["loginId": .string(challenge.login_id)], projectID: nil) }
            challenge = nil; redirect = ""; phase = .saved
            return value
        } catch {
            if current == generation {
                phase = .failed
                self.error = localized("保存结果未能确认。请刷新模型列表核对；不会自动重试。") + "\n" + error.localizedDescription
            }
            return nil
        }
    }

    /// Close immediately invalidates late reads/challenges; cancel only this attempt.
    func cancel() async -> String? {
        guard canClose, phase != .closed else { return nil }
        generation = UUID(); pollTask?.cancel(); pollTask = nil
        let id = challenge?.login_id
        challenge = nil; redirect = ""; phase = .closed; busy = false
        guard let id else { return nil }
        do {
            _ = try await client.invoke("cancel_codex_login", args: ["loginId": .string(id)], projectID: nil)
            return nil
        } catch {
            return localized("登录取消请求未能确认。") + "\n" + error.localizedDescription
        }
    }

    private func decode<T: Decodable>(_ type: T.Type, _ value: SettingsValue) throws -> T {
        try JSONDecoder().decode(type, from: JSONEncoder().encode(value))
    }
}
