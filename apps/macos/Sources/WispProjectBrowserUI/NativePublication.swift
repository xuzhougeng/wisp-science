import SwiftUI
import WispProjectBrowser

struct PublicationRecord: Codable, Identifiable, Equatable {
    var id: String
    var projectID: String
    var title: String
    var description: String
    enum CodingKeys: String, CodingKey {
        case id, title, description
        case projectID = "project_id"
    }
}

struct PublicationRevisionRecord: Codable, Identifiable, Equatable {
    var id: String
    var label: String
    var state: String
    var revisionNumber: Int64
    var publicationID: String
    enum CodingKeys: String, CodingKey {
        case id, label, state
        case revisionNumber = "revision_number"
        case publicationID = "publication_id"
    }
}

struct PublicationItemRecord: Codable, Identifiable, Equatable {
    var id: String
    var title: String
    var kind: String
    var ordinal: Int64
    var revisionID: String?
    var parentItemID: String?
    var content: String?
    enum CodingKeys: String, CodingKey {
        case id, title, kind, ordinal, content
        case revisionID = "revision_id"
        case parentItemID = "parent_item_id"
    }
}

struct PublicationOutlineRow: Identifiable {
    var item: PublicationItemRecord
    var depth: Int
    var id: String { item.id }

    static func build(_ items: [PublicationItemRecord]) -> [Self] {
        let sorted = items.sorted { $0.ordinal == $1.ordinal ? $0.id < $1.id : $0.ordinal < $1.ordinal }
        let ids = Set(items.map(\.id))
        let children = Dictionary(grouping: sorted, by: { $0.parentItemID ?? "" })
        var result: [Self] = []
        var seen = Set<String>()
        func append(_ item: PublicationItemRecord, depth: Int) {
            guard seen.insert(item.id).inserted else { return }
            result.append(Self(item: item, depth: depth))
            for child in children[item.id] ?? [] { append(child, depth: depth + 1) }
        }
        for item in sorted where item.parentItemID == nil || !ids.contains(item.parentItemID!) { append(item, depth: 0) }
        // Older or malformed trees still expose every item exactly once.
        for item in sorted where !seen.contains(item.id) { append(item, depth: 0) }
        return result
    }
}

struct PublicationBindingRecord: Codable, Identifiable, Equatable {
    var id: String
    var revisionID: String
    var itemID: String?
    var sourceKind: String
    var sourceID: String
    var purpose: String
    var supportedClaimItemID: String?
    var selectionState: String
    var reviewState: String
    var reproductionState: String
    var visibility: String
    var sourceSnapshotJSON: String
    enum CodingKeys: String, CodingKey {
        case id, purpose, visibility
        case revisionID = "revision_id", itemID = "item_id", sourceKind = "source_kind", sourceID = "source_id"
        case supportedClaimItemID = "supported_claim_item_id", selectionState = "selection_state"
        case reviewState = "review_state", reproductionState = "reproduction_state", sourceSnapshotJSON = "source_snapshot_json"
    }
}

struct PublicationWorkspaceRecord: Codable, Equatable {
    var publications: [PublicationRecord]
    var publication: PublicationRecord?
    var revision: PublicationRevisionRecord?
    var items: [PublicationItemRecord]
    var revisions: [PublicationRevisionRecord] = []
    var bindings: [PublicationBindingRecord] = []

    enum CodingKeys: String, CodingKey { case publications, publication, revision, items, revisions, bindings }
    init(publications: [PublicationRecord], publication: PublicationRecord?, revision: PublicationRevisionRecord?, items: [PublicationItemRecord]) {
        self.publications = publications
        self.publication = publication
        self.revision = revision
        self.items = items
    }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        publications = try values.decode([PublicationRecord].self, forKey: .publications)
        publication = try values.decodeIfPresent(PublicationRecord.self, forKey: .publication)
        revision = try values.decodeIfPresent(PublicationRevisionRecord.self, forKey: .revision)
        items = try values.decode([PublicationItemRecord].self, forKey: .items)
        revisions = try values.decodeIfPresent([PublicationRevisionRecord].self, forKey: .revisions) ?? revision.map { [$0] } ?? []
        bindings = try values.decodeIfPresent([PublicationBindingRecord].self, forKey: .bindings) ?? []
    }
    static var empty: Self { Self(publications: [], publication: nil, revision: nil, items: []) }
}

struct PublicationDraft: Equatable {
    var title = ""
    var description = ""
    var revisionLabel = ""
}

enum NativePublicationCommand {
    static let read = "native_publication_workspace"
    static let create = "native_publication_create"
}

@MainActor
final class NativePublicationModel: ObservableObject {
    @Published var presented = false
    @Published var draft = PublicationDraft()
    @Published private(set) var projectID: String?
    @Published private(set) var workspace = PublicationWorkspaceRecord(publications: [], publication: nil, revision: nil, items: [])
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    @Published var selectedItemID: String?
    @Published var selectedBindingID: String?
    private var requestedPublicationID: String?
    private var requestedRevisionID: String?
    private var generation = UUID()

    var selectedItem: PublicationItemRecord? { workspace.items.first { $0.id == selectedItemID } }
    var visibleBindings: [PublicationBindingRecord] {
        workspace.bindings.filter { selectedItemID == nil || $0.itemID == selectedItemID || $0.supportedClaimItemID == selectedItemID }
    }
    var selectedBinding: PublicationBindingRecord? { visibleBindings.first { $0.id == selectedBindingID } }

    func selectItem(_ id: String?) {
        selectedItemID = id
        selectedBindingID = nil
    }


    func open(projectID: String) {
        generation = UUID()
        self.projectID = projectID
        workspace = .empty
        requestedPublicationID = nil
        requestedRevisionID = nil
        selectedItemID = nil
        selectedBindingID = nil
        error = nil
        busy = false
        presented = true
    }

    func dismiss() {
        invalidate()
    }

    func invalidate() {
        generation = UUID()
        presented = false
        busy = false
    }

    var canCreate: Bool {
        !busy && !draft.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !draft.revisionLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func reload(_ client: any NativeSettingsQuerying) async {
        guard presented, let projectID, !projectID.isEmpty else { return }
        var args: [String: SettingsValue] = [:]
        if let requestedPublicationID { args["publication_id"] = .string(requestedPublicationID) }
        if let requestedRevisionID { args["revision_id"] = .string(requestedRevisionID) }
        await send(client, command: NativePublicationCommand.read, args: args, projectID: projectID, clearDraft: false)
    }

    func selectPublication(_ id: String, client: any NativeSettingsQuerying) async {
        guard presented, workspace.publications.contains(where: { $0.id == id }) else { return }
        requestedPublicationID = id
        requestedRevisionID = nil
        await reload(client)
    }

    func selectRevision(_ id: String, client: any NativeSettingsQuerying) async {
        guard presented, workspace.revisions.contains(where: { $0.id == id && $0.publicationID == requestedPublicationID }) else { return }
        requestedRevisionID = id
        await reload(client)
    }

    func create(_ client: any NativeSettingsQuerying) async {
        guard presented, let projectID, !projectID.isEmpty, !busy else { return }
        let title = draft.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = draft.revisionLabel.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, !label.isEmpty else {
            error = "请填写论文标题和版本标签。"
            return
        }
        await send(
            client,
            command: NativePublicationCommand.create,
            args: [
                "title": .string(draft.title),
                "description": .string(draft.description),
                "revision_label": .string(draft.revisionLabel),
            ],
            projectID: projectID,
            clearDraft: true)
    }

    private func send(_ client: any NativeSettingsQuerying, command: String, args: [String: SettingsValue], projectID: String, clearDraft: Bool) async {
        let current = UUID()
        generation = current
        busy = true
        error = nil
        // Keep selector choices while clearing all details during a read. A failed
        // switch must never display the previous revision as the requested one.
        workspace.publication = nil
        workspace.revision = nil
        workspace.items = []
        workspace.bindings = []
        selectedItemID = nil
        selectedBindingID = nil
        defer { if generation == current { busy = false } }
        do {
            let value = try await client.invoke(command, args: args, projectID: projectID)
            guard generation == current, presented, self.projectID == projectID else { return }
            let page = try JSONDecoder().decode(PublicationWorkspaceRecord.self, from: JSONEncoder().encode(value))
            guard page.publications.allSatisfy({ $0.projectID == projectID }),
                  page.publication.map({ $0.projectID == projectID }) ?? true,
                  (clearDraft || (requestedPublicationID.map({ $0 == page.publication?.id }) ?? true)),
                  (clearDraft || (requestedRevisionID.map({ $0 == page.revision?.id }) ?? true)),
                  page.revisions.allSatisfy({ $0.publicationID == page.publication?.id }),
                  page.revision.map({ $0.publicationID == page.publication?.id }) ?? true,
                  page.items.allSatisfy({ $0.revisionID == nil || $0.revisionID == page.revision?.id }),
                  page.bindings.allSatisfy({ $0.revisionID == page.revision?.id }) else {
                throw ProjectBrowserError.service("论文或修订作用域不匹配。")
            }
            workspace = page
            requestedPublicationID = page.publication?.id
            requestedRevisionID = page.revision?.id
            if clearDraft { draft = PublicationDraft() }
            error = nil
        } catch {
            guard generation == current, presented, self.projectID == projectID else { return }
            let lead = command == NativePublicationCommand.create ? "论文证据未能确认创建，不会自动重试。\n" : "论文证据未能确认读取，不会自动重试。\n"
            self.error = lead + error.localizedDescription
        }
    }
}

enum NativePublicationLabel {
    static func text(_ value: String) -> String {
        let labels = ["draft": "草稿", "freezing": "正在冻结", "frozen": "已冻结", "published": "已发布", "deleting": "正在删除",
                      "section": "章节", "claim": "论点", "figure": "图", "table": "表", "methods": "方法", "supplement": "补充材料",
                      "candidate": "候选", "selected": "已选用", "rejected": "已排除", "unreviewed": "未审阅", "reviewed": "已审阅",
                      "not_run": "未重现", "passed": "重现通过", "failed": "重现失败", "not_applicable": "不适用",
                      "public": "公开", "restricted": "受限", "private": "私有", "artifact_version": "产物版本", "run": "运行", "execution_log": "执行日志",
                      "message_span": "消息片段", "tool_call": "工具调用", "code_cell": "代码单元", "external_resource": "外部资源"]
        return localized(labels[value] ?? value)
    }
}

struct NativePublicationColumn: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var publication: NativePublicationModel

    var body: some View {
        GeometryReader { geometry in
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text(localized("论文证据")).font(.title2.bold())
                    Text(localized("只读工作区")).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button(localized("刷新")) { Task { await publication.reload(model.calendarClient()) } }.disabled(publication.busy)
                    Button(localized("返回对话")) { publication.dismiss() }
                }
                if !publication.workspace.publications.isEmpty { selectors }
                if publication.busy {
                    ProgressView(localized("正在读取论文证据…")).frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if let error = publication.error {
                    Text(error).foregroundStyle(.red).textSelection(.enabled)
                    Spacer()
                } else if publication.workspace.publications.isEmpty {
                    Text(localized("还没有论文证据。可在 WebView 版本中创建论文和整理证据后，在此查看。")).foregroundStyle(.secondary)
                    Spacer()
                } else if publication.workspace.revision == nil {
                    Text(localized("当前论文还没有修订。")).foregroundStyle(.secondary)
                    Spacer()
                } else if geometry.size.width >= 760 {
                    HStack(alignment: .top, spacing: 20) {
                        structure.frame(width: 240)
                        Divider()
                        details.frame(maxWidth: .infinity)
                    }
                } else {
                    ScrollView {
                        VStack(alignment: .leading, spacing: 16) {
                            structureContent
                            Divider()
                            detailContent
                        }
                    }
                }
            }
            .padding(20)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
        .background(NativeSettingsEscape { publication.dismiss() })
        .task(id: publication.projectID ?? "") { await publication.reload(model.calendarClient()) }
    }

    private var selectors: some View {
        VStack(alignment: .leading, spacing: 8) {
            Picker("论文", selection: Binding(get: { publication.workspace.publication?.id ?? "" }, set: { id in
                Task { await publication.selectPublication(id, client: model.calendarClient()) }
            })) {
                Text(localized("选择论文")).tag("")
                ForEach(publication.workspace.publications) { Text($0.title).tag($0.id) }
            }.disabled(publication.busy)
            Picker("修订", selection: Binding(get: { publication.workspace.revision?.id ?? "" }, set: { id in
                Task { await publication.selectRevision(id, client: model.calendarClient()) }
            })) {
                Text(localized("选择修订")).tag("")
                ForEach(publication.workspace.revisions) { revision in
                    Text("\(revision.label) · \(NativePublicationLabel.text(revision.state))").tag(revision.id)
                }
            }.disabled(publication.busy)
        }
    }

    private var structure: some View { ScrollView { structureContent } }
    private var structureContent: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(localized("论文结构")).font(.headline)
            Button("全部证据（\(publication.workspace.bindings.count)）") { publication.selectItem(nil) }
                .buttonStyle(.plain).foregroundStyle(publication.selectedItemID == nil ? Color.accentColor : Color.primary)
            if publication.workspace.items.isEmpty { Text(localized("当前修订尚无结构条目。")).foregroundStyle(.secondary) }
            ForEach(PublicationOutlineRow.build(publication.workspace.items)) { row in
                let item = row.item
                Button { publication.selectItem(item.id) } label: {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(NativePublicationLabel.text(item.kind)).font(.caption).foregroundStyle(.secondary)
                        Text(item.title).multilineTextAlignment(.leading)
                    }
                    .padding(8).padding(.leading, CGFloat(min(row.depth, 5)) * 12).frame(maxWidth: .infinity, alignment: .leading)
                    .background(publication.selectedItemID == item.id ? Color.accentColor.opacity(0.12) : Color.clear)
                    .cornerRadius(6)
                }.buttonStyle(.plain)
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }

    private var details: some View { ScrollView { detailContent } }
    private var detailContent: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let item = publication.selectedItem {
                Text(item.title).font(.title3.bold())
                if let content = item.content, !content.isEmpty { Text(content).textSelection(.enabled) }
            } else if let paper = publication.workspace.publication {
                Text(paper.title).font(.title3.bold())
                if !paper.description.isEmpty { Text(paper.description).textSelection(.enabled) }
            }
            Text("关联证据（\(publication.visibleBindings.count)）").font(.headline)
            if publication.visibleBindings.isEmpty {
                Text(localized("当前选择没有关联证据。")).foregroundStyle(.secondary)
            }
            ForEach(publication.visibleBindings) { binding in
                Button { publication.selectedBindingID = binding.id } label: {
                    VStack(alignment: .leading, spacing: 5) {
                        Text(NativePublicationLabel.text(binding.sourceKind)).font(.headline)
                        Text(binding.purpose.isEmpty ? binding.sourceID : binding.purpose).lineLimit(3)
                        Text("\(NativePublicationLabel.text(binding.selectionState)) · \(NativePublicationLabel.text(binding.reviewState)) · \(NativePublicationLabel.text(binding.reproductionState))")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .padding(12).frame(maxWidth: .infinity, alignment: .leading)
                    .background(publication.selectedBindingID == binding.id ? Color.accentColor.opacity(0.12) : Color.secondary.opacity(0.06))
                    .cornerRadius(8)
                }.buttonStyle(.plain)
            }
            if let binding = publication.selectedBinding {
                Divider()
                Text(localized("证据来源详情")).font(.headline)
                Text("来源 ID：\(binding.sourceID)")
                Text("证据 ID：\(binding.id)")
                Text("修订 ID：\(binding.revisionID)")
                Text("可见性：\(NativePublicationLabel.text(binding.visibility))")
                Text(localized("来源快照")).font(.headline)
                Text(binding.sourceSnapshotJSON).font(.system(.caption, design: .monospaced))
            }
        }.textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
    }
}
