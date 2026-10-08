import SwiftUI
import WispProjectBrowser

@MainActor
final class NativeSessionTimerModel: ObservableObject {
    let client: any NativeConversationQuerying
    let project: String
    let session: String
    @Published var minutes = 60
    @Published var prompt = ""
    @Published private(set) var timer: SettingsValue = .null
    @Published private(set) var busy = false
    @Published private(set) var error: String?
    @Published private(set) var uncertain = false
    @Published private(set) var refreshed = false
    var canWrite: Bool { !busy && !uncertain && refreshed }
    init(client: any NativeConversationQuerying, project: String, session: String) { self.client = client; self.project = project; self.session = session }
    func read() async {
        guard !busy else { return }; busy = true; refreshed = false; error = nil
        defer { busy = false }
        do {
            let value = try await invoke(["action": .string("read")])
            guard value == .null || value["project_id"].string == project && value["frame_id"].string == session && value["replace_previous_turn"].bool else { throw ProjectBrowserError.invalidResponse }
            timer = value; refreshed = true
            if prompt.isEmpty && value != .null { prompt = value["prompt"].string; minutes = max(1, Int(value["interval_secs"].integer / 60)) }
        } catch { self.error = error.localizedDescription }
    }
    func acknowledge() { guard !busy, refreshed else { return }; uncertain = false; error = nil }
    func write(_ operation: [String: SettingsValue]) async {
        guard canWrite else { return }; busy = true; uncertain = true; refreshed = false
        do { _ = try await invoke(operation); uncertain = false }
        catch { self.error = localized("定时器操作结果未确认，请刷新并核对；不会自动重试。") + "\n" + error.localizedDescription }
        busy = false
        if !uncertain { await read() }
    }
    private func invoke(_ operation: [String: SettingsValue]) async throws -> SettingsValue {
        try await client.invoke("native_conversation_timer", args: ["session_id": .string(session), "operation": .object(operation)], projectID: project)
    }
}

struct NativeSessionTimerSheet: View {
    @ObservedObject var model: NativeSessionTimerModel
    let close: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(localized("会话定时器")).font(.title2.bold()); Spacer(); Button(localized("关闭"), action: close).disabled(model.busy) }
            Text(localized("定时器复用当前会话，并替换上次完整轮次；会话忙碌时等待，应用关闭时暂停。")).foregroundStyle(.secondary)
            if let error = model.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            HStack { Button(localized("刷新")) { Task { await model.read() } }.disabled(model.busy); if model.uncertain { Button(localized("已核对结果，允许继续")) { model.acknowledge() }.disabled(!model.refreshed || model.busy) } }
            if model.timer != .null {
                Text(localized("下次运行") + ": " + Date(timeIntervalSince1970: TimeInterval(model.timer["next_run_at"].integer)).formatted())
                Toggle(localized("启用"), isOn: Binding(get: { model.timer["enabled"].bool }, set: { on in Task { await model.write(["action": .string("set_enabled"), "enabled": .bool(on)]) } })).disabled(!model.canWrite)
            }
            Stepper(localized("间隔分钟") + ": \(model.minutes)", value: $model.minutes, in: 1...525600)
            Text(localized("提示词")); TextEditor(text: $model.prompt).frame(minHeight: 100)
            HStack {
                Button(localized("保存定时器")) { Task { await model.write(["action": .string("set"), "expression": .string("\(model.minutes)m \(model.prompt)")]) } }.disabled(!model.canWrite || model.prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                if model.timer != .null { Button(localized("取消定时器"), role: .destructive) { Task { await model.write(["action": .string("cancel")]) } }.disabled(!model.canWrite) }
            }
        }.padding(24).frame(minWidth: 360, idealWidth: 520, minHeight: 360)
        .task { await model.read() }
        .background(NativeSettingsEscape(enabled: !model.busy, close: close))
    }
}
