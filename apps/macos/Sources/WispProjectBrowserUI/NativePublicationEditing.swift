import AppKit
import SwiftUI
import WispProjectBrowser

// Wire keys and fixtures come from wisp-dto::native_publication. Mutations use
// the existing publication service, including its snapshot and freeze checks.
struct PublicationReadinessRecord: Codable, Equatable {
    var revisionID: String
    var targetVisibility: String
    var capabilityLevel: String
    var blockers: [PublicationFindingRecord]
    var warnings: [PublicationFindingRecord]
    var omissions: [PublicationFindingRecord]
    var manifestSHA256: String
    var canFreeze: Bool
    enum CodingKeys: String, CodingKey {
        case blockers, warnings, omissions
        case revisionID = "revision_id", targetVisibility = "target_visibility", capabilityLevel = "capability_level"
        case manifestSHA256 = "manifest_sha256", canFreeze = "can_freeze"
    }
}

struct PublicationFindingRecord: Codable, Equatable {
    var code: String
    var message: String
    var bindingID: String?
    var sourceID: String?
    var waivable: Bool
    var waived: Bool
    enum CodingKeys: String, CodingKey {
        case code, message, waivable, waived
        case bindingID = "binding_id", sourceID = "source_id"
    }
}

struct PublicationItemDraft: Equatable {
    var id: String = UUID().uuidString
    var parentID = ""
    var kind = "section"
    var title = ""
    var content = ""
    var ordinal: Int64 = 0
    init(item: PublicationItemRecord? = nil, ordinal: Int64 = 0) {
        self.ordinal = ordinal
        if let item {
            id = item.id; parentID = item.parentItemID ?? ""; kind = item.kind
            title = item.title; content = item.content ?? ""; self.ordinal = item.ordinal
        }
    }
}

struct PublicationEvidenceDraft: Equatable {
    var sourceKind = "artifact_version"
    var sourceID = ""
    var sourceLabel = ""
    var itemID = ""
    var claimID = ""
    var purpose = ""
    var selection = "selected"
    var visibility = "private"
}

struct PublicationFreezePolicy: Equatable {
    var visibility = "private"
    var personalDataReviewed = false
    var redistributionReviewed = false
    var snapshotRestrictedBytes = false
    var value: SettingsValue { .object([
        "target_visibility": .string(visibility), "phi_pii_reviewed": .bool(personalDataReviewed),
        "redistribution_reviewed": .bool(redistributionReviewed), "snapshot_restricted_bytes": .bool(snapshotRestrictedBytes),
    ]) }
}

enum PublicationOperation: Equatable {
    case saveItem(PublicationItemDraft)
    case bindEvidence(PublicationEvidenceDraft)
    case updateBinding(String, String, String)
    case cloneRevision(String)
    case saveWaiver(String, String, String)
    case check(PublicationFreezePolicy)
    case freeze(PublicationFreezePolicy)
    case verify(String)
    case buildCapsule(String)

    func permits(state: String) -> Bool {
        switch self {
        case .cloneRevision: return state == "draft" || state == "frozen" || state == "published"
        case .verify, .buildCapsule: return state == "frozen" || state == "published"
        default: return state == "draft"
        }
    }

    func validation(in page: PublicationWorkspaceRecord) -> String? {
        switch self {
        case .saveItem(let item):
            if item.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || item.ordinal < 0 { return "请填写条目标题和非负顺序。" }
            if !item.parentID.isEmpty && (item.parentID == item.id || !page.items.contains(where: { $0.id == item.parentID })) { return "父条目必须属于当前修订且不能是自身。" }
        case .bindEvidence(let evidence):
            if evidence.sourceID.isEmpty || evidence.purpose.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return "请选择精确来源并说明证据用途。" }
            if !evidence.itemID.isEmpty && !page.items.contains(where: { $0.id == evidence.itemID }) { return "条目必须属于当前修订。" }
            if !evidence.claimID.isEmpty && !page.items.contains(where: { $0.id == evidence.claimID && $0.kind == "claim" }) { return "支持的论点必须属于当前修订。" }
        case .updateBinding(let id, _, _):
            if !page.bindings.contains(where: { $0.id == id }) { return "证据必须属于当前修订。" }
        case .cloneRevision(let label):
            if label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return "请填写版本标签。" }
        case .saveWaiver(let code, let author, let reason):
            if code.isEmpty || author.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || reason.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { return "请填写检查项、审阅人和豁免理由。" }
        case .verify(let id):
            if id.isEmpty || !page.lineage.contains(where: { $0["producing_run_id"].string == id }) { return "请选择当前修订的来源运行。" }
        case .buildCapsule(let path):
            if !path.hasPrefix("/") || !path.lowercased().hasSuffix(".zip") { return "请选择 ZIP 文件的绝对保存路径。" }
        default: break
        }
        return nil
    }

    var fields: [String: SettingsValue] {
        func optional(_ value: String) -> SettingsValue { value.isEmpty ? .null : .string(value) }
        switch self {
        case .saveItem(let item):
            return ["action": .string("save_item"), "id": .string(item.id), "parent_item_id": optional(item.parentID),
                    "kind": .string(item.kind), "title": .string(item.title), "content": .string(item.content), "ordinal": .integer(item.ordinal)]
        case .bindEvidence(let evidence):
            return ["action": .string("bind_evidence"), "item_id": optional(evidence.itemID), "supported_claim_item_id": optional(evidence.claimID),
                    "source_kind": .string(evidence.sourceKind), "source_id": .string(evidence.sourceID), "purpose": .string(evidence.purpose),
                    "selection_state": .string(evidence.selection), "visibility": .string(evidence.visibility)]
        case .updateBinding(let id, let selection, let visibility):
            return ["action": .string("update_binding"), "binding_id": .string(id), "selection_state": .string(selection), "visibility": .string(visibility)]
        case .cloneRevision(let label): return ["action": .string("clone_revision"), "label": .string(label)]
        case .saveWaiver(let code, let author, let reason):
            return ["action": .string("save_waiver"), "finding_code": .string(code), "author": .string(author), "reason": .string(reason)]
        case .check(let policy): return ["action": .string("check"), "policy": policy.value]
        case .freeze(let policy): return ["action": .string("freeze"), "policy": policy.value]
        case .verify(let id): return ["action": .string("verify"), "source_run_id": .string(id), "comparisons": .array([])]
        case .buildCapsule(let path): return ["action": .string("build_capsule"), "destination": .string(path)]
        }
    }
}

struct PublicationSourceRecord: Codable, Identifiable, Equatable {
    var kind: String
    var id: String
    var title: String
    var detail: String
    var text: String?
    var textSHA256: String?
    var frameID: String?
    var messageSeq: Int64?
    enum CodingKeys: String, CodingKey {
        case kind, id, title, detail, text
        case textSHA256 = "text_sha256", frameID = "frame_id", messageSeq = "message_seq"
    }

    func evidence(utf16Range: NSRange? = nil) throws -> (kind: String, id: String) {
        guard kind == "message_span" else {
            guard ["artifact_version", "run"].contains(kind), !id.isEmpty else { throw ProjectBrowserError.invalidResponse }
            return (kind, id)
        }
        guard let text, let frameID, !frameID.isEmpty, let messageSeq, let textSHA256,
              textSHA256.count == 64, textSHA256.allSatisfy({ $0.isHexDigit }) else { throw ProjectBrowserError.invalidResponse }
        let selection = utf16Range ?? NSRange(location: 0, length: text.utf16.count)
        guard selection.location >= 0, selection.length > 0, selection.location <= text.utf16.count,
              selection.length <= text.utf16.count - selection.location,
              let start = Self.byteOffset(selection.location, text: text),
              let end = Self.byteOffset(selection.location + selection.length, text: text) else { throw ProjectBrowserError.service(localized("请选择完整字符组成的非空消息片段。")) }
        guard end - start <= 64 * 1024 else { throw ProjectBrowserError.service(localized("消息证据片段不能超过 64 KiB。")) }
        let locator: SettingsValue = .object(["byte_start": .integer(Int64(start)), "byte_end": .integer(Int64(end)),
            "frame_id": .string(frameID), "message_seq": .integer(messageSeq), "message_content_sha256": .string(textSHA256)])
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return ("message_span", String(decoding: try encoder.encode(locator), as: UTF8.self))
    }
    private static func byteOffset(_ offset: Int, text: String) -> Int? {
        var units = 0; var bytes = 0
        for scalar in text.unicodeScalars {
            if units == offset { return bytes }
            units += scalar.value > 0xffff ? 2 : 1
            bytes += String(scalar).utf8.count
        }
        return units == offset ? bytes : nil
    }
}

@MainActor
final class PublicationSourceModel: ObservableObject {
    @Published var kind = "files"
    @Published var query = ""
    @Published private(set) var offset: Int64 = 0
    @Published private(set) var sources: [PublicationSourceRecord] = []
    @Published private(set) var hasMore = false
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    @Published var selected: PublicationSourceRecord?
    @Published var selection: NSRange?
    private var generation = UUID()
    func invalidate() { generation = UUID(); busy = false }
    func load(projectID: String, client: any NativeSettingsQuerying, offset: Int64 = 0) async {
        let ticket = UUID(); generation = ticket; busy = true; error = nil
        sources = []; selected = nil; selection = nil; self.offset = max(0, offset)
        defer { if generation == ticket { busy = false } }
        do {
            let result = try await client.invoke(NativePublicationCommand.sources,
                args: ["kind": .string(kind), "query": .string(query), "offset": .integer(self.offset)], projectID: projectID)
            guard generation == ticket else { return }
            let records = try JSONDecoder().decode([PublicationSourceRecord].self, from: JSONEncoder().encode(result["sources"]))
            guard records.allSatisfy({ !$0.id.isEmpty && ["artifact_version", "run", "message_span"].contains($0.kind) }) else { throw ProjectBrowserError.invalidResponse }
            sources = records; hasMore = result["has_more"].bool
        } catch { if generation == ticket { self.error = error.localizedDescription } }
    }
}

// An AppKit text selection exposes UTF-16 ranges; evidence stores UTF-8 bytes.
// Keep preview read-only so the content hash describes exactly what is quoted.
struct PublicationExcerptView: NSViewRepresentable {
    let text: String
    @Binding var selection: NSRange?
    func makeCoordinator() -> Coordinator { Coordinator(selection: $selection) }
    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSScrollView(); scroll.hasVerticalScroller = true; scroll.borderType = .bezelBorder
        let view = NSTextView(frame: NSRect(x: 0, y: 0, width: 400, height: 180)); view.isEditable = false; view.isSelectable = true
        view.font = .systemFont(ofSize: 13); view.autoresizingMask = [.width]; view.isVerticallyResizable = true
        view.isHorizontallyResizable = false; view.textContainer?.widthTracksTextView = true
        view.maxSize = NSSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude)
        view.delegate = context.coordinator; scroll.documentView = view
        return scroll
    }
    func updateNSView(_ scroll: NSScrollView, context: Context) {
        context.coordinator.selection = $selection
        if let view = scroll.documentView as? NSTextView, view.string != text { view.string = text; view.setSelectedRange(NSRange(location: 0, length: 0)) }
    }
    final class Coordinator: NSObject, NSTextViewDelegate {
        var selection: Binding<NSRange?>
        init(selection: Binding<NSRange?>) { self.selection = selection }
        func textViewDidChangeSelection(_ notification: Notification) {
            guard let view = notification.object as? NSTextView else { return }
            let range = view.selectedRange(); selection.wrappedValue = range.length == 0 ? nil : range
        }
    }
}

enum PublicationEditorKind {
    case create, item(PublicationItemRecord?), evidence, clone, readiness, reproduction
    var title: String {
        switch self {
        case .create: return "新建论文"
        case .item(let item): return item == nil ? "新增结构条目" : "编辑条目"
        case .evidence: return "关联证据"
        case .clone: return "复制为新修订"
        case .readiness: return "检查与冻结"
        case .reproduction: return "重现与导出"
        }
    }
}

struct PublicationEditorTarget: Identifiable {
    let id = UUID()
    let kind: PublicationEditorKind
    let page: PublicationWorkspaceRecord
    var selectedItemID: String?
}

struct NativePublicationEditor: View {
    @ObservedObject var publication: NativePublicationModel
    let client: any NativeSettingsQuerying
    let target: PublicationEditorTarget
    let close: () -> Void
    @StateObject private var sources = PublicationSourceModel()
    @State private var item: PublicationItemDraft
    @State private var evidence: PublicationEvidenceDraft
    @State private var label = ""
    @State private var policy = PublicationFreezePolicy()
    @State private var checkedPolicy: PublicationFreezePolicy?
    @State private var waiverCode = ""
    @State private var author = ""
    @State private var reason = ""
    @State private var runID = ""
    @State private var localError: String?
    @State private var advancedKind = "artifact_version"
    @State private var advancedID = ""
    @State private var alert: EditorAlert?
    private let originalItem: PublicationItemDraft
    private let originalEvidence: PublicationEvidenceDraft
    private let projectID: String?
    private let scopeID: String?
    private enum EditorAlert { case discard, acknowledge, freeze }

    init(publication: NativePublicationModel, client: any NativeSettingsQuerying, target: PublicationEditorTarget, close: @escaping () -> Void) {
        self.publication = publication; self.client = client; self.target = target; self.close = close
        projectID = publication.projectID
        scopeID = publication.scopeIdentity
        let record: PublicationItemRecord?
        if case .item(let existing) = target.kind { record = existing } else { record = nil }
        let item = PublicationItemDraft(item: record, ordinal: (target.page.items.map(\.ordinal).max() ?? -1) + 1)
        let evidence = PublicationEvidenceDraft(itemID: target.selectedItemID ?? "")
        originalItem = item; originalEvidence = evidence
        _item = State(initialValue: item); _evidence = State(initialValue: evidence)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text(localized(target.kind.title)).font(.title2.bold())
                Spacer()
                Button(localized("关闭"), action: attemptClose).disabled(publication.busy)
            }
            if let revision = target.page.revision { Text("\(revision.label) · \(NativePublicationLabel.text(revision.state))").foregroundStyle(.secondary) }
            if let error = localError ?? publication.error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            if publication.uncertain {
                Text(localized("写入结果未确认。请刷新检查当前论文，再允许继续编辑。")).foregroundStyle(.orange)
                HStack {
                    Button(localized("刷新")) { Task { await publication.reload(client) } }.disabled(publication.busy)
                    Button(localized("已核对，允许继续编辑…")) { alert = .acknowledge }.disabled(!publication.canAcknowledge)
                }
            }
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    switch target.kind {
                    case .create: creation
                    case .item: itemEditor
                    case .evidence: evidenceEditor
                    case .clone: cloneEditor
                    case .readiness: readinessEditor
                    case .reproduction: reproductionEditor
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }
            if publication.busy { ProgressView(localized("正在处理论文证据…")) }
        }
        .padding(24).frame(minWidth: 400, idealWidth: 640, minHeight: 420, idealHeight: 660)
        .background(NativeSettingsEscape(enabled: !publication.busy, close: attemptClose))
        .onDisappear { sources.invalidate() }
        .onChange(of: publication.projectID) { id in if id != projectID { sources.invalidate(); close() } }
        .onChange(of: publication.scopeIdentity) { id in if id != scopeID { sources.invalidate(); close() } }
        .alert(alertTitle, isPresented: Binding(get: { alert != nil }, set: { if !$0 { alert = nil } })) {
            Button(localized("取消"), role: .cancel) { alert = nil }
            switch alert {
            case .discard: Button(localized("放弃修改"), role: .destructive) { close() }
            case .acknowledge: Button(localized("已核对结果")) { publication.acknowledgeResult() }
            case .freeze: Button(localized("冻结当前修订")) { submit(.freeze(policy), closeOnSuccess: true) }
            case nil: EmptyView()
            }
        } message: { Text(localized(alert == .freeze ? "冻结会锁定当前修订及其证据快照。后续编辑需要复制为新修订。" : alert == .acknowledge ? "确认已检查论文和修订的最新状态。再次提交可能产生新的记录。" : "未保存的编辑会被丢弃。")) }
    }

    private var alertTitle: String {
        localized(alert == .freeze ? "冻结当前修订？" : alert == .acknowledge ? "允许继续编辑？" : "放弃未保存的修改？")
    }
    private var scopeMatches: Bool { publication.scopeIdentity == scopeID && publication.projectID == projectID && publication.workspace.revision?.id == target.page.revision?.id }
    private var canEdit: Bool { scopeMatches && publication.editable }

    private var creation: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField(localized("论文标题"), text: $publication.draft.title)
            TextField(localized("版本标签"), text: $publication.draft.revisionLabel)
            Text(localized("论文描述"))
            TextEditor(text: $publication.draft.description).frame(minHeight: 140)
            Button(localized("创建论文")) { Task { if await publication.create(client, expectedScope: scopeID) { close() } } }.disabled(!publication.canCreate || publication.scopeIdentity != scopeID)
        }
    }
    private var itemEditor: some View {
        VStack(alignment: .leading, spacing: 12) {
            TextField(localized("条目标题"), text: $item.title)
            Picker(localized("条目类型"), selection: $item.kind) {
                ForEach(["section", "claim", "figure", "table", "methods", "supplement"], id: \.self) { Text(NativePublicationLabel.text($0)).tag($0) }
            }
            Picker(localized("父条目"), selection: $item.parentID) {
                Text(localized("无父条目")).tag("")
                ForEach(target.page.items.filter { $0.id != item.id }) { Text($0.title).tag($0.id) }
            }
            TextField(localized("条目顺序"), value: $item.ordinal, format: .number)
            Text(localized("条目内容"))
            TextEditor(text: $item.content).frame(minHeight: 180)
            Button(localized("保存条目")) { submit(.saveItem(item), closeOnSuccess: true) }
                .disabled(!canEdit || item.title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || item.ordinal < 0)
        }
    }
    private var evidenceEditor: some View {
        VStack(alignment: .leading, spacing: 12) {
            if evidence.sourceID.isEmpty {
                sourcePicker
            } else {
                Text(evidence.sourceLabel).font(.headline)
                Text(NativePublicationLabel.text(evidence.sourceKind)).font(.caption).foregroundStyle(.secondary)
                Text(evidence.sourceID).font(.caption.monospaced()).textSelection(.enabled)
                Button(localized("重新选择来源")) { evidence.sourceID = "" }
                Picker(localized("关联条目"), selection: $evidence.itemID) {
                    Text(localized("整个修订")).tag("")
                    ForEach(target.page.items) { Text($0.title).tag($0.id) }
                }
                Picker(localized("支持的论点"), selection: $evidence.claimID) {
                    Text(localized("不关联论点")).tag("")
                    ForEach(target.page.items.filter { $0.kind == "claim" }) { Text($0.title).tag($0.id) }
                }
                TextField(localized("证据用途"), text: $evidence.purpose)
                Picker(localized("选用状态"), selection: $evidence.selection) {
                    ForEach(["candidate", "selected", "rejected"], id: \.self) { Text(NativePublicationLabel.text($0)).tag($0) }
                }
                Picker(localized("证据可见性"), selection: $evidence.visibility) { visibilityChoices }
                Button(localized("保存证据关联")) { submit(.bindEvidence(evidence), closeOnSuccess: true) }
                    .disabled(!canEdit || evidence.purpose.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }
    private var sourcePicker: some View {
        VStack(alignment: .leading, spacing: 12) {
            Picker(localized("证据来源"), selection: $sources.kind) {
                Text(localized("文件版本")).tag("files")
                Text(localized("运行")).tag("runs")
                Text(localized("消息片段")).tag("messages")
            }.onChange(of: sources.kind) { _ in loadSources() }
            HStack {
                TextField(localized("搜索来源"), text: $sources.query).onSubmit { loadSources() }
                Button(localized("搜索")) { loadSources() }.disabled(sources.busy)
            }
            if let error = sources.error { Text(error).foregroundStyle(.red) }
            if sources.busy { ProgressView() }
            if !sources.busy && sources.sources.isEmpty { Text(localized("没有匹配的证据来源。")).foregroundStyle(.secondary) }
            if !sources.sources.isEmpty {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 8) {
                        ForEach(sources.sources) { source in
                            Button {
                                sources.selected = source; sources.selection = nil
                            } label: {
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(source.title).fontWeight(.semibold).lineLimit(2)
                                    Text(source.kind == "artifact_version" ? "v\(source.detail)" : localized(source.detail == "assistant" ? "助手" : source.detail == "user" ? "用户" : source.detail))
                                        .font(.caption).foregroundStyle(.secondary)
                                    if let text = source.text { Text(String(text.prefix(90))).font(.caption).lineLimit(2) }
                                }.frame(maxWidth: .infinity, alignment: .leading).padding(8)
                                    .background(sources.selected?.id == source.id ? Color.accentColor.opacity(0.12) : Color.secondary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))
                            }.buttonStyle(.plain)
                        }
                    }
                }.frame(height: min(220, CGFloat(sources.sources.count) * 80))
            }
            HStack {
                Button(localized("上一页")) { loadSources(offset: max(0, sources.offset - 50)) }.disabled(sources.busy || sources.offset == 0)
                Button(localized("下一页")) { loadSources(offset: sources.offset + 50) }.disabled(sources.busy || !sources.hasMore)
            }
            if let source = sources.selected {
                if let text = source.text {
                    Text(localized("选择消息片段；未选择时使用显示的全文。预览最多显示前 16 KiB。")).font(.caption).foregroundStyle(.secondary)
                    PublicationExcerptView(text: text, selection: $sources.selection).frame(height: 180)
                }
                Button(localized("使用此精确来源")) {
                    do {
                        let reference = try source.evidence(utf16Range: sources.selection)
                        evidence.sourceKind = reference.kind; evidence.sourceID = reference.id; evidence.sourceLabel = source.title
                        localError = nil
                    } catch { localError = error.localizedDescription }
                }.disabled(sources.busy || !canEdit)
            }
            DisclosureGroup(localized("精确来源定位")) {
                VStack(alignment: .leading, spacing: 10) {
                    Text(localized("输入不可变来源 ID，或消息片段、工具调用的规范 JSON 定位符。")).font(.caption).foregroundStyle(.secondary)
                    Picker(localized("来源类型"), selection: $advancedKind) {
                        ForEach(["artifact_version", "run", "execution_log", "message_span", "tool_call", "code_cell", "external_resource"], id: \.self) {
                            Text(NativePublicationLabel.text($0)).tag($0)
                        }
                    }
                    TextField(localized("来源 ID 或定位符"), text: $advancedID)
                    Button(localized("使用精确定位符")) {
                        evidence.sourceKind = advancedKind; evidence.sourceID = advancedID
                        evidence.sourceLabel = NativePublicationLabel.text(advancedKind)
                    }.disabled(!canEdit || advancedID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                }.padding(.top, 8)
            }
        }.task { loadSources() }
    }
    private var cloneEditor: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(localized("结构与证据将复制到新的可编辑草稿。"))
            TextField(localized("版本标签"), text: $label)
            Button(localized("创建新修订")) { submit(.cloneRevision(label), closeOnSuccess: true) }
                .disabled(!scopeMatches || !publication.canWrite || label.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
    }
    private var visibilityChoices: some View {
        ForEach(["private", "restricted", "public"], id: \.self) { Text(NativePublicationLabel.text($0)).tag($0) }
    }
    private var readinessEditor: some View {
        VStack(alignment: .leading, spacing: 14) {
            Picker(localized("冻结可见性"), selection: $policy.visibility) { visibilityChoices }
            Toggle(localized("已审阅个人信息和敏感数据"), isOn: $policy.personalDataReviewed)
            Toggle(localized("已审阅再分发许可"), isOn: $policy.redistributionReviewed)
            Toggle(localized("在快照中包含受限来源字节"), isOn: $policy.snapshotRestrictedBytes)
            Button(localized("检查当前修订")) {
                let checked = policy
                Task {
                    if let revision = target.page.revision, await publication.mutate(.check(checked), revisionID: revision.id, client: client, expectedScope: scopeID) { checkedPolicy = checked }
                }
            }.disabled(!canEdit)
            if let readiness = publication.workspace.readiness {
                NativePublicationReadiness(readiness: readiness)
                let findings = readiness.blockers + readiness.warnings + readiness.omissions
                let waivableCodes = Array(Set(findings.filter { $0.waivable && !$0.waived }.map(\.code))).sorted()
                if !waivableCodes.isEmpty {
                    Divider()
                    Text(localized("检查项豁免")).font(.headline)
                    Picker(localized("检查项"), selection: $waiverCode) {
                        Text(localized("选择检查项")).tag("")
                        ForEach(waivableCodes, id: \.self) { Text($0).tag($0) }
                    }
                    TextField(localized("审阅人"), text: $author)
                    TextField(localized("豁免理由"), text: $reason)
                    Button(localized("保存豁免")) { checkedPolicy = nil; submit(.saveWaiver(waiverCode, author, reason), closeOnSuccess: false) }
                        .disabled(!canEdit || waiverCode.isEmpty || author.isEmpty || reason.isEmpty)
                }
                Button(localized("冻结当前修订…")) { alert = .freeze }
                    .disabled(!canEdit || !readiness.canFreeze || checkedPolicy != policy || readiness.revisionID != target.page.revision?.id)
                Text(localized("修改检查选项或证据后，需要重新检查才能冻结。")).font(.caption).foregroundStyle(.secondary)
            }
        }
    }
    private var reproductionEditor: some View {
        VStack(alignment: .leading, spacing: 14) {
            let runs = Dictionary(grouping: target.page.lineage.filter { !$0["producing_run_id"].string.isEmpty }, by: { $0["producing_run_id"].string })
            Text(localized("在隔离工作区重现冻结修订的来源运行，并比较输出。")).foregroundStyle(.secondary)
            if runs.isEmpty { Text(localized("当前修订没有可重现的来源运行。")).foregroundStyle(.secondary) }
            Picker(localized("来源运行"), selection: $runID) {
                Text(localized("选择来源运行")).tag("")
                ForEach(runs.keys.sorted(), id: \.self) { id in Text(runs[id]?.first?["producing_run_title"].string ?? id).tag(id) }
            }
            Button(localized("重现并验证")) { submit(.verify(runID), closeOnSuccess: false) }
                .disabled(!scopeMatches || !publication.canWrite || runID.isEmpty)
            Divider()
            Button(localized("导出证据胶囊…")) { exportCapsule() }.disabled(!scopeMatches || !publication.canWrite)
            NativePublicationReports(workspace: publication.workspace)
        }
    }
    private func loadSources(offset: Int64 = 0) {
        guard let projectID else { return }
        Task { await sources.load(projectID: projectID, client: client, offset: offset) }
    }
    private func submit(_ operation: PublicationOperation, closeOnSuccess: Bool) {
        guard let revision = target.page.revision, scopeMatches else { return }
        Task { if await publication.mutate(operation, revisionID: revision.id, client: client, expectedScope: scopeID), closeOnSuccess { close() } }
    }
    private func attemptClose() {
        guard !publication.busy else { return }
        switch target.kind {
        case .item where item != originalItem: alert = .discard
        case .evidence where evidence != originalEvidence: alert = .discard
        case .clone where !label.isEmpty: alert = .discard
        default: close()
        }
    }
    private func exportCapsule() {
        let panel = NSSavePanel(); panel.allowedContentTypes = [.zip]; panel.nameFieldStringValue = "evidence-capsule.zip"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        submit(.buildCapsule(url.path), closeOnSuccess: false)
    }
}

struct NativePublicationReadiness: View {
    let readiness: PublicationReadinessRecord
    var frozen = false
    var statusTitle: String { frozen ? "冻结检查记录" : readiness.canFreeze ? "检查通过，可冻结" : "当前修订尚不能冻结" }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(localized(statusTitle)).font(.headline)
            Text("\(NativePublicationLabel.text(readiness.targetVisibility)) · \(NativePublicationLabel.text(readiness.capabilityLevel))").font(.caption)
            findings(readiness.blockers, title: "阻止冻结", color: .red)
            findings(readiness.warnings, title: "警告", color: .orange)
            findings(readiness.omissions, title: "省略内容", color: .secondary)
            Text("SHA-256: \(readiness.manifestSHA256)").font(.caption.monospaced()).textSelection(.enabled)
        }
    }
    private func findings(_ findings: [PublicationFindingRecord], title: String, color: Color) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            if !findings.isEmpty { Text(localized(title)).fontWeight(.semibold).foregroundStyle(color) }
            ForEach(Array(findings.enumerated()), id: \.offset) { _, finding in
                Text("\(finding.waived ? localized("已豁免") + " · " : "")\(finding.message)").font(.caption)
            }
        }
    }
}

struct NativePublicationReports: View {
    let workspace: PublicationWorkspaceRecord
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if let readiness = workspace.readiness { NativePublicationReadiness(readiness: readiness, frozen: ["frozen", "published"].contains(workspace.revision?.state ?? "")) }
            records(workspace.waivers, title: "已记录的豁免", fields: ["finding_code", "author", "reason"])
            records(workspace.reviews, title: "证据审阅记录", fields: ["reviewer", "method", "result", "report_json"])
            records(workspace.reproductionRuns, title: "重现记录", fields: ["source_run_id", "status", "capability_level", "expected_environment_hash", "actual_environment_hash", "exit_code", "error", "stdout_tail", "stderr_tail"])
            records(workspace.reproductionResults, title: "输出比较", fields: ["output_path", "comparator_kind", "report_json"])
            records(workspace.capsuleBuilds, title: "证据胶囊", fields: ["status", "visibility", "output_path", "archive_sha256", "error"])
        }.textSelection(.enabled)
    }
    private func records(_ values: [SettingsValue], title: String, fields: [String]) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            if !values.isEmpty { Text(localized(title)).font(.headline) }
            ForEach(Array(values.enumerated()), id: \.offset) { _, value in
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(fields, id: \.self) { key in
                        if !value[key].string.isEmpty { Text("\(localized(fieldLabel(key))): \(value[key].string)").font(.caption) }
                    }
                    if value["passed"] != .null { Text(localized(value["passed"].bool ? "通过" : "未通过")).font(.caption).foregroundStyle(value["passed"].bool ? .green : .red) }
                    if value["environment_matched"] != .null { Text(localized(value["environment_matched"].bool ? "环境一致" : "环境不一致")).font(.caption).foregroundStyle(value["environment_matched"].bool ? .green : .orange) }
                }.padding(10).frame(maxWidth: .infinity, alignment: .leading)
                    .background(Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 6))
            }
        }
    }
    private func fieldLabel(_ key: String) -> String {
        ["finding_code": "检查项", "author": "审阅人", "reason": "豁免理由", "reviewer": "审阅人", "method": "审阅方法",
         "result": "审阅结果", "report_json": "详细报告", "source_run_id": "来源运行", "status": "状态", "capability_level": "证据能力",
         "error": "错误", "stdout_tail": "标准输出", "stderr_tail": "标准错误", "output_path": "输出路径", "comparator_kind": "比较方法",
         "archive_sha256": "归档 SHA-256", "visibility": "可见性", "expected_environment_hash": "预期环境校验值",
         "actual_environment_hash": "实际环境校验值", "exit_code": "退出代码"][key] ?? key
    }
}
