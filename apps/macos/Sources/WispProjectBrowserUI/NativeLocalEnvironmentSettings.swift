import SwiftUI
import WispProjectBrowser

struct NativeLocalEnvironmentSettings: View {
    @ObservedObject var model: NativeSettingsModel
    var body: some View {
        NativeSettingsGroup(title: "本地运行环境") {
            Text(localized("检测已安装的解释器，或指定可执行文件路径。")).foregroundStyle(.secondary)
            ForEach([("python_executable", "Python"), ("rscript_executable", "Rscript"), ("node_executable", "Node.js"), ("npm_executable", "npm"), ("uv_executable", "uv"), ("pixi_executable", "Pixi"), ("sci_executable", "sci"), ("cua_driver_executable", "Cua Driver")], id: \.0) { key, label in
                NativeSettingsField(field: .init(key: key, label: label, kind: .file), value: Binding(get: { model.values["get_bootstrap_status"]?["local_environment"]["paths"][key] ?? .null }, set: { new in
                    var status = model.values["get_bootstrap_status"] ?? .object([:]); var env = status["local_environment"]; var paths = env["paths"]; paths[key] = new; env["paths"] = paths; status["local_environment"] = env; model.values["get_bootstrap_status"] = status
                }))
            }
            HStack {
                Button(localized("重新检测")) { Task { if let status = await model.run("detect_local_environment", refresh: false, success: "检测完成") { model.values["get_bootstrap_status"] = status } } }
                Spacer()
                Button(localized("保存路径")) { Task { if let status = await model.run("save_local_environment_paths", ["paths": model.values["get_bootstrap_status"]?["local_environment"]["paths"] ?? .object([:])], refresh: false) { model.values["get_bootstrap_status"] = status } } }.buttonStyle(NativeSettingsButtonStyle(primary: true))
            }
            if let warning = model.values["get_bootstrap_status"]?["local_environment"]["warning"].string, !warning.isEmpty { Text(warning).foregroundStyle(.orange) }
        }.disabled(model.values["get_bootstrap_status"] == nil)
    }
}
