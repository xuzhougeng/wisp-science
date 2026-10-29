import AppKit
import SwiftUI
import WispProjectBrowser

/// `wisp_dto::ProjectMemo`: a researcher-private note the agent never reads.
struct NativeProjectMemo: Codable, Equatable, Identifiable {
    let id: String
    let project_id: String
    let title: String
    let body: String
    let pinned: Bool
    let created_at: Int64
    let updated_at: Int64
    /// `memo_heading` in ui/src/memos.rs: the title, else the first non-empty body line.
    var heading: String? {
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        if !title.isEmpty { return title }
        return body.split(separator: "\n", omittingEmptySubsequences: false).map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }.first { !$0.isEmpty }
    }
}

/// Pure helpers mirrored from ui/src/memos.rs.
enum NativeProjectMemos {
    /// The memo alone in a blank composer, otherwise after one empty line.
    static func composeInsert(_ current: String, _ body: String) -> String {
        current.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? body : current.replacingOccurrences(of: "\\s+$", with: "", options: .regularExpression) + "\n\n" + body
    }
    static func filter(_ memos: [NativeProjectMemo], query: String) -> [NativeProjectMemo] {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        return query.isEmpty ? memos : memos.filter { $0.title.lowercased().contains(query) || $0.body.lowercased().contains(query) }
    }
    /// Pinned first, then most recently updated.
    static func sorted(_ memos: [NativeProjectMemo]) -> [NativeProjectMemo] {
        memos.sorted { ($0.pinned ? 1 : 0, $0.updated_at, $0.id) > ($1.pinned ? 1 : 0, $1.updated_at, $1.id) }
    }
}

/// Project memos through the native settings allowlist; the host scopes every
/// command to `projectID`, and replies for another project are rejected.
@MainActor final class NativeProjectMemosModel: ObservableObject {
    @Published private(set) var memos: [NativeProjectMemo] = []
    @Published private(set) var loading = false
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private let client: any NativeConversationQuerying
    let projectID: String
    init(client: any NativeConversationQuerying, projectID: String) { self.client = client; self.projectID = projectID }

    private func call<T: Decodable>(_ command: String, _ args: [String: SettingsValue] = [:], as: T.Type) async throws -> T {
        let value = try await client.invoke(command, args: args, projectID: projectID)
        return try JSONDecoder().decode(T.self, from: JSONEncoder().encode(value))
    }
    private func accept(_ memo: NativeProjectMemo) throws -> NativeProjectMemo {
        guard memo.project_id == projectID else { throw ProjectBrowserError.invalidResponse }
        return memo
    }
    func load() async {
        loading = memos.isEmpty; error = nil
        defer { loading = false }
        do {
            let rows = try await call("list_project_memos", as: [NativeProjectMemo].self)
            guard rows.allSatisfy({ $0.project_id == projectID }) else { throw ProjectBrowserError.invalidResponse }
            memos = NativeProjectMemos.sorted(rows)
        } catch { self.error = localized("无法读取备忘录。") + " " + error.localizedDescription }
    }
    /// `id == nil` creates. Returns whether the host confirmed the write.
    func save(id: String?, title: String, body: String) async -> Bool {
        guard !busy, !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        busy = true; error = nil
        defer { busy = false }
        do {
            let saved = try accept(id == nil
                ? await call("add_project_memo", ["input": .object(["title": .string(title), "body": .string(body)])], as: NativeProjectMemo.self)
                : await call("update_project_memo", ["patch": .object(["id": .string(id!), "title": .string(title), "body": .string(body)])], as: NativeProjectMemo.self))
            upsert(saved); return true
        } catch { self.error = error.localizedDescription; return false }
    }
    func togglePin(_ memo: NativeProjectMemo) async {
        guard !busy else { return }
        busy = true; error = nil
        defer { busy = false }
        do { upsert(try accept(await call("update_project_memo", ["patch": .object(["id": .string(memo.id), "pinned": .bool(!memo.pinned)])], as: NativeProjectMemo.self))) }
        catch { self.error = error.localizedDescription }
    }
    func delete(_ memo: NativeProjectMemo) async {
        guard !busy else { return }
        busy = true; error = nil
        defer { busy = false }
        do { _ = try await client.invoke("delete_project_memo", args: ["memoId": .string(memo.id)], projectID: projectID); memos.removeAll { $0.id == memo.id } }
        catch { self.error = error.localizedDescription }
    }
    private func upsert(_ memo: NativeProjectMemo) { memos = NativeProjectMemos.sorted(memos.filter { $0.id != memo.id } + [memo]) }
}

/// Right panel 备忘录. Escape closes the editor (asking first when it has
/// unsaved changes) before the panel's own Escape handler sees it.
struct NativeProjectMemosView: View {
    @ObservedObject var model: NativeProjectMemosModel
    let insert: (String) -> Void
    @State private var query = ""
    @State private var editing: String?? = nil
    @State private var title = ""
    @State private var text = ""
    @State private var original = ("", "")
    @State private var confirmDiscard = false
    @State private var expanded: String?
    @State private var inserted: String?
    @State private var deleting: NativeProjectMemo?
    @Environment(\.colorScheme) private var scheme

    private var dirty: Bool { (title, text) != original }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 6) {
                WispIcon(name: "lock", size: 13).foregroundStyle(.secondary)
                Text(localized("仅本机 · AI 不可见")).font(.caption).foregroundStyle(.secondary)
                Spacer()
                Button { open(nil) } label: { Label { Text(localized("新建备忘录")) } icon: { WispIcon(name: "plus", size: 14) } }
                    .disabled(editing != nil)
            }
            TextField(localized("搜索备忘录"), text: $query).textFieldStyle(.roundedBorder).accessibilityLabel(localized("搜索备忘录"))
            if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            if editing != nil {
                NativeProjectMemoEditor(title: $title, text: $text, busy: model.busy, escape: !confirmDiscard, save: {
                    let id = editing ?? nil
                    Task { if await model.save(id: id, title: title, body: text) { editing = nil } }
                }, close: closeEditor)
            }
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 8) {
                    let rows = NativeProjectMemos.filter(model.memos, query: query)
                    if model.loading { ProgressView().controlSize(.small) }
                    else if model.memos.isEmpty { Text(localized("备忘录只给你自己看，AI 不会读到。需要时点「插入」把它放进输入框。")).font(.caption).foregroundStyle(.secondary).padding(.vertical, 20) }
                    else if rows.isEmpty { Text(localized("没有匹配的备忘录。")).font(.caption).foregroundStyle(.secondary).padding(.vertical, 20) }
                    ForEach(rows) { card($0) }
                }
            }
        }
        .task { await model.load() }
        .confirmationDialog(localized("放弃修改？"), isPresented: $confirmDiscard) {
            Button(localized("放弃"), role: .destructive) { editing = nil }
            Button(localized("继续编辑"), role: .cancel) {}
        }
        .confirmationDialog(localized("删除"), isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }), presenting: deleting) { memo in
            Button(localized("删除"), role: .destructive) { deleting = nil; Task { await model.delete(memo) } }
            Button(localized("取消"), role: .cancel) { deleting = nil }
        }
    }

    private func card(_ memo: NativeProjectMemo) -> some View {
        let open = expanded == memo.id
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 6) {
                if memo.pinned { WispIcon(name: "pin", size: 12).help(localized("已置顶")).accessibilityLabel(localized("已置顶")) }
                Button { expanded = open ? nil : memo.id } label: {
                    Text(memo.heading ?? localized("未命名")).fontWeight(.semibold).foregroundStyle(memo.heading == nil ? Color.secondary : Color.primary).lineLimit(1)
                }.buttonStyle(.plain).help(localized(open ? "收起" : "展开全文"))
                Spacer()
                Button(localized(inserted == memo.id ? "已插入" : "插入输入框")) {
                    insert(memo.body); inserted = memo.id
                    Task { try? await Task.sleep(nanoseconds: 1_500_000_000); if inserted == memo.id { inserted = nil } }
                }.font(.caption)
                Button { self.open(memo) } label: { WispIcon(name: "edit", size: 14) }.buttonStyle(.plain)
                    .help(localized("编辑")).accessibilityLabel(localized("编辑")).disabled(editing != nil)
                Menu {
                    Button { Task { await model.togglePin(memo) } } label: { Label { Text(localized(memo.pinned ? "取消置顶" : "置顶")) } icon: { WispIcon(name: "pin") } }
                    Button { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(memo.body, forType: .string) } label: { Label { Text(localized("复制")) } icon: { WispIcon(name: "copy") } }
                    Button(role: .destructive) { deleting = memo } label: { Label { Text(localized("删除")) } icon: { WispIcon(name: "trash") } }
                } label: { WispIcon(name: "more", size: 14) }
                    .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().accessibilityLabel(localized("更多操作")).disabled(model.busy)
            }
            Text(memo.body).font(.caption).foregroundStyle(.secondary).lineLimit(open ? nil : 2).textSelection(.enabled)
        }
        .padding(10).frame(maxWidth: .infinity, alignment: .leading)
        .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
        .accessibilityIdentifier("memo-\(memo.id)")
    }

    private func open(_ memo: NativeProjectMemo?) {
        editing = .some(memo?.id); title = memo?.title ?? ""; text = memo?.body ?? ""; original = (title, text)
    }
    private func closeEditor() { if dirty { confirmDiscard = true } else { editing = nil } }
}

/// The inline memo form. Its scoped Escape listener runs before the panel's,
/// so one press closes the editor (or asks to discard) and leaves the panel open.
struct NativeProjectMemoEditor: View {
    @Binding var title: String
    @Binding var text: String
    let busy: Bool
    var escape = true
    let save: () -> Void
    let close: () -> Void
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            TextField(localized("标题（可选）"), text: $title).textFieldStyle(.roundedBorder).accessibilityLabel(localized("标题（可选）"))
            TextEditor(text: $text).font(.system(size: 13)).frame(minHeight: 110, maxHeight: 320)
                .overlay(alignment: .topLeading) { if text.isEmpty { Text(localized("写点只有你需要看到的内容…")).foregroundStyle(.tertiary).padding(6).allowsHitTesting(false) } }
                .accessibilityLabel(localized("写点只有你需要看到的内容…"))
            HStack {
                Spacer()
                Button(localized("取消"), action: close)
                Button(localized("保存"), action: save).disabled(busy || text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
        .padding(10).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
        .background(NativeSettingsEscape(enabled: escape, close: close))
    }
}
