import SwiftUI
import WispProjectBrowser

struct NativeDocxReviewTarget {
    let client: any NativeConversationQuerying
    let projectID: String
    let sessionID: String?
    var readOnly: Bool
}

@MainActor
final class NativeDocxReviewModel: ObservableObject {
    @Published private(set) var content: NativePanelFileContent
    @Published private(set) var snapshot: DocxReviewSnapshot?
    @Published private(set) var busy = false
    @Published private(set) var stale = false
    @Published private(set) var error: String?
    @Published var readOnly: Bool
    private let target: NativeDocxReviewTarget?
    private var generation = UUID()
    private var closed = false
    init(content: NativePanelFileContent, target: NativeDocxReviewTarget?) {
        self.content = content; self.target = target; readOnly = target?.readOnly ?? true
    }
    var canLoad: Bool {
        target != nil && content.path.hasPrefix("/") && !content.path.contains("://")
            && (content.path as NSString).pathExtension.lowercased() == "docx"
    }
    var canResolve: Bool { !closed && canLoad && !readOnly && !busy && !stale && snapshot?.writable == true }
    func canResolve(_ selection: DocxReviewSelection) -> Bool {
        canResolve && snapshot?.changes.contains(where: { change in
            switch selection { case .change(let id): return change.id == id; case .kind(let kind): return change.kind == kind }
        }) == true
    }
    private func call(_ action: String, _ values: [String: SettingsValue] = [:]) async throws -> NativeDocxReview {
        guard let target, canLoad else { throw ProjectBrowserError.invalidResponse }
        var args = values
        args["session_id"] = target.sessionID.map(SettingsValue.string) ?? .null
        args["path"] = .string(content.path)
        let prefix = target.sessionID == nil ? "native_project_file_" : "native_conversation_panel_file_"
        let value = try await target.client.invoke(prefix + action, args: args, projectID: target.projectID)
        return try NativeDocxReview.decode(value, project: target.projectID, session: target.sessionID, path: content.path)
    }
    func refresh() async {
        guard !closed, !busy, canLoad else { return }
        let current = UUID(); generation = current; busy = true; error = nil
        defer { if current == generation { busy = false } }
        do {
            let result = try await call("docx_review")
            guard current == generation, !closed else { return }
            guard !Task.isCancelled else { stale = true; return }
            content = result.content; snapshot = result.review; stale = false
        } catch {
            guard current == generation, !closed else { return }
            stale = true; self.error = error.localizedDescription
        }
    }
    func resolve(_ selection: DocxReviewSelection, action: DocxReviewAction) async {
        guard canResolve(selection), let snapshot else { return }
        let current = generation; busy = true; error = nil
        defer { if current == generation { busy = false } }
        do {
            let result = try await call("docx_resolve", ["source_sha256": .string(snapshot.source_sha256), "selection": selection.value, "action": .string(action.rawValue)])
            guard current == generation, !closed else { return }
            guard !Task.isCancelled else { stale = true; return }
            content = result.content; self.snapshot = result.review
        } catch {
            guard current == generation, !closed else { return }
            // A lost reply may follow a completed save. Require an explicit
            // refresh; never retry a mutation or re-enable its old snapshot.
            stale = true; self.error = error.localizedDescription
        }
    }
    func close() { closed = true; generation = UUID(); busy = false }
}

struct NativeDocxReviewPreview: View {
    @StateObject private var model: NativeDocxReviewModel
    let target: NativeDocxReviewTarget?
    let quote: ((NativeDocumentSelection) -> Bool)?
    @Binding var saving: Bool
    init(content: NativePanelFileContent, target: NativeDocxReviewTarget?, quote: ((NativeDocumentSelection) -> Bool)?, saving: Binding<Bool>) {
        self.target = target; self.quote = quote; _saving = saving
        _model = StateObject(wrappedValue: NativeDocxReviewModel(content: content, target: target))
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ViewThatFits(in: .horizontal) {
                HStack { title; Spacer(); actions }
                VStack(alignment: .leading) { title; actions }
            }
            Text(localized("审阅正文文字修订。每次操作都会保存当前文档，并在项目中保留备份。")).font(.caption).foregroundStyle(.secondary)
            if let error = model.error {
                Text(error + "\n" + localized("请刷新修订后继续。")).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
            }
            if !model.canLoad || model.readOnly || model.snapshot?.writable == false {
                Text(localized("当前预览为只读。请在可写项目中打开本地当前文件处理修订。")).font(.caption).foregroundStyle(.secondary)
            }
            if let snapshot = model.snapshot {
                if snapshot.unsupported_count > 0 { Text(localized("复杂修订保持不变，请在 Word 中处理") + " (\(snapshot.unsupported_count))").font(.caption).foregroundStyle(.secondary) }
                if snapshot.changes.isEmpty { Text(localized("没有待处理的正文文字修订。")).font(.caption).foregroundStyle(.secondary) }
                else {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 10) {
                            ForEach(snapshot.changes) { change in
                                VStack(alignment: .leading, spacing: 4) {
                                    HStack {
                                        Text(label(change.kind)).fontWeight(.semibold)
                                        Text(change.author).foregroundStyle(.secondary).lineLimit(1)
                                        Spacer()
                                        Button(localized("接受")) { perform(.change(change.id), .accept) }.accessibilityIdentifier("docx-accept-" + change.id)
                                        Button(localized("拒绝")) { perform(.change(change.id), .reject) }.accessibilityIdentifier("docx-reject-" + change.id)
                                    }.disabled(!model.canResolve(.change(change.id)))
                                    if !change.old.isEmpty { Text(change.old).strikethrough().foregroundStyle(.red).textSelection(.enabled) }
                                    if !change.new.isEmpty { Text(change.new).foregroundStyle(.green).fontWeight(.semibold).textSelection(.enabled) }
                                    Divider()
                                }.accessibilityElement(children: .contain)
                            }
                        }.font(.system(size: 12))
                    }.frame(maxHeight: 180)
                }
            }
            NativeRichPreview(content: model.content, kind: .docx, quote: model.busy || model.stale ? nil : quote)
                .frame(minHeight: 120)
        }.task { await model.refresh() }
            .onChange(of: model.busy) { saving = $0 }
            .onChange(of: target?.readOnly) { model.readOnly = $0 ?? true }
            .onDisappear { model.close(); saving = false }
    }
    private var title: some View { Text(localized("修订审阅")).font(.headline) }
    private var actions: some View {
        HStack(spacing: 6) {
            Button(localized("刷新修订")) { Task { await model.refresh() } }.disabled(model.busy || !model.canLoad)
            Button(localized("接受所有删除")) { perform(.kind(.deletion), .accept) }.disabled(!model.canResolve(.kind(.deletion))).accessibilityIdentifier("docx-accept-deletions")
            Button(localized("接受所有改写")) { perform(.kind(.rewrite), .accept) }.disabled(!model.canResolve(.kind(.rewrite))).accessibilityIdentifier("docx-accept-rewrites")
        }.controlSize(.small)
    }
    private func perform(_ selection: DocxReviewSelection, _ action: DocxReviewAction) { Task { await model.resolve(selection, action: action) } }
    private func label(_ kind: DocxChangeKind) -> String {
        switch kind { case .deletion: return localized("删除"); case .insertion: return localized("插入"); case .rewrite: return localized("改写") }
    }
}
