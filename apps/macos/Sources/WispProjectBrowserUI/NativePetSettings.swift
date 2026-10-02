import AppKit
import SwiftUI
import WispProjectBrowser

struct NativePetSettings: View {
    @ObservedObject var model: NativeSettingsModel
    private var status: SettingsValue { model.values["get_pet"] ?? .null }
    private var asset: SettingsValue { status["asset"] }

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            NativeSettingsGroup(title: "已保存的桌宠") {
                if model.loading && status == .null {
                    ProgressView(localized("正在读取桌宠…"))
                } else if !status["error"].string.isEmpty {
                    Text(status["error"].string).foregroundStyle(.red).textSelection(.enabled)
                } else if asset != .null {
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .top, spacing: 20) { preview; metadata.frame(minWidth: 240, maxWidth: .infinity, alignment: .leading) }
                        VStack(alignment: .leading, spacing: 16) { preview; metadata }
                    }
                } else if status != .null && !status["enabled"].bool && !status["directory"].string.isEmpty {
                    Text(localized("桌宠已关闭")).font(.headline)
                    Text(status["directory"].string).font(.caption).textSelection(.enabled)
                    Text(localized("已保留资源目录。启用并保存后读取桌宠预览。")).foregroundStyle(.secondary)
                } else if status != .null {
                    Text(localized("尚未配置桌宠")).font(.headline)
                    Text(localized("选择有效的桌宠资源目录并保存后，可在这里查看资源信息。")).foregroundStyle(.secondary)
                } else {
                    Text(localized("桌宠信息不可用，请重新载入。")).foregroundStyle(.secondary)
                }
                Text(localized("此处预览已保存资源的静止帧。目录草稿在保存后重新读取；取消不会更改当前桌宠。")).font(.caption).foregroundStyle(.secondary)
            }
            NativeSettingsGroup(title: "桌宠设置") {
                Text(localized("全局设置。保存会提交通用、对话、桌宠和同步的全部设置草稿。")).font(.caption).foregroundStyle(.secondary)
                NativeSettingsField(field: .init(key: "pet_enabled", label: "启用桌宠", kind: .toggle, hint: "保存后更新桌宠显示；关闭不会删除资源。"), value: model.binding("get_settings", "pet_enabled"))
                NativeSettingsField(field: .init(key: "pet_directory", label: "资源目录", kind: .directory, hint: "选择包含 pet.json 及其指定图像文件的目录"), value: model.binding("get_settings", "pet_directory"))
                HStack {
                    Spacer()
                    Button(localized("取消")) { model.discardDrafts() }
                    Button(localized("保存")) { Task { await model.saveSettings() } }.buttonStyle(NativeSettingsButtonStyle(primary: true))
                }
            }.disabled(model.loading || model.busy || model.values["get_settings"] == nil)
            if let runtime = model.values["get_pet_runtime_status"] {
                NativeSettingsGroup(title: "任务状态") {
                    Text(localized("桌宠反映当前任务状态，以下数据只读。" )).font(.caption).foregroundStyle(.secondary)
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 24) { counts(runtime) }
                        VStack(alignment: .leading, spacing: 12) { counts(runtime) }
                    }
                }
            }
        }
    }
    private var metadata: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(asset["displayName"].string).font(.title3.bold()).fixedSize(horizontal: false, vertical: true)
            if !asset["description"].string.isEmpty { Text(asset["description"].string).fixedSize(horizontal: false, vertical: true) }
            Text(asset["id"].string + " · v" + asset["spriteVersionNumber"].string).font(.caption).textSelection(.enabled)
            Text(status["directory"].string).font(.caption).foregroundStyle(.secondary).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
        }
    }
    @ViewBuilder private var preview: some View {
        if let frame = Self.firstFrame(asset["spritesheetDataUrl"].string) {
            Image(nsImage: frame).resizable().scaledToFit().frame(width: 120, height: 130).accessibilityLabel(asset["displayName"].string)
        } else {
            Text(localized("预览不可用")).foregroundStyle(.secondary).frame(width: 120, height: 130)
        }
    }
    @ViewBuilder private func counts(_ runtime: SettingsValue) -> some View {
        ForEach([("running", "运行中"), ("waiting", "等待审批"), ("reviewing", "审核中")], id: \.0) { key, label in
            VStack(alignment: .leading, spacing: 4) { Text("\(runtime[key].array.count)").font(.title2.monospacedDigit()); Text(localized(label)).foregroundStyle(.secondary) }
        }
    }
    static func firstFrame(_ dataURL: String) -> NSImage? {
        guard dataURL.hasPrefix("data:image/"), let comma = dataURL.firstIndex(of: ","),
              let data = Data(base64Encoded: String(dataURL[dataURL.index(after: comma)...])),
              let source = NSBitmapImageRep(data: data), let image = source.cgImage,
              image.width == 1536, image.height == 2288,
              let frame = image.cropping(to: CGRect(x: 0, y: 0, width: 192, height: 208)) else { return nil }
        return NSImage(cgImage: frame, size: NSSize(width: 192, height: 208))
    }
}
