import SwiftUI
import WispProjectBrowser

struct NativeWorkflowStudio: View {
    @ObservedObject var model: NativeSettingsModel
    let back: () -> Void
    @State private var draft: NativeWorkflowDraft?
    @State private var pending: SettingsValue?
    @State private var preferredID: String?
    @State private var leaving = false
    @State private var confirm = false
    @State private var conversion = false
    private var rows: [SettingsValue] { model.values["list_workflow_templates"]?.array ?? [] }
    var body: some View {
        HStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 14) {
                Button(localized("返回设置")) { select(nil) }
                Text(localized("工作流库")).font(.title2.bold())
                Text("\(rows.count) " + localized("个工作流")).font(.caption).foregroundStyle(.secondary)
                Button(localized("新建工作流")) { select(.object(["id": .string(UUID().uuidString), "name": .string(localized("新工作流")), "builtin": .bool(false), "proposal": .object(["goal": .string(""), "context": .string(""), "approval_policy": .string("review_all"), "tasks": .array([])])])) }
                Button(localized("从技能或旧模板转换")) { conversion = true }
                if model.loading { ProgressView() }
                if let error = model.error { Text(error).foregroundStyle(.orange) }
                ScrollView {
                    VStack(alignment: .leading, spacing: 8) {
                        ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                            Button { select(row) } label: {
                                VStack(alignment: .leading, spacing: 6) {
                                    Text(row["name"].string).font(.headline)
                                    Text(row["description"].string).font(.caption).lineLimit(3)
                                    let tasks = row["proposal"]["tasks"].array
                                    Text("\(tasks.count) " + localized("个任务") + " · \(NativeWorkflowLayout(tasks: tasks).stageCount) " + localized("个阶段")).font(.caption)
                                }.frame(maxWidth: .infinity, alignment: .leading).padding(12)
                                    .background(draft?.template["id"] == row["id"] ? Color.accentColor.opacity(0.12) : Color.secondary.opacity(0.05), in: RoundedRectangle(cornerRadius: 10))
                            }.buttonStyle(.plain)
                        }
                    }
                }
            }.padding(16).frame(width: 230).disabled(draft?.busy == true)
            Divider()
            if let draft {
                NativeWorkflowCanvasEditor(draft: draft, embedded: true, close: { self.draft = nil }, saved: { preferredID = draft.template["id"].string; Task { await model.load() } }).id(draft.id)
            } else {
                VStack(spacing: 12) { Text(localized("选择工作流或新建工作流")).font(.title2); Text(localized("在画布编排任务，并在右侧编辑配置。")) }.frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
            .background { if draft == nil { NativeSettingsEscape(enabled: !conversion && !confirm, close: back) } }
            .confirmationDialog(localized("放弃尚未保存的工作流修改？"), isPresented: $confirm) {
                Button(localized("放弃修改"), role: .destructive) { draft?.close(); if leaving { back() } else if let pending { install(pending) } }
                Button(localized("继续编辑"), role: .cancel) { pending = nil; leaving = false }
            }
            .sheet(isPresented: $conversion) {
                VStack { HStack { Text(localized("转换工作流")); Spacer(); Button(localized("关闭")) { conversion = false } }; ScrollView { NativeWorkflowSettings(model: model, conversionOnly: true) } }.padding(20).frame(minWidth: 620, minHeight: 500).background(NativeSettingsEscape { conversion = false })
            }
            .onChange(of: model.readRevision) { _ in if draft == nil, let first = rows.first(where: { $0["id"].string == preferredID }) ?? rows.first { install(first) } }
            .onAppear { if draft == nil, let first = rows.first(where: { $0["id"].string == preferredID }) ?? rows.first { install(first) } }
    }
    private func select(_ value: SettingsValue?) {
        guard draft?.busy != true, draft?.uncertain != true else { return }
        if draft?.dirty == true { pending = value; leaving = value == nil; confirm = true }
        else if let value { draft?.close(); install(value) } else { draft?.close(); back() }
    }
    private func install(_ row: SettingsValue) {
        preferredID = row["id"].string
        draft = NativeWorkflowDraft(template: row, sourceHash: .null, client: model.client, projectID: model.projectID, removable: rows.contains { $0["id"] == row["id"] } && !row["builtin"].bool, layoutStore: model.layoutScope.map { NativeWorkflowLayoutStore(scope: $0) })
    }
}
