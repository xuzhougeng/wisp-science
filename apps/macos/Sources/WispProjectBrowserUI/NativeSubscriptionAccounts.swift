import Foundation
import SwiftUI
import WispProjectBrowser

/// Account status is a read-only global projection; a failed read is not a sign-out.
@MainActor
final class NativeSubscriptionAccounts: ObservableObject {
    @Published private(set) var accounts: [String: NativeCodexSubscriptionStatus] = [:]
    @Published private(set) var errors: [String: String] = [:]
    @Published private(set) var loading = false
    private var generation = UUID()

    func load(_ client: any NativeSettingsQuerying) async {
        let current = UUID()
        generation = current
        accounts = [:]
        errors = [:]
        loading = true
        defer { if generation == current { loading = false } }
        for provider in ["codex", "xai"] {
            do {
                let value = try await client.invoke("codex_subscription_status", args: ["provider": .string(provider)], projectID: nil)
                guard generation == current, !Task.isCancelled else { return }
                accounts[provider] = try JSONDecoder().decode(NativeCodexSubscriptionStatus.self, from: JSONEncoder().encode(value))
            } catch {
                guard generation == current, !Task.isCancelled else { return }
                errors[provider] = error.localizedDescription
            }
        }
    }

    func invalidate() { generation = UUID(); loading = false }
}
