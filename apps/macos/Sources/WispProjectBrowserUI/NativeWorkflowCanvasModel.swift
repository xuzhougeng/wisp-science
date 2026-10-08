import Foundation
import SwiftUI
import WispProjectBrowser

struct NativeWorkflowLayout {
    struct Node: Identifiable {
        let id: Int
        let level: Int
        let point: CGPoint
    }
    struct Edge: Identifiable, Equatable {
        let source: Int
        let target: Int
        var id: String { "\(source):\(target)" }
    }
    let nodes: [Node]
    let edges: [Edge]
    let size: CGSize
    let stageCount: Int
    init(tasks: [SettingsValue]) {
        var byID: [String: Int] = [:]
        for (index, task) in tasks.enumerated() where byID[task["id"].string] == nil { byID[task["id"].string] = index }
        var pending = Set(tasks.indices), levels = Array(repeating: 0, count: tasks.count)
        while !pending.isEmpty {
            let ready = pending.sorted().filter { index in
                tasks[index]["depends_on"].array.allSatisfy { dependency in byID[dependency.string].map { !pending.contains($0) } ?? true }
            }
            if ready.isEmpty {
                let fallback = (levels.max() ?? 0) + 1
                for index in pending { levels[index] = fallback }
                break
            }
            for index in ready {
                levels[index] = tasks[index]["depends_on"].array.compactMap { byID[$0.string] }.map { levels[$0] + 1 }.max() ?? 0
                pending.remove(index)
            }
        }
        var rows: [Int: Int] = [:], nodes: [Node] = []
        for index in tasks.indices {
            let level = levels[index], row = rows[level, default: 0]
            rows[level] = row + 1
            nodes.append(Node(id: index, level: level, point: CGPoint(x: 28 + level * 320, y: 65 + row * 174)))
        }
        self.nodes = nodes
        edges = tasks.enumerated().flatMap { index, task in task["depends_on"].array.compactMap { dependency in byID[dependency.string].map { Edge(source: $0, target: index) } } }
        stageCount = tasks.isEmpty ? 0 : (levels.max() ?? 0) + 1
        size = CGSize(width: max(520, 56 + stageCount * 320 - 80), height: max(320, 90 + (rows.values.max() ?? 0) * 174))
    }
    static func fit(size: CGSize, viewport: CGSize) -> CGFloat {
        max(0.25, min(1.4, min(viewport.width * 0.9 / max(1, size.width), viewport.height * 0.9 / max(1, size.height))))
    }
    static func point(_ screen: CGPoint, camera: CGPoint, zoom: CGFloat) -> CGPoint {
        CGPoint(x: (screen.x - camera.x) / zoom, y: (screen.y - camera.y) / zoom)
    }
}

@MainActor
final class NativeWorkflowDraft: ObservableObject, Identifiable {
    let id = UUID()
    @Published var template: SettingsValue
    @Published var selectedNode: Int?
    @Published var selectedEdge: NativeWorkflowLayout.Edge?
    @Published var positions: [Int: CGPoint] = [:]
    @Published var zoom: CGFloat = 1
    @Published var camera = CGPoint(x: 20, y: 20)
    @Published var busy = false
    @Published var error: String?
    @Published var message: String?
    @Published private(set) var uncertain = false
    @Published private(set) var reconciled = false
    @Published private(set) var persisted: SettingsValue?
    private var baseline: SettingsValue
    private var generation = UUID()
    let sourceHash: SettingsValue
    let client: any NativeSettingsQuerying
    let projectID: String?
    private(set) var removable: Bool
    init(template: SettingsValue, sourceHash: SettingsValue = .null, client: any NativeSettingsQuerying, projectID: String?, removable: Bool = false) {
        self.template = template; baseline = template; self.sourceHash = sourceHash; self.client = client; self.projectID = projectID
        self.removable = removable
    }
    var tasks: [SettingsValue] { template["proposal"]["tasks"].array }
    var layout: NativeWorkflowLayout { NativeWorkflowLayout(tasks: tasks) }
    var readOnly: Bool { template["builtin"].bool }
    var dirty: Bool { template != baseline }
    var canWrite: Bool { !readOnly && !busy && !uncertain }
    func close() { generation = UUID() }
    func point(_ node: NativeWorkflowLayout.Node) -> CGPoint { positions[node.id] ?? node.point }
    func moveNode(_ index: Int, origin: CGPoint, viewportTranslation: CGSize) {
        guard tasks.indices.contains(index) else { return }
        // The gesture reports unscaled viewport distances; convert exactly once.
        let delta = NativeWorkflowLayout.point(CGPoint(x: viewportTranslation.width, y: viewportTranslation.height), camera: .zero, zoom: zoom)
        positions[index] = CGPoint(x: origin.x + delta.x, y: origin.y + delta.y)
    }
    func fit(_ viewport: CGSize) { zoom = NativeWorkflowLayout.fit(size: layout.size, viewport: viewport); camera = CGPoint(x: 20, y: 20); positions = [:] }
    func binding(_ key: String, proposal: Bool = false) -> Binding<SettingsValue> {
        Binding(get: { proposal ? self.template["proposal"][key] : self.template[key] }, set: { value in
            guard self.canWrite else { return }
            if proposal { self.template["proposal"][key] = value } else { self.template[key] = value }
        })
    }
    func addNode() {
        guard canWrite, tasks.count < 8 else { return }
        let ids = Set(tasks.map { $0["id"].string })
        let id = (1...).lazy.map { "task_\($0)" }.first { !ids.contains($0) }!
        var updated = tasks
        updated.append(.object(["id": .string(id), "instruction": .string(""), "depends_on": .array([]), "capabilities": .array([]), "skill_ids": .array([]), "task_kind": .string("agent"), "isolated": .bool(false), "output_schema": .null, "specialist_id": .null, "model_id": .null, "executor": .null, "budget": .null, "timeout_secs": .null, "run_activity": .null]))
        template["proposal"]["tasks"] = .array(updated); selectedNode = updated.count - 1; selectedEdge = nil
    }
    func updateNode(_ index: Int, value: SettingsValue) -> Bool {
        guard canWrite, tasks.indices.contains(index) else { return false }
        let newID = value["id"].string, oldID = tasks[index]["id"].string
        guard Self.validID(newID), !tasks.enumerated().contains(where: { $0.offset != index && $0.element["id"].string == newID }) else { error = localized("节点 ID 必须唯一，并使用小写字母开头的 1–31 位字母、数字、下划线或连字符。"); return false }
        var updated = tasks; updated[index] = value
        if newID != oldID {
            updated = updated.map { task in
                var task = task
                task["depends_on"] = .array(task["depends_on"].array.map { $0.string == oldID ? .string(newID) : $0 })
                if task["run_activity"]["input_task_id"].string == oldID { task["run_activity"]["input_task_id"] = .string(newID) }
                return task
            }
        }
        template["proposal"]["tasks"] = .array(updated); error = nil; return true
    }
    func removeNode(_ index: Int) {
        guard canWrite, tasks.indices.contains(index) else { return }
        let id = tasks[index]["id"].string
        var updated = tasks; updated.remove(at: index)
        updated = updated.map { task in var task = task; task["depends_on"] = .array(task["depends_on"].array.filter { $0.string != id }); return task }
        template["proposal"]["tasks"] = .array(updated); selectedNode = nil; selectedEdge = nil; positions = [:]
    }
    func dependency(source: Int, target: Int, enabled: Bool) {
        guard canWrite, source != target, tasks.indices.contains(source), tasks.indices.contains(target) else { return }
        var updated = tasks, dependencies = updated[target]["depends_on"].array
        let id = updated[source]["id"]
        if enabled && !dependencies.contains(id) { dependencies.append(id) }
        if !enabled { dependencies.removeAll { $0 == id } }
        updated[target]["depends_on"] = .array(dependencies)
        if enabled && Self.graphIssues(updated).contains(localized("依赖关系存在循环。")) { error = localized("此连接会形成循环依赖。"); return }
        template["proposal"]["tasks"] = .array(updated); error = nil; selectedEdge = enabled ? .init(source: source, target: target) : nil
    }
    static func validID(_ id: String) -> Bool {
        let bytes = Array(id.utf8)
        return (1...31).contains(bytes.count) && bytes.first.map { (97...122).contains($0) } == true && bytes.allSatisfy { (97...122).contains($0) || (48...57).contains($0) || $0 == 95 || $0 == 45 }
    }
    static func graphIssues(_ tasks: [SettingsValue]) -> [String] {
        var issues: [String] = [], ids = Set<String>()
        for task in tasks {
            let id = task["id"].string
            if !validID(id) { issues.append(localized("无效节点 ID：") + id) }
            if !ids.insert(id).inserted { issues.append(localized("重复节点 ID：") + id) }
        }
        for task in tasks {
            let deps = task["depends_on"].array.map(\.string)
            if Set(deps).count != deps.count { issues.append(localized("重复依赖：") + task["id"].string) }
            for dep in deps where !ids.contains(dep) { issues.append(localized("缺少依赖节点：") + dep) }
        }
        var completed = Set<String>()
        while completed.count < ids.count {
            let ready = tasks.filter { !completed.contains($0["id"].string) && $0["depends_on"].array.allSatisfy { completed.contains($0.string) } }
            if ready.isEmpty { if !issues.contains(where: { $0.hasPrefix(localized("缺少依赖节点：")) }) { issues.append(localized("依赖关系存在循环。")) }; break }
            completed.formUnion(ready.map { $0["id"].string })
        }
        return issues
    }
    var issues: [String] {
        var result = Self.graphIssues(tasks)
        if template["name"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || template["name"].string.unicodeScalars.count > 100 { result.append(localized("名称必须包含 1–100 个字符。")) }
        if template["description"].string.unicodeScalars.count > 500 { result.append(localized("说明不能超过 500 个字符。")) }
        let proposal = template["proposal"]
        if proposal["goal"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || proposal["goal"].string.unicodeScalars.count > 2000 { result.append(localized("目标必须包含 1–2000 个字符。")) }
        if proposal["context"].string.unicodeScalars.count > 12000 { result.append(localized("上下文不能超过 12000 个字符。")) }
        if tasks.isEmpty || tasks.count > 8 { result.append(localized("工作流必须包含 1–8 个节点。")) }
        for task in tasks {
            if task["instruction"].string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || task["instruction"].string.unicodeScalars.count > 8000 { result.append(localized("节点指令必须包含 1–8000 个字符：") + task["id"].string) }
            if !task["skill_ids"].array.isEmpty { result.append(localized("旧技能绑定需要先转换：") + task["id"].string) }
            if task["task_kind"].string != "run_activity" && task["capabilities"].array.isEmpty { result.append(localized("节点需要至少一个工具能力：") + task["id"].string) }
        }
        return result
    }
    func copy() {
        guard !busy && !uncertain else { return }
        template["id"] = .string(UUID().uuidString); template["name"] = .string(template["name"].string + " " + localized("副本")); template["builtin"] = .bool(false)
        baseline = template; removable = false; error = nil; message = localized("副本尚未保存。"); persisted = nil
    }
    func save() async -> Bool {
        guard canWrite else { return false }
        guard issues.isEmpty else { error = issues.joined(separator: "\n"); return false }
        busy = true; error = nil; let current = generation
        var prepared = template
        prepared["proposal"]["tasks"] = .array(tasks.map { row in
            var row = row
            for key in ["specialist_id", "model_id"] where row[key].string.isEmpty { row[key] = .null }
            return row
        })
        var args = ["template": prepared]; if sourceHash != .null { args["conversionSourceSha256"] = sourceHash }
        do {
            let saved = try await client.invoke("save_workflow_template", args: args, projectID: projectID)
            guard current == generation else { return false }
            guard saved["id"] == prepared["id"], saved["proposal"]["tasks"].array.count == prepared["proposal"]["tasks"].array.count else { throw ProjectBrowserError.invalidResponse }
            template = saved; baseline = saved; busy = false; message = localized("已保存"); return true
        } catch {
            guard current == generation else { return false }
            busy = false; uncertain = true; reconciled = false; self.error = localized("保存结果未确认。请重新读取模板并核对，再决定是否继续编辑。") + "\n" + error.localizedDescription
            return false
        }
    }
    func reconcile() async {
        guard uncertain && !busy else { return }
        busy = true; let current = generation
        do {
            let rows = try await client.invoke("list_workflow_templates", args: [:], projectID: projectID)
            guard current == generation else { return }
            persisted = rows.array.first { $0["id"] == template["id"] }; reconciled = true; busy = false
        } catch { if current == generation { busy = false; self.error = error.localizedDescription } }
    }
    func remove() async -> Bool {
        guard removable && canWrite else { return false }
        busy = true; error = nil; let current = generation
        do {
            let rows = try await client.invoke("remove_workflow_template", args: ["templateId": template["id"]], projectID: projectID)
            guard current == generation else { return false }
            guard case .array = rows, !rows.array.contains(where: { $0["id"] == template["id"] }) else { throw ProjectBrowserError.invalidResponse }
            busy = false; return true
        } catch {
            guard current == generation else { return false }
            busy = false; uncertain = true; reconciled = false; self.error = localized("删除结果未确认，请重新读取保存状态并核对。") + "\n" + error.localizedDescription
            return false
        }
    }
    func acknowledge() { guard uncertain && reconciled && !busy else { return }; uncertain = false; error = nil; message = localized("已核对保存状态，草稿保留。") }
}
