import SwiftUI
import WispProjectBrowser

struct CapabilitySkill: Codable, Equatable {
    var scope: String
    var enabled: Bool
}

struct CapabilityConnection: Codable, Equatable {
    var enabled: Bool
}

struct CapabilitySummary: Equatable {
    var version = ""
    var workspace = ""
    var errors: [String] = []
    var bundledSkills = 0
    var projectSkills = 0
    var mcpConnections = 0
    var memoryFiles = 0
}

enum NativeCapabilityRead {
    static let commands = ["get_bootstrap_status", "list_skills", "list_mcp_connections", "get_memory_view"]
    static let probe = "probe_execution_context"

    static func skillCounts(_ skills: [CapabilitySkill]) -> (bundled: Int, project: Int) {
        let enabled = skills.filter(\.enabled)
        let bundled = enabled.filter { $0.scope == "bundled" }.count
        return (bundled, enabled.count - bundled)
    }
}

@MainActor
final class NativeCapabilitiesModel: ObservableObject {
    @Published var presented = false
    @Published private(set) var projectID: String?
    @Published private(set) var summary = CapabilitySummary()
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    private var generation = UUID()

    func open(projectID: String) {
        self.projectID = projectID
        presented = true
    }

    func dismiss() {
        guard !busy else { return }
        presented = false
    }

    func openSettings(_ section: String, in model: ProjectBrowserModel) {
        presented = false
        model.projectSettingsID = nil
        model.settingsSectionID = section
        model.settingsPresented = true
    }

    func invalidate() {
        generation = UUID()
        presented = false
    }

    func reload(_ client: any NativeSettingsQuerying) async {
        guard presented, let projectID, !projectID.isEmpty else { return }
        let requested = projectID
        let current = UUID()
        generation = current
        busy = true
        error = nil
        defer { if generation == current { busy = false } }
        do {
            let bootstrap = try await client.invoke("get_bootstrap_status", args: [:], projectID: requested)
            let skills = try await client.invoke("list_skills", args: [:], projectID: requested)
            let connections = try await client.invoke("list_mcp_connections", args: [:], projectID: requested)
            let memory = try await client.invoke("get_memory_view", args: ["project_id": .string(requested)], projectID: requested)
            guard generation == current, presented, self.projectID == requested else { return }
            let decodedSkills = try Self.decode([CapabilitySkill].self, skills)
            let counts = NativeCapabilityRead.skillCounts(decodedSkills)
            let enabledConnections = try Self.decode([CapabilityConnection].self, connections["connections"]).filter(\.enabled).count
            summary = CapabilitySummary(
                version: bootstrap["app_version"].string,
                workspace: bootstrap["workspace"].string,
                errors: bootstrap["errors"].array.compactMap { value in
                    if case .string(let text) = value { return text }
                    return nil
                },
                bundledSkills: counts.bundled,
                projectSkills: counts.project,
                mcpConnections: enabledConnections,
                memoryFiles: memory["files"].array.count)
            error = nil
        } catch {
            guard generation == current, presented, self.projectID == requested else { return }
            self.error = "能力未能确认读取，不会自动重试。\n" + error.localizedDescription
        }
    }

    private static func decode<T: Decodable>(_ type: T.Type, _ value: SettingsValue) throws -> T {
        try JSONDecoder().decode(type, from: JSONEncoder().encode(value))
    }
}

struct NativeCapabilitiesSheet: View {
    @ObservedObject var model: ProjectBrowserModel
    @ObservedObject var capabilities: NativeCapabilitiesModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text("能力").font(.headline)
                Spacer()
                Button("关闭") { capabilities.dismiss() }
            }
            if !capabilities.summary.version.isEmpty {
                Text("运行时 \(capabilities.summary.version)").font(.subheadline)
            }
            if !capabilities.summary.workspace.isEmpty {
                Text(capabilities.summary.workspace).font(.caption).foregroundStyle(.secondary).lineLimit(2)
            }
            if let error = capabilities.error {
                Text(error).font(.caption).foregroundStyle(.red).textSelection(.enabled)
            }
            ForEach(capabilities.summary.errors, id: \.self) { item in
                Text(item).font(.caption).foregroundStyle(.orange)
            }
            LazyVGrid(columns: [GridItem(.flexible()), GridItem(.flexible())], spacing: 8) {
                stat("\(capabilities.summary.bundledSkills)", "内置技能", "skills")
                stat("\(capabilities.summary.projectSkills)", "项目技能", "skills")
                stat("\(capabilities.summary.mcpConnections)", "连接", "connections")
                stat("\(capabilities.summary.memoryFiles)", "记忆文件", "memory")
            }
            Button(capabilities.busy ? "正在读取…" : "刷新") { Task { await capabilities.reload(model.calendarClient()) } }
                .disabled(capabilities.busy)
        }
        .padding(20)
        .frame(width: 420, height: 360)
        .interactiveDismissDisabled(capabilities.busy)
        .background(NativeSettingsEscape(enabled: !capabilities.busy) { capabilities.dismiss() })
        .task(id: capabilities.projectID ?? "") { await capabilities.reload(model.calendarClient()) }
    }

    private func stat(_ value: String, _ label: String, _ section: String) -> some View {
        Button {
            capabilities.openSettings(section, in: model)
        } label: {
            VStack {
                Text(value).font(.title2)
                Text(label).font(.caption)
            }
            .frame(maxWidth: .infinity, minHeight: 64)
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}
