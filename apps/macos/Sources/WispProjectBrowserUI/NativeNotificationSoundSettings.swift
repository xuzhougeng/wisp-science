import SwiftUI
import WispProjectBrowser

struct NativeNotificationSoundSettings: View {
    @ObservedObject var model: NativeSettingsModel
    @State private var sounds: [SettingsValue] = []
    @State private var error: String?
    @State private var busy = false
    private var sound: String { model.values["get_settings"]?["notification_sound"].string ?? "" }
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            NativeSettingsSwitch(title: "提示音", hint: "任务完成或需要处理时播放；与桌面通知独立保存。", isOn: Binding(get: { model.values["get_settings"]?["notification_sound_enabled"].bool ?? false }, set: { model.binding("get_settings", "notification_sound_enabled").wrappedValue = .bool($0) }))
            HStack {
                Picker(localized("声音"), selection: Binding(get: { sound }, set: { model.binding("get_settings", "notification_sound").wrappedValue = .string($0) })) {
                    if !sounds.contains(where: { $0["id"].string == sound }) { Text(sound.isEmpty ? localized("请选择声音") : sound).tag(sound) }
                    ForEach(Array(sounds.enumerated()), id: \.offset) { _, item in Text(item["label"].string.isEmpty ? item["id"].string : item["label"].string).tag(item["id"].string) }
                }
                Button(localized("试听")) { Task { await preview() } }.disabled(busy || !sounds.contains(where: { $0["id"].string == sound }))
                Button(localized("重新读取")) { Task { await load() } }.disabled(busy)
            }
            if let error { Text(error).font(.caption).foregroundStyle(.orange) }
        }.disabled(model.values["get_settings"]?["notification_sound_enabled"] == nil)
            .task { await load() }
    }
    private func load() async {
        guard !busy else { return }; busy = true; defer { busy = false }
        do { sounds = try await model.client.invoke("list_notification_sounds", args: [:], projectID: nil).array; error = nil }
        catch { self.error = error.localizedDescription }
    }
    private func preview() async {
        guard !busy else { return }; busy = true; defer { busy = false }
        do { _ = try await model.client.invoke("preview_notification_sound", args: ["sound": .string(sound)], projectID: nil); error = nil }
        catch { self.error = error.localizedDescription }
    }
}
