import SwiftUI
import WispProjectBrowser

@MainActor final class NativeToolchainModel: ObservableObject {
    @Published private(set) var status: SettingsValue?
    @Published private(set) var error: String?
    @Published private(set) var reading = false
    @Published private(set) var writing = false
    @Published private(set) var uncertain = false
    let client: any NativeSettingsQuerying
    private var generation = UUID()
    init(client: any NativeSettingsQuerying) { self.client = client }
    var running: Bool { status?["state"].string == "running" }
    func close() { generation = UUID(); reading = false; writing = false }
    func refresh() async {
        guard !reading else { return }
        let token = generation; reading = true
        defer { if token == generation { reading = false } }
        do {
            let value = try await client.invoke("get_toolchain_status", args: [:], projectID: nil)
            guard token == generation else { return }
            guard ["idle", "running", "done", "failed", "cancelled"].contains(value["state"].string) else { throw ProjectBrowserError.invalidResponse }
            status = value
            if !uncertain { error = nil }
        } catch { if token == generation { self.error = error.localizedDescription } }
    }
    func acknowledge() { if status != nil && !reading && !writing { uncertain = false; error = nil } }
    func install(_ tools: Set<String>) async {
        guard !running, !uncertain, status != nil, !tools.isEmpty, tools.isSubset(of: ["uv", "python", "r", "node"]) else { return }
        await mutate("install_toolchain", ["tools": .array(tools.sorted().map(SettingsValue.string))])
    }
    func cancel() async { guard running else { return }; await mutate("cancel_toolchain_install", [:]) }
    private func mutate(_ command: String, _ args: [String: SettingsValue]) async {
        guard !writing else { return }
        let token = generation; writing = true; uncertain = true
        do {
            _ = try await client.invoke(command, args: args, projectID: nil)
            guard token == generation else { return }
            writing = false; uncertain = false; await refresh()
        } catch {
            if token == generation { writing = false; status = nil; self.error = localized("安装操作结果未确认，请重新读取后核对。") + "\n" + error.localizedDescription }
        }
    }
}

struct NativeToolchainSettings: View {
    @StateObject private var model: NativeToolchainModel
    @State private var selected: Set<String> = ["uv", "python"]
    init(client: any NativeSettingsQuerying) { _model = StateObject(wrappedValue: NativeToolchainModel(client: client)) }
    var body: some View {
        NativeSettingsGroup(title: "安装到 Wisp") {
            Text(localized("将所选工具安装到 Wisp 管理的目录，并配置运行路径。")).font(.caption).foregroundStyle(.secondary)
            if let status = model.status {
                Text(status["root"].string).font(.caption).textSelection(.enabled)
                ForEach([("uv", "uv"), ("python", "Python"), ("r", "R"), ("node", "Node.js")], id: \.0) { id, label in
                    Toggle(isOn: Binding(get: { selected.contains(id) }, set: { if $0 { selected.insert(id) } else { selected.remove(id) } })) {
                        HStack { Text(label); Spacer(); Text(localized(status["installed"].array.contains(.string(id)) ? "已安装" : "未安装")).font(.caption).foregroundStyle(.secondary) }
                    }.disabled(model.running || model.writing)
                }
                if model.running {
                    ProgressView(value: Double(status["step"].integer), total: Double(max(1, status["total"].integer)))
                    Text(status["label"].string).font(.caption)
                }
                if !status["error"].string.isEmpty { Text(status["error"].string).foregroundStyle(.orange).textSelection(.enabled) }
                if !status["log"].array.isEmpty {
                    ScrollView { Text(status["log"].array.map(\.string).joined(separator: "\n")).font(.system(.caption, design: .monospaced)).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }.frame(maxHeight: 150)
                }
            }
            if let error = model.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            HStack {
                Button(localized("重新读取")) { Task { await model.refresh() } }.disabled(model.reading || model.writing)
                if model.uncertain { Button(localized("已核对结果，允许继续")) { model.acknowledge() }.disabled(model.status == nil || model.reading || model.writing) }
                Spacer()
                if model.running { Button(localized("停止安装")) { Task { await model.cancel() } }.disabled(model.writing) }
                else { Button(localized(model.status?["state"].string == "failed" ? "重试安装" : "安装所选工具")) { Task { await model.install(selected) } }.disabled(model.status == nil || selected.isEmpty || model.writing || model.uncertain) }
            }
        }.task {
            await model.refresh()
            while !Task.isCancelled {
                do { try await Task.sleep(nanoseconds: 1_000_000_000) } catch { return }
                if model.running { await model.refresh() }
            }
        }.onDisappear { model.close() }
    }
}
