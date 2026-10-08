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
    var capabilityLevel: String?
    var manifestSHA256: String?
    enum CodingKeys: String, CodingKey {
        case id, label, state
        case revisionNumber = "revision_number"
        case publicationID = "publication_id"
        case capabilityLevel = "capability_level", manifestSHA256 = "manifest_sha256"
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
    var readiness: PublicationReadinessRecord?
    var lineage: [SettingsValue] = []
    var drift: [SettingsValue] = []
    var waivers: [SettingsValue] = []
    var reviews: [SettingsValue] = []
    var reproductionRuns: [SettingsValue] = []
    var reproductionResults: [SettingsValue] = []
    var capsuleBuilds: [SettingsValue] = []

    enum CodingKeys: String, CodingKey {
        case publications, publication, revision, items, revisions, bindings, readiness, lineage, drift, waivers, reviews
        case reproductionRuns = "reproduction_runs", reproductionResults = "reproduction_results", capsuleBuilds = "capsule_builds"
    }
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
        readiness = try values.decodeIfPresent(PublicationReadinessRecord.self, forKey: .readiness)
        lineage = try values.decodeIfPresent([SettingsValue].self, forKey: .lineage) ?? []
        drift = try values.decodeIfPresent([SettingsValue].self, forKey: .drift) ?? []
        waivers = try values.decodeIfPresent([SettingsValue].self, forKey: .waivers) ?? []
        reviews = try values.decodeIfPresent([SettingsValue].self, forKey: .reviews) ?? []
        reproductionRuns = try values.decodeIfPresent([SettingsValue].self, forKey: .reproductionRuns) ?? []
        reproductionResults = try values.decodeIfPresent([SettingsValue].self, forKey: .reproductionResults) ?? []
        capsuleBuilds = try values.decodeIfPresent([SettingsValue].self, forKey: .capsuleBuilds) ?? []
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
    static let sources = "native_publication_sources"
    static let mutate = "native_publication_mutate"
}

@MainActor
final class NativePublicationModel: ObservableObject {
    private struct Scope: Hashable {
        var database: URL?
        var project: String
    }
    @Published var presented = false
    @Published var draft = PublicationDraft()
    @Published private(set) var projectID: String?
    @Published private(set) var workspace = PublicationWorkspaceRecord(publications: [], publication: nil, revision: nil, items: [])
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    @Published var selectedItemID: String?
    @Published var selectedBindingID: String?
    @Published private var uncertainProjects = Set<Scope>()
    @Published private var refreshedProjects = Set<Scope>()
    private var pendingProjects = Set<Scope>()
    private var drafts: [Scope: PublicationDraft] = [:]
    private var scope: Scope?
    private var requestedPublicationID: String?
    private var requestedRevisionID: String?
    private var generation = UUID()

    var selectedItem: PublicationItemRecord? { workspace.items.first { $0.id == selectedItemID } }
    var visibleBindings: [PublicationBindingRecord] {
        workspace.bindings.filter { selectedItemID == nil || $0.itemID == selectedItemID || $0.supportedClaimItemID == selectedItemID }
    }
    var selectedBinding: PublicationBindingRecord? { visibleBindings.first { $0.id == selectedBindingID } }
    var scopeIdentity: String? { scope.map { ($0.database?.absoluteString ?? "memory") + "\0" + $0.project } }
    var uncertain: Bool { scope.map { uncertainProjects.contains($0) } ?? false }
    var canAcknowledge: Bool { !busy && scope.map { refreshedProjects.contains($0) && !pendingProjects.contains($0) } == true }
    var canWrite: Bool { presented && !busy && !uncertain && projectID != nil }
    var editable: Bool { canWrite && workspace.revision?.state == "draft" }

    func acknowledgeResult() {
        guard canAcknowledge, let scope else { return }
        uncertainProjects.remove(scope)
        refreshedProjects.remove(scope)
        error = nil
    }

    func selectItem(_ id: String?) {
        selectedItemID = id
        selectedBindingID = nil
    }


    func open(projectID: String, databaseURL: URL? = nil) {
        if let previous = scope { drafts[previous] = draft }
        let scope = Scope(database: databaseURL?.resolvingSymlinksInPath().standardizedFileURL, project: projectID)
        self.scope = scope
        generation = UUID()
        self.projectID = projectID
        draft = drafts[scope] ?? PublicationDraft()
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
        canWrite && !draft.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            && !draft.revisionLabel.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    func reload(_ client: any NativeSettingsQuerying) async {
        guard presented, let projectID, !projectID.isEmpty, !busy else { return }
        var args: [String: SettingsValue] = [:]
        if let requestedPublicationID { args["publication_id"] = .string(requestedPublicationID) }
        if let requestedRevisionID { args["revision_id"] = .string(requestedRevisionID) }
        await send(client, command: NativePublicationCommand.read, args: args, projectID: projectID, clearDraft: false)
    }

    func selectPublication(_ id: String, client: any NativeSettingsQuerying) async {
        guard presented, !busy, workspace.publications.contains(where: { $0.id == id }) else { return }
        requestedPublicationID = id
        requestedRevisionID = nil
        await reload(client)
    }

    func selectRevision(_ id: String, client: any NativeSettingsQuerying) async {
        guard presented, !busy, workspace.revisions.contains(where: { $0.id == id && $0.publicationID == requestedPublicationID }) else { return }
        requestedRevisionID = id
        await reload(client)
    }

    @discardableResult
    func create(_ client: any NativeSettingsQuerying, expectedScope: String? = nil) async -> Bool {
        guard canWrite, expectedScope == nil || expectedScope == scopeIdentity, let projectID, !projectID.isEmpty else { return false }
        let title = draft.title.trimmingCharacters(in: .whitespacesAndNewlines)
        let label = draft.revisionLabel.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !title.isEmpty, !label.isEmpty else {
            error = localized("请填写论文标题和版本标签。")
            return false
        }
        return await send(
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

    @discardableResult
    func mutate(_ operation: PublicationOperation, revisionID: String, client: any NativeSettingsQuerying, expectedScope: String? = nil) async -> Bool {
        guard canWrite, expectedScope == nil || expectedScope == scopeIdentity, let projectID, workspace.revision?.id == revisionID,
              operation.permits(state: workspace.revision?.state ?? "") else { return false }
        if let validation = operation.validation(in: workspace) { error = localized(validation); return false }
        return await send(client, command: NativePublicationCommand.mutate,
                   args: ["revision_id": .string(revisionID), "operation": .object(operation.fields)],
                   projectID: projectID, clearDraft: false)
    }

    @discardableResult
    private func send(_ client: any NativeSettingsQuerying, command: String, args: [String: SettingsValue], projectID: String, clearDraft: Bool) async -> Bool {
        guard let scope, scope.project == projectID else { return false }
        let current = UUID()
        generation = current
        busy = true
        error = nil
        let writing = command != NativePublicationCommand.read
        let expectedPublication = requestedPublicationID
        let expectedRevision = requestedRevisionID
        if writing {
            uncertainProjects.insert(scope)
            pendingProjects.insert(scope)
            refreshedProjects.remove(scope)
        }
        // Keep selector choices while clearing all details during a read. A failed
        // switch must never display the previous revision as the requested one.
        if !writing {
            let choices = workspace.publications
            let revisions = workspace.revisions
            workspace = .empty
            workspace.publications = choices
            workspace.revisions = revisions
            selectedItemID = nil
            selectedBindingID = nil
        }
        defer {
            if writing { pendingProjects.remove(scope) }
            if generation == current { busy = false }
        }
        do {
            let value = try await client.invoke(command, args: args, projectID: projectID)
            var page = try JSONDecoder().decode(PublicationWorkspaceRecord.self, from: JSONEncoder().encode(command == NativePublicationCommand.mutate ? value["workspace"] : value))
            if command == NativePublicationCommand.mutate, value["readiness"] != .null {
                page.readiness = try JSONDecoder().decode(PublicationReadinessRecord.self, from: JSONEncoder().encode(value["readiness"]))
            }
            let cloning = args["operation"]?["action"].string == "clone_revision"
            guard page.publications.allSatisfy({ $0.projectID == projectID }),
                  (!writing || (page.publication != nil && page.revision != nil)),
                  page.publication.map({ $0.projectID == projectID }) ?? true,
                  (clearDraft || (expectedPublication.map({ $0 == page.publication?.id }) ?? true)),
                  (clearDraft || cloning || (expectedRevision.map({ $0 == page.revision?.id }) ?? true)),
                  page.revisions.allSatisfy({ $0.publicationID == page.publication?.id }),
                  page.revision.map({ $0.publicationID == page.publication?.id }) ?? true,
                  page.items.allSatisfy({ $0.revisionID == nil || $0.revisionID == page.revision?.id }),
                  page.bindings.allSatisfy({ $0.revisionID == page.revision?.id }),
                  (!cloning || (page.revision?.state == "draft" && page.revision?.id != expectedRevision)),
                  page.readiness.map({ $0.revisionID == page.revision?.id }) ?? true else {
                throw ProjectBrowserError.service(localized("论文或修订作用域不匹配。"))
            }
            if writing { uncertainProjects.remove(scope) }
            if clearDraft {
                let submitted = PublicationDraft(title: args["title"]?.string ?? "", description: args["description"]?.string ?? "", revisionLabel: args["revision_label"]?.string ?? "")
                if drafts[scope] == submitted { drafts[scope] = PublicationDraft() }
                if self.scope == scope && draft == submitted { draft = PublicationDraft() }
            }
            guard generation == current, presented, self.projectID == projectID else { return false }
            if !writing && uncertain { refreshedProjects.insert(scope) }
            workspace = page
            requestedPublicationID = page.publication?.id
            requestedRevisionID = page.revision?.id
            if !workspace.items.contains(where: { $0.id == selectedItemID }) { selectedItemID = nil }
            if !workspace.bindings.contains(where: { $0.id == selectedBindingID }) { selectedBindingID = nil }
            error = nil
            return true
        } catch {
            guard generation == current, presented, self.projectID == projectID else { return false }
            let lead = localized(writing ? "论文证据写入结果未确认。草稿已保留，请刷新核对；不会自动重试。" : "论文证据未能确认读取，不会自动重试。")
            self.error = lead + "\n" + error.localizedDescription
            return false
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
                      "message_span": "消息片段", "tool_call": "工具调用", "code_cell": "代码单元", "external_resource": "外部资源",
                      "exact": "精确", "likely": "可能", "uncertain": "不确定", "archived": "已归档", "traceable": "可追溯",
                      "re_executable": "可重新执行", "reproduced": "已重现"]
        return localized(labels[value] ?? value)
    }
}

struct NativePublicationColumn: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var publication: NativePublicationModel
    @State private var editor: PublicationEditorTarget?
    @State private var acknowledge = false

    var body: some View {
        GeometryReader { geometry in
            VStack(alignment: .leading, spacing: 16) {
                if geometry.size.width >= 760 {
                    HStack { heading; Spacer(); navigation }
                } else {
                    VStack(alignment: .leading, spacing: 8) { heading; HStack { Spacer(); navigation } }
                }
                actions
                if !publication.workspace.publications.isEmpty { selectors }
                if publication.uncertain {
                    Text(localized("写入结果未确认。请刷新检查当前论文，再允许继续编辑。")).foregroundStyle(.orange)
                    Button(localized("已核对，允许继续编辑…")) { acknowledge = true }.disabled(!publication.canAcknowledge)
                }
                if let error = publication.error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
                if publication.busy {
                    ProgressView(localized("正在处理论文证据…")).frame(maxWidth: .infinity, maxHeight: .infinity)
                } else if publication.workspace.publications.isEmpty {
                    Text(localized("还没有论文证据。新建论文后可整理结构和关联精确证据。")).foregroundStyle(.secondary)
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
        .sheet(item: $editor) { target in
            NativePublicationEditor(publication: publication, client: model.calendarClient(), target: target) { editor = nil }
        }
        .alert(localized("允许继续编辑？"), isPresented: $acknowledge) {
            Button(localized("取消"), role: .cancel) {}
            Button(localized("已核对结果")) { publication.acknowledgeResult() }
        } message: { Text(localized("确认已检查论文和修订的最新状态。再次提交可能产生新的记录。")) }
    }

    private var heading: some View {
        HStack {
            Text(localized("论文证据")).font(.title2.bold())
            if let revision = publication.workspace.revision { Text(NativePublicationLabel.text(revision.state)).font(.caption).foregroundStyle(.secondary) }
        }
    }
    private var navigation: some View {
        HStack {
            Button(localized("刷新")) { Task { await publication.reload(model.calendarClient()) } }.disabled(publication.busy)
            Button(localized("返回对话")) { publication.dismiss() }
        }
    }

    private var actions: some View {
        HStack {
            Button(localized("新建论文")) { editor = PublicationEditorTarget(kind: .create, page: publication.workspace) }.disabled(!publication.canWrite)
            Menu(localized("编辑修订")) {
                Button { present(.item(nil)) } label: { HStack { WispIcon(name: "plus", size: 16); Text(localized("新增结构条目")) } }.disabled(!publication.editable)
                Button { present(.evidence) } label: { HStack { WispIcon(name: "link", size: 16); Text(localized("关联证据")) } }.disabled(!publication.editable)
                Button { present(.clone) } label: { HStack { WispIcon(name: "copy", size: 16); Text(localized("复制为新修订")) } }.disabled(!publication.canWrite)
                Button { present(.readiness) } label: { HStack { WispIcon(name: "check", size: 16); Text(localized("检查与冻结")) } }.disabled(!publication.editable)
                Button { present(.reproduction) } label: { HStack { WispIcon(name: "archive-export", size: 16); Text(localized("重现与导出")) } }.disabled(!publication.canWrite || !["frozen", "published"].contains(publication.workspace.revision?.state ?? ""))
            }.disabled(publication.workspace.revision == nil || publication.busy)
            Spacer()
        }
    }
    private func present(_ kind: PublicationEditorKind) {
        editor = PublicationEditorTarget(kind: kind, page: publication.workspace, selectedItemID: publication.selectedItemID)
    }

    private var selectors: some View {
        VStack(alignment: .leading, spacing: 8) {
            Picker(localized("论文"), selection: Binding(get: { publication.workspace.publication?.id ?? "" }, set: { id in
                Task { await publication.selectPublication(id, client: model.calendarClient()) }
            })) {
                Text(localized("选择论文")).tag("")
                ForEach(publication.workspace.publications) { Text($0.title).tag($0.id) }
            }.disabled(publication.busy)
            Picker(localized("修订"), selection: Binding(get: { publication.workspace.revision?.id ?? "" }, set: { id in
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
            Button(String(format: localized("全部证据（%d）"), publication.workspace.bindings.count)) { publication.selectItem(nil) }
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
                HStack {
                    Text(item.title).font(.title3.bold())
                    Spacer()
                    Button(localized("编辑条目")) { present(.item(item)) }.disabled(!publication.editable)
                }
                if let content = item.content, !content.isEmpty { Text(content).textSelection(.enabled) }
            } else if let paper = publication.workspace.publication {
                Text(paper.title).font(.title3.bold())
                if !paper.description.isEmpty { Text(paper.description).textSelection(.enabled) }
            }
            Text(String(format: localized("关联证据（%d）"), publication.visibleBindings.count)).font(.headline)
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
                Text("\(localized("来源 ID")): \(binding.sourceID)")
                Text("\(localized("证据 ID")): \(binding.id)")
                Text("\(localized("修订 ID")): \(binding.revisionID)")
                Text("\(localized("可见性")): \(NativePublicationLabel.text(binding.visibility))")
                if publication.editable {
                    Menu(localized("选用状态")) {
                        ForEach(["candidate", "selected", "rejected"], id: \.self) { state in
                            Button(NativePublicationLabel.text(state)) { update(binding, selection: state, visibility: binding.visibility) }
                        }
                    }
                    Menu(localized("证据可见性")) {
                        ForEach(["private", "restricted", "public"], id: \.self) { visibility in
                            Button(NativePublicationLabel.text(visibility)) { update(binding, selection: binding.selectionState, visibility: visibility) }
                        }
                    }
                }
                if let lineage = publication.workspace.lineage.first(where: { $0["binding_id"].string == binding.id }) {
                    Text(localized("证据沿袭")).font(.headline)
                    Text("\(lineage["source_label"].string) · \(NativePublicationLabel.text(lineage["quality"].string))")
                    if !lineage["checksum"].string.isEmpty { Text("SHA-256: \(lineage["checksum"].string)").font(.caption.monospaced()) }
                    if !lineage["producing_run_id"].string.isEmpty { Text("\(localized("来源运行")): \(lineage["producing_run_title"].string)") }
                    ForEach(Array((lineage["input_labels"].array + lineage["code_labels"].array).enumerated()), id: \.offset) { _, value in Text(value.string).font(.caption) }
                }
                if let drift = publication.workspace.drift.first(where: { $0["binding_id"].string == binding.id }), drift["has_drift"].bool {
                    Text(localized("来源已有新版本；当前证据仍绑定原始版本。")).foregroundStyle(.orange)
                }
                Text(localized("来源快照")).font(.headline)
                Text(binding.sourceSnapshotJSON).font(.system(.caption, design: .monospaced))
            }
            NativePublicationReports(workspace: publication.workspace)
        }.textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
    }
    private func update(_ binding: PublicationBindingRecord, selection: String, visibility: String) {
        let scope = publication.scopeIdentity; let client = model.calendarClient()
        Task { await publication.mutate(.updateBinding(binding.id, selection, visibility), revisionID: binding.revisionID, client: client, expectedScope: scope) }
    }
}
