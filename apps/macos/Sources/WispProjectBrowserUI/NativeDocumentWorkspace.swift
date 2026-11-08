import SwiftUI
import WispProjectBrowser

@MainActor final class NativeDocumentTabs: ObservableObject {
    @MainActor struct Document: Identifiable {
        let id: String
        let scope: String
        let model: NativeFilesModel
        var closeRequest = 0
        var title: String { model.preview.map { ($0.content.path as NSString).lastPathComponent } ?? localized("文件") }
    }
    @Published private(set) var documents: [Document] = []
    @Published var selected: [String: String] = [:]
    @Published var sideChat: Set<String> = []
    func open(_ model: NativeFilesModel, scope: String) {
        guard let preview = model.preview else { return }
        let id = scope + ":" + [model.contextID, preview.content.path].map { "\($0.utf8.count):\($0)" }.joined()
        if !documents.contains(where: { $0.id == id }) { documents.append(.init(id: id, scope: scope, model: model)) }
        selected[scope] = id
    }
    func requestClose(_ id: String) {
        guard let index = documents.firstIndex(where: { $0.id == id }) else { return }
        selected[documents[index].scope] = id
        documents[index].closeRequest += 1
    }
    func close(_ id: String) {
        guard let document = documents.first(where: { $0.id == id }) else { return }
        document.model.close(); documents.removeAll { $0.id == id }
        if selected[document.scope] == id { selected[document.scope] = nil }
    }
}

struct NativeDocumentTabStrip: View {
    @ObservedObject var tabs: NativeDocumentTabs
    let scope: String
    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                Button(localized("对话")) { tabs.selected[scope] = nil }.buttonStyle(.plain)
                ForEach(tabs.documents.filter { $0.scope == scope }) { document in
                    HStack(spacing: 6) {
                        Button(document.title) { tabs.selected[scope] = document.id }.buttonStyle(.plain)
                        Button { tabs.requestClose(document.id) } label: { WispIcon(name: "close", size: 12) }.buttonStyle(.plain).accessibilityLabel(localized("关闭文件") + " " + document.title)
                    }.padding(8).background(tabs.selected[scope] == document.id ? Color.accentColor.opacity(0.1) : Color.clear, in: RoundedRectangle(cornerRadius: 6))
                }
                if tabs.selected[scope] != nil {
                    Button {
                        if tabs.sideChat.contains(scope) { tabs.sideChat.remove(scope) } else { tabs.sideChat.insert(scope) }
                    } label: { WispIcon(name: "bubble", size: 16) }.buttonStyle(.plain).help(localized("在文档旁边对话")).accessibilityLabel(localized("在文档旁边对话"))
                }
            }.padding(.horizontal, 12).frame(height: 40)
        }
    }
}

struct NativeWorkspaceDocument: View {
    @ObservedObject var model: NativeFilesModel
    let active: Bool
    let closeRequest: Int
    let close: () -> Void
    let quote: (NativeSideChatQuote) -> Void
    var body: some View {
        if let preview = model.preview {
            let identity = model.previewIdentity
            NativePanelFilePreview(content: preview.content, close: close,
                save: model.canWrite && !preview.content.truncated && preview.content.text != nil ? { try await model.save($0, original: preview.content) } : nil,
                quote: { text in if let value = model.quote(text, source: preview.content.path) { quote(value) } },
                documentQuote: { selection in
                    guard let value = model.documentQuote(selection, original: preview.content, identity: identity) else { return false }; quote(value); return true
                }, loadImage: { try await model.readPreviewImage($0, original: preview.content) },
                docxTarget: preview.context_id == "local" ? NativeDocxReviewTarget(client: model.client, projectID: model.projectID, sessionID: model.sessionID, readOnly: model.readOnly || model.catalog?.read_only != false) : nil,
                active: active, closeRequest: closeRequest)
        }
    }
}
