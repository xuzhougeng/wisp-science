import Foundation
import SwiftUI
import WispProjectBrowser

@MainActor
final class NativeSearchModel: ObservableObject {
    @Published private(set) var items: [NativeSearchItem] = []
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private(set) var resultQuery: String?
    private var generation = UUID()
    private let client: any NativeSettingsQuerying
    private let projectID: String?

    init(client: any NativeSettingsQuerying, projectID: String?) {
        self.client = client; self.projectID = projectID
    }

    func invalidate() { generation = UUID(); items = []; busy = false; error = nil; resultQuery = nil }

    func search(_ query: String, debounce: UInt64 = 120_000_000) async {
        let current = UUID(); generation = current
        items = []; error = nil; busy = true; resultQuery = nil
        defer { if generation == current { busy = false } }
        do {
            guard query.utf8.count <= 512 else { throw ProjectBrowserError.unavailable(localized("搜索关键词过长，请缩短后重试。")) }
            if debounce > 0 { try await Task.sleep(nanoseconds: debounce) }
            guard generation == current, !Task.isCancelled else { return }
            let value = try await client.invoke("native_workspace_search", args: ["query": .string(query)], projectID: projectID)
            let result = try NativeSearchResponse.decode(value, query: query, projectID: projectID)
            guard generation == current, !Task.isCancelled else { return }
            resultQuery = query; items = result.items
        } catch is CancellationError {
        } catch {
            if generation == current, !Task.isCancelled { self.error = localized("搜索未能读取，请重试。") + "\n" + error.localizedDescription }
        }
    }
}

/// The selected artifact opens by its stable ID in its owning conversation.
struct NativeSearchArtifactView: View {
    let item: NativeSearchItem
    let close: () -> Void
    @StateObject private var panel: NativePanelModel
    init(item: NativeSearchItem, client: any NativeConversationQuerying, close: @escaping () -> Void) {
        self.item = item; self.close = close
        _panel = StateObject(wrappedValue: NativePanelModel(client: client, projectID: item.project_id, sessionID: item.session_id ?? ""))
    }
    var body: some View {
        Group {
            if let content = panel.preview {
                NativePanelFilePreview(content: content, close: close)
            } else {
                VStack(alignment: .leading, spacing: 16) {
                    HStack { Text(item.title).font(.headline); Spacer(); Button(localized("关闭"), action: close) }
                    if let error = panel.error {
                        Text(error).foregroundStyle(.orange).textSelection(.enabled)
                        Button(localized("重新读取")) { Task { await panel.readArtifact(item.objectID) } }
                    } else { ProgressView(localized("正在读取产物…")) }
                    Spacer()
                }.padding(20).frame(width: 600, height: 420)
                    .background(NativeSettingsEscape(close: close))
            }
        }.task { await panel.readArtifact(item.objectID) }.onDisappear { panel.close() }
    }
}
