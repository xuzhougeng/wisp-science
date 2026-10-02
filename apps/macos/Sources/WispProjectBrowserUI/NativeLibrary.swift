import SwiftUI
import WispProjectBrowser

struct LibraryEntry: Codable, Identifiable, Equatable {
    var id: String
    var kind: String
    var title: String
    var language: String?
    var codePreview: String
    var sourceProjectID: String
    var sourceProjectName: String
    var sourceSessionID: String
    var sourceSessionTitle: String
    var sourcePath: String?
    var createdAt: Int64

    enum CodingKeys: String, CodingKey {
        case id, kind, title, language
        case codePreview = "code_preview"
        case sourceProjectID = "source_project_id"
        case sourceProjectName = "source_project_name"
        case sourceSessionID = "source_session_id"
        case sourceSessionTitle = "source_session_title"
        case sourcePath = "source_path"
        case createdAt = "created_at"
    }
}

struct LibraryFilter: Identifiable, Equatable {
    var id: String { kind }
    let kind: String
    let title: String
}

enum NativeLibraryCommand {
    static let search = "native_library_search"
    static let delete = "native_library_delete"
}

@MainActor
final class NativeLibraryModel: ObservableObject {
    @Published var presented = false
    @Published var query = ""
    @Published var kind = ""
    @Published private(set) var items: [LibraryEntry] = []
    @Published private(set) var busy = false
    @Published private(set) var deleting: Set<String> = []
    @Published private(set) var error: String?
    private var searchGeneration = UUID()
    private var removed: Set<String> = []
    static let filters = [
        LibraryFilter(kind: "", title: "全部"),
        LibraryFilter(kind: "code", title: "代码"),
        LibraryFilter(kind: "figure", title: "图片"),
        LibraryFilter(kind: "text", title: "文本"),
    ]

    func dismiss() {
        guard deleting.isEmpty else { return }
        presented = false
    }

    static func offersInsert(activeSessionID: String?) -> Bool {
        guard let activeSessionID else { return false }
        return !activeSessionID.isEmpty
    }

    static func composerText(_ item: LibraryEntry) -> String {
        let body = item.codePreview.trimmingCharacters(in: .whitespacesAndNewlines)
        if body.isEmpty { return "请看收藏「\(item.title)」。" }
        let language = item.language ?? ""
        return "请用收藏「\(item.title)」重新运行：\n```\(language)\n\(body)\n```"
    }

    static func insertIntoConversation(_ item: LibraryEntry, conversation: NativeConversationModel, activeSessionID: String?) -> Bool {
        guard offersInsert(activeSessionID: activeSessionID) else { return false }
        return conversation.prefillLibrary(composerText(item))
    }

    static func decode(_ value: SettingsValue) throws -> [LibraryEntry] {
        try JSONDecoder().decode([LibraryEntry].self, from: JSONEncoder().encode(value))
    }

    func reload(_ client: any NativeSettingsQuerying) async {
        guard presented, !busy else { return }
        let generation = UUID()
        searchGeneration = generation
        busy = true
        error = nil
        let query = query
        let kind = kind
        defer { if searchGeneration == generation { busy = false } }
        do {
            var args: [String: SettingsValue] = ["query": .string(query)]
            if !kind.isEmpty { args["kind"] = .string(kind) }
            let value = try await client.invoke(NativeLibraryCommand.search, args: args, projectID: nil)
            guard searchGeneration == generation, presented else { return }
            items = try Self.decode(value).filter { !removed.contains($0.id) }
            error = nil
        } catch {
            guard searchGeneration == generation, presented else { return }
            self.error = "收藏库未能确认读取，不会自动重试。\n" + error.localizedDescription
        }
    }

    func delete(_ client: any NativeSettingsQuerying, id: String) async {
        let id = id.trimmingCharacters(in: .whitespacesAndNewlines)
        guard presented, !id.isEmpty, !deleting.contains(id) else { return }
        deleting = deleting.union([id])
        defer {
            var next = deleting
            next.remove(id)
            deleting = next
        }
        do {
            let value = try await client.invoke(NativeLibraryCommand.delete, args: ["id": .string(id)], projectID: nil)
            guard value.bool else {
                error = "这条收藏已不存在。不会自动重试。"
                return
            }
            removed.insert(id)
            items = items.filter { $0.id != id }
            error = nil
        } catch {
            self.error = "删除未能确认，不会自动重试。\n" + error.localizedDescription
        }
    }
}

struct NativeLibrarySheet: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var library: NativeLibraryModel

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("收藏库").font(.headline)
            HStack {
                TextField("搜索标题、代码或项目", text: $library.query)
                    .textFieldStyle(.roundedBorder)
                    .disabled(library.busy)
                    .onSubmit { Task { await library.reload(model.libraryClient()) } }
                Button(library.busy ? "正在搜索…" : "搜索") { Task { await library.reload(model.libraryClient()) } }
                    .disabled(library.busy)
            }
            HStack {
                ForEach(NativeLibraryModel.filters) { filter in
                    Button(filter.title) {
                        library.kind = filter.kind
                        Task { await library.reload(model.libraryClient()) }
                    }
                    .disabled(library.busy)
                    .accessibilityAddTraits(library.kind == filter.kind ? .isSelected : [])
                }
            }
            if let error = library.error {
                Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            ScrollView {
                if library.items.isEmpty && !library.busy && library.error == nil {
                    Text("收藏库是空的").foregroundStyle(.secondary).frame(maxWidth: .infinity, alignment: .leading)
                }
                LazyVStack(alignment: .leading, spacing: 8) {
                    ForEach(library.items) { item in
                        VStack(alignment: .leading, spacing: 6) {
                            Text(item.title).font(.headline)
                            Text("\(item.sourceProjectName) / \(item.sourceSessionTitle)").font(.caption).foregroundStyle(.secondary)
                            if !item.codePreview.isEmpty {
                                Text(item.codePreview).font(.system(.caption, design: .monospaced)).lineLimit(4)
                            }
                            HStack {
                                if NativeLibraryModel.offersInsert(activeSessionID: model.activeSessionID) {
                                    Button("填入对话框") {
                                        if NativeLibraryModel.insertIntoConversation(item, conversation: model.nativeConversation(), activeSessionID: model.activeSessionID) {
                                            library.presented = false
                                        }
                                    }
                                }
                                Button("打开来源") { Task { await model.openLibrarySource(item) } }
                                    .disabled(!library.deleting.isEmpty)
                                Button(library.deleting.contains(item.id) ? "正在删除…" : "删除") {
                                    Task { await library.delete(model.libraryClient(), id: item.id) }
                                }
                                .disabled(library.deleting.contains(item.id))
                            }
                        }
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
        }
        .padding(20)
        .frame(width: 560, height: 460)
        .interactiveDismissDisabled(!library.deleting.isEmpty)
        .background(NativeSettingsEscape(enabled: library.deleting.isEmpty) { library.dismiss() })
        .task { await library.reload(model.libraryClient()) }
    }
}
