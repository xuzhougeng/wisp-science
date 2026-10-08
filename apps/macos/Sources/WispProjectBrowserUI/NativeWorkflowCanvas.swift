import SwiftUI
import WispProjectBrowser

struct NativeWorkflowNodeDraft: Identifiable {
    let id = UUID()
    let index: Int
    let value: SettingsValue
}

struct NativeWorkflowCanvasEditor: View {
    @ObservedObject var draft: NativeWorkflowDraft
    let close: () -> Void
    let saved: () -> Void
    @State private var nodeEditor: NativeWorkflowNodeDraft?
    @State private var confirmDiscard = false
    @State private var confirmRemove = false
    @State private var confirmDelete = false
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ViewThatFits(in: .horizontal) {
                HStack { title; Spacer(); actions }
                VStack(alignment: .leading) { title; actions }
            }
            if let error = draft.error { Text(error).foregroundStyle(.orange).font(.caption).textSelection(.enabled) }
            if let message = draft.message { Text(message).foregroundStyle(.secondary).font(.caption) }
            if draft.uncertain { recovery }
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    NativeWorkflowGraph(draft: draft, edit: openNode).frame(height: 420)
                    inspector
                    metadata.disabled(!draft.canWrite)
                    validation
                }.padding(.trailing, 4)
            }
        }
        .padding(22).frame(minWidth: 420, idealWidth: 1060, maxWidth: .infinity, minHeight: 640, idealHeight: 800, maxHeight: .infinity)
        .background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme))
        .buttonStyle(WispButtonStyle()).font(WispDesign.font(size: 14))
        .sheet(item: $nodeEditor) { editor in NativeWorkflowNodeEditor(draft: draft, editor: editor) { nodeEditor = nil } }
        .interactiveDismissDisabled(draft.dirty || draft.busy || draft.uncertain || nodeEditor != nil)
        .background(NativeSettingsEscape(enabled: nodeEditor == nil && !confirmDiscard && !confirmRemove && !confirmDelete) { requestClose() })
        .confirmationDialog(localized("放弃尚未保存的修改？"), isPresented: $confirmDiscard) {
            Button(localized("放弃修改"), role: .destructive) { draft.close(); close() }
            Button(localized("继续编辑"), role: .cancel) {}
        }
        .confirmationDialog(localized("移除节点及其依赖连线？计算活动的输入配置需要重新核对。"), isPresented: $confirmRemove) {
            Button(localized("移除节点"), role: .destructive) { if let index = draft.selectedNode { draft.removeNode(index) } }
            Button(localized("取消"), role: .cancel) {}
        }
        .confirmationDialog(localized("确认删除？此操作会移除当前配置。"), isPresented: $confirmDelete) {
            Button(localized("删除"), role: .destructive) { Task { if await draft.remove() { saved(); draft.close(); close() } } }
            Button(localized("取消"), role: .cancel) {}
        }
    }
    private var metadata: some View {
        VStack(alignment: .leading, spacing: 18) {
                    NativeSettingsField(field: .init(key: "name", label: "名称"), value: draft.binding("name"))
                    NativeSettingsField(field: .init(key: "description", label: "说明", kind: .multiline), value: draft.binding("description"))
                    NativeSettingsField(field: .init(key: "goal", label: "目标", kind: .multiline), value: draft.binding("goal", proposal: true))
                    DisclosureGroup(localized("共享上下文与审批策略")) {
                        NativeSettingsField(field: .init(key: "context", label: "上下文", kind: .multiline), value: draft.binding("context", proposal: true))
                        NativeSettingsField(field: .init(key: "approval_policy", label: "审批策略", kind: .choice([("review_all", "逐项审核"), ("auto_safe", "自动执行安全操作")])), value: draft.binding("approval_policy", proposal: true))
                    }
        }
    }
    private var title: some View {
        HStack { Text(localized("工作流画布")).font(.title2.weight(.semibold)); if draft.readOnly { Text(localized("内置 · 只读")).foregroundStyle(.secondary).font(.caption) } }
    }
    private var actions: some View {
        HStack(spacing: 10) {
            Button(localized("复制")) { draft.copy() }.disabled(draft.busy || draft.uncertain)
            if draft.removable && !draft.readOnly { Button(localized("删除…"), role: .destructive) { confirmDelete = true }.disabled(!draft.canWrite) }
            Button(localized("关闭")) { requestClose() }.disabled(draft.busy || draft.uncertain)
            if !draft.readOnly { Button(localized("保存模板")) { Task { if await draft.save() { saved(); draft.close(); close() } } }.buttonStyle(NativeSettingsButtonStyle(primary: true)).disabled(!draft.canWrite || !draft.issues.isEmpty) }
            if draft.busy { ProgressView().controlSize(.small) }
        }
    }
    private var recovery: some View {
        VStack(alignment: .leading, spacing: 10) {
            Button(localized("重新读取保存状态")) { Task { await draft.reconcile() } }.disabled(draft.busy)
            if draft.reconciled {
                Text(draft.persisted == nil ? localized("当前没有此 ID 的模板。") : draft.persisted?["proposal"] == draft.template["proposal"] ? localized("当前保存的工作流定义与草稿相同。") : localized("当前已保存版本与草稿不同，请核对。"))
                if let persisted = draft.persisted { Text(persisted["name"].string + " · " + persisted["id"].string).font(.caption).textSelection(.enabled) }
                Button(localized("已核对，保留草稿继续编辑")) { draft.acknowledge() }
            }
        }.padding(12).frame(maxWidth: .infinity, alignment: .leading).background(Color.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
    }
    @ViewBuilder private var inspector: some View {
        if let index = draft.selectedNode, draft.tasks.indices.contains(index) {
            let node = draft.tasks[index]
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    Text(node["id"].string).fontWeight(.semibold).textSelection(.enabled); Spacer()
                    Button(localized("编辑节点")) { openNode(index) }.disabled(draft.busy || draft.uncertain)
                    Button(localized("移除节点"), role: .destructive) { confirmRemove = true }.disabled(!draft.canWrite)
                }
                Text(node["instruction"].string).lineLimit(3).foregroundStyle(.secondary).textSelection(.enabled)
                Text(localized("依赖节点")).fontWeight(.medium)
                if draft.tasks.count == 1 { Text(localized("添加其他节点后可建立依赖。")).foregroundStyle(.secondary) }
                ForEach(Array(draft.tasks.enumerated()), id: \.offset) { source, row in
                    if source != index {
                        Toggle(row["id"].string, isOn: Binding(get: { node["depends_on"].array.contains(row["id"]) }, set: { draft.dependency(source: source, target: index, enabled: $0) })).disabled(!draft.canWrite)
                    }
                }
            }.padding(14).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 10))
        } else if let edge = draft.selectedEdge, draft.tasks.indices.contains(edge.source), draft.tasks.indices.contains(edge.target) {
            HStack {
                Text(draft.tasks[edge.source]["id"].string + " → " + draft.tasks[edge.target]["id"].string).textSelection(.enabled)
                Spacer(); Button(localized("移除依赖")) { draft.dependency(source: edge.source, target: edge.target, enabled: false) }.disabled(!draft.canWrite)
            }
        } else { Text(localized("选择节点编辑指令与依赖，拖动空白区域平移，拖动节点调整位置。")).foregroundStyle(.secondary) }
    }
    private var validation: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(localized("结构校验")).fontWeight(.semibold)
            if draft.issues.isEmpty { Text(localized("结构校验通过。保存时还会核对工具能力、活动配置和预算。")).foregroundStyle(.secondary) }
            else { ForEach(Array(draft.issues.enumerated()), id: \.offset) { _, issue in Text(issue).foregroundStyle(.orange).textSelection(.enabled) } }
        }
    }
    private func openNode(_ index: Int) { guard draft.tasks.indices.contains(index), !draft.busy && !draft.uncertain else { return }; nodeEditor = .init(index: index, value: draft.tasks[index]) }
    private func requestClose() { guard !draft.busy && !draft.uncertain else { return }; if draft.dirty { confirmDiscard = true } else { draft.close(); close() } }
}

struct NativeWorkflowGraph: View {
    @ObservedObject var draft: NativeWorkflowDraft
    let edit: (Int) -> Void
    @State private var dragOrigins: [Int: CGPoint] = [:]
    @State private var panOrigin: CGPoint?
    @State private var zoomOrigin: CGFloat?
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            GeometryReader { viewport in
                ZStack(alignment: .topLeading) {
                    WispDesign.color("bg-sunken", scheme).contentShape(Rectangle()).gesture(DragGesture().onChanged { value in
                        if panOrigin == nil { panOrigin = draft.camera }
                        draft.camera = CGPoint(x: panOrigin!.x + value.translation.width, y: panOrigin!.y + value.translation.height)
                    }.onEnded { _ in panOrigin = nil })
                    graph.scaleEffect(draft.zoom, anchor: .topLeading).offset(x: draft.camera.x, y: draft.camera.y)
                    toolbar(viewport.size).padding(10).frame(width: viewport.size.width, height: viewport.size.height, alignment: .bottomTrailing)
                }.frame(width: viewport.size.width, height: viewport.size.height, alignment: .topLeading).coordinateSpace(name: "workflowCanvasViewport").clipped().overlay(RoundedRectangle(cornerRadius: 10).stroke(WispDesign.color("border", scheme)))
                    .simultaneousGesture(MagnificationGesture().onChanged { value in
                        if zoomOrigin == nil { zoomOrigin = draft.zoom }; draft.zoom = max(0.25, min(2, zoomOrigin! * value))
                    }.onEnded { _ in zoomOrigin = nil })
            }
            Text("\(draft.tasks.count) " + localized("节点") + " · \(draft.layout.stageCount) " + localized("阶段") + " · \(Int(draft.zoom * 100))%").font(.caption).foregroundStyle(.secondary)
        }
    }
    private var graph: some View {
        let layout = draft.layout
        return ZStack(alignment: .topLeading) {
            ForEach(0..<layout.stageCount, id: \.self) { stage in
                Text(localized("阶段") + " \(stage + 1)").font(.caption.weight(.semibold)).foregroundStyle(.secondary).position(x: CGFloat(148 + stage * 320), y: 28)
            }
            ForEach(layout.edges) { edge in edgeView(edge, layout: layout) }
            ForEach(layout.nodes) { node in nodeView(node) }
            if draft.tasks.isEmpty { Text(localized("添加第一个节点以建立工作流。")).foregroundStyle(.secondary).position(x: 230, y: 150) }
        }.frame(width: layout.size.width, height: layout.size.height)
    }
    private func toolbar(_ size: CGSize) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 8) { add; zoomOut; zoomIn; fit(size) }
            VStack(alignment: .trailing, spacing: 8) { HStack { add; fit(size) }; HStack { zoomOut; zoomIn } }
        }.fixedSize(horizontal: false, vertical: true).padding(8).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8)).buttonStyle(WispButtonStyle())
    }
    private var add: some View { Button { draft.addNode() } label: { WispIcon(name: "plus") }.help(localized("添加节点")).accessibilityLabel(localized("添加节点")).disabled(!draft.canWrite || draft.tasks.count >= 8) }
    private var zoomOut: some View { Button(localized("缩小")) { draft.zoom = max(0.25, draft.zoom / 1.2) } }
    private var zoomIn: some View { Button(localized("放大")) { draft.zoom = min(2, draft.zoom * 1.2) } }
    private func fit(_ size: CGSize) -> some View { Button(localized("适应画布")) { draft.fit(size) } }
    private func nodeView(_ node: NativeWorkflowLayout.Node) -> some View {
        let task = draft.tasks[node.id], point = draft.point(node)
        return VStack(alignment: .leading, spacing: 9) {
            HStack { Text(task["id"].string).fontWeight(.semibold).lineLimit(1); Spacer(); WispIcon(name: task["task_kind"].string == "run_activity" ? "terminal" : "sparkles", size: 16) }
            Text(task["instruction"].string.isEmpty ? localized("填写节点指令") : task["instruction"].string).lineLimit(3).frame(maxWidth: .infinity, alignment: .leading)
            Text(localized("依赖") + ": \(task["depends_on"].array.count) · " + localized("能力") + ": \(task["capabilities"].array.count)").font(.caption).foregroundStyle(.secondary)
        }.padding(14).frame(width: 240, height: 140, alignment: .topLeading)
            .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(draft.selectedNode == node.id ? WispDesign.color("clay", scheme) : WispDesign.color("border", scheme), lineWidth: draft.selectedNode == node.id ? 2 : 1))
            .contentShape(RoundedRectangle(cornerRadius: 12))
            .onTapGesture(count: 2) { draft.selectedNode = node.id; draft.selectedEdge = nil; edit(node.id) }
            .onTapGesture { draft.selectedNode = node.id; draft.selectedEdge = nil }
            .gesture(DragGesture(minimumDistance: 4, coordinateSpace: .named("workflowCanvasViewport")).onChanged { value in
                if dragOrigins[node.id] == nil { dragOrigins[node.id] = point }
                draft.moveNode(node.id, origin: dragOrigins[node.id]!, viewportTranslation: value.translation)
            }.onEnded { _ in dragOrigins[node.id] = nil })
            .position(x: point.x + 120, y: point.y + 70)
            .accessibilityElement(children: .combine).accessibilityLabel(task["id"].string + ", " + task["instruction"].string)
            .accessibilityAddTraits(.isButton).accessibilityAction { draft.selectedNode = node.id; edit(node.id) }
    }
    private func edgeView(_ edge: NativeWorkflowLayout.Edge, layout: NativeWorkflowLayout) -> some View {
        let start = draft.point(layout.nodes[edge.source]), end = draft.point(layout.nodes[edge.target])
        let a = CGPoint(x: start.x + 240, y: start.y + 70), b = CGPoint(x: end.x, y: end.y + 70), bend = max(32, abs(b.x - a.x) / 2)
        var path = Path(); path.move(to: a); path.addCurve(to: b, control1: CGPoint(x: a.x + bend, y: a.y), control2: CGPoint(x: b.x - bend, y: b.y))
        return ZStack {
            path.stroke(draft.selectedEdge == edge ? WispDesign.color("clay", scheme) : WispDesign.color("text-muted", scheme), lineWidth: draft.selectedEdge == edge ? 3 : 1.5)
            path.strokedPath(StrokeStyle(lineWidth: 18)).fill(Color.clear).contentShape(path.strokedPath(StrokeStyle(lineWidth: 18))).onTapGesture { draft.selectedEdge = edge; draft.selectedNode = nil }
            Path { p in p.move(to: CGPoint(x: b.x - 7, y: b.y - 4)); p.addLine(to: b); p.addLine(to: CGPoint(x: b.x - 7, y: b.y + 4)) }.stroke(WispDesign.color("text-muted", scheme), lineWidth: 1.5)
        }.accessibilityElement(children: .ignore).accessibilityLabel(draft.tasks[edge.source]["id"].string + " → " + draft.tasks[edge.target]["id"].string).accessibilityAddTraits(.isButton).accessibilityAction { draft.selectedEdge = edge; draft.selectedNode = nil }
    }
}

struct NativeWorkflowNodeEditor: View {
    @ObservedObject var draft: NativeWorkflowDraft
    let editor: NativeWorkflowNodeDraft
    let close: () -> Void
    @State private var value: SettingsValue
    @State private var confirmDiscard = false
    init(draft: NativeWorkflowDraft, editor: NativeWorkflowNodeDraft, close: @escaping () -> Void) { self.draft = draft; self.editor = editor; self.close = close; _value = State(initialValue: editor.value) }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(localized("编辑节点")).font(.title2.weight(.semibold))
            if let error = draft.error { Text(error).foregroundStyle(.orange).font(.caption) }
            ScrollView { VStack(spacing: 16) { ForEach(Self.fields) { field in NativeSettingsField(field: field, value: Binding(get: { value[field.key] }, set: { value[field.key] = $0 })) } }.padding(.trailing, 4).disabled(!draft.canWrite) }
            HStack { Spacer(); Button(localized("取消")) { requestClose() }; if !draft.readOnly { Button(localized("应用到画布")) { if draft.updateNode(editor.index, value: value) { close() } }.buttonStyle(NativeSettingsButtonStyle(primary: true)).disabled(!draft.canWrite) } }
        }.padding(22).frame(minWidth: 390, idealWidth: 600, minHeight: 600, idealHeight: 720)
            .interactiveDismissDisabled(value != editor.value).background(NativeSettingsEscape(enabled: !confirmDiscard) { requestClose() })
            .confirmationDialog(localized("放弃尚未保存的修改？"), isPresented: $confirmDiscard) { Button(localized("放弃修改"), role: .destructive) { close() }; Button(localized("继续编辑"), role: .cancel) {} }
    }
    private func requestClose() { if value != editor.value { confirmDiscard = true } else { close() } }
    static let fields: [SettingsField] = [
        .init(key: "id", label: "节点 ID"), .init(key: "instruction", label: "指令", kind: .multiline),
        .init(key: "depends_on", label: "依赖节点", kind: .lines), .init(key: "capabilities", label: "工具能力", kind: .lines),
        .init(key: "task_kind", label: "任务类型", kind: .choice([("agent", "智能体"), ("run_activity", "计算活动")])),
        .init(key: "run_activity", label: "计算活动配置", kind: .json), .init(key: "output_schema", label: "输出 JSON Schema", kind: .json),
        .init(key: "specialist_id", label: "专家 ID"), .init(key: "model_id", label: "模型 ID"),
        .init(key: "isolated", label: "独立上下文", kind: .toggle), .init(key: "timeout_secs", label: "超时秒数", kind: .integer),
        .init(key: "executor", label: "执行器", kind: .json), .init(key: "budget", label: "预算", kind: .json)
    ]
}
