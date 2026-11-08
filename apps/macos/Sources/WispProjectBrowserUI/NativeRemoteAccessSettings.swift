import AppKit
import CoreImage.CIFilterBuiltins
import SwiftUI
import WispProjectBrowser

struct NativeRemoteAccessSettings: View {
    @ObservedObject var model: NativeSettingsModel
    @State private var reset = false
    private var status: SettingsValue { model.values["channels_status"]?["remote"] ?? .null }
    var body: some View {
        NativeSettingsGroup(title: "网页远程访问") {
            Text(localized("通过联机链接或二维码，从另一台设备访问 Wisp。访问令牌保存在系统钥匙串。"))
                .font(.caption).foregroundStyle(.secondary)
            NativeSettingsSwitch(title: "启用网页远程访问", isOn: Binding(get: { status["enabled"].bool }, set: { set("enabled", .bool($0)) }))
            TextField(localized("中继地址"), text: Binding(get: { status["relay_url"].string }, set: { set("relay_url", .string($0)) }))
            SecureField(localized("中继令牌（留空保留）"), text: $model.remoteAccessToken)
            HStack {
                Button(localized("保存")) { Task {
                    _ = await model.run("set_remote_access", ["enabled": status["enabled"], "relayUrl": .string(status["relay_url"].string.trimmingCharacters(in: .whitespacesAndNewlines)), "relayToken": .string(model.remoteAccessToken)])
                } }.disabled(model.busy || status == .null)
                Button(localized("重置连接码…")) { reset = true }.disabled(model.busy || status["code"] == .null)
            }
            Text(status["state"].string + " · " + status["detail"].string).font(.caption).textSelection(.enabled)
            if !status["code"].string.isEmpty { Text(status["code"].string).font(.title3.monospaced()).textSelection(.enabled) }
            if let link = Self.link(status) {
                Link(localized("打开联机链接"), destination: link)
                Button(localized("复制联机链接")) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(link.absoluteString, forType: .string) }
                if let image = Self.qr(link.absoluteString) {
                    Image(nsImage: image).interpolation(.none).resizable().frame(width: 200, height: 200).accessibilityLabel(localized("网页远程访问二维码"))
                }
            }
            Text("\(status["clients"].integer) " + localized("台设备已连接")).font(.caption)
        }.disabled(status == .null)
            .confirmationDialog(localized("重置连接码后，需要使用新的链接重新连接。"), isPresented: $reset) {
                Button(localized("重置连接码"), role: .destructive) { Task { _ = await model.run("reset_remote_access_code") } }
                Button(localized("取消"), role: .cancel) {}
            }
    }
    private func set(_ key: String, _ value: SettingsValue) {
        var document = model.values["channels_status"] ?? .object([:])
        document["remote"][key] = value; model.values["channels_status"] = document
    }
    static func link(_ status: SettingsValue) -> URL? {
        guard let url = URL(string: status["link"].string), ["https", "http"].contains(url.scheme?.lowercased() ?? ""), url.host != nil else { return nil }
        return url
    }
    static func qr(_ content: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator(); filter.message = Data(content.utf8)
        guard let output = filter.outputImage, let image = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return NSImage(cgImage: image, size: NSSize(width: output.extent.width, height: output.extent.height))
    }
}

struct NativeLabSettings: View {
    @ObservedObject var model: NativeSettingsModel
    @State private var inviteCode = ""
    @State private var confirmation: SettingsOperation?
    private var status: SettingsValue { model.values["lab_status"] ?? .null }
    var body: some View {
        NativeSettingsGroup(title: "Wisp Lab 课题组") {
            Text(status["lab_name"].string).font(.headline)
            Text(status["state"].string + " · " + status["detail"].string).font(.caption).textSelection(.enabled)
            if ["none", "removed"].contains(status["state"].string) {
                Button(localized("创建或加入课题组")) {
                    model.editor = SettingsEditor(title: "创建或加入课题组", draft: .object([:]), fields: [
                        .init(key: "relayUrl", label: "中继地址"), .init(key: "relayToken", label: "中继令牌", kind: .secure),
                        .init(key: "name", label: "成员名称"), .init(key: "labName", label: "课题组名称"), .init(key: "invite", label: "邀请码（创建时留空）")
                    ], command: "lab_join", parameter: nil)
                }
            } else if status != .null {
                Text(status["member_name"].string + " · " + status["relay_url"].string).font(.caption)
                HStack {
                    Button(localized("更新中继令牌")) { model.editor = SettingsEditor(title: "更新中继令牌", draft: .object([:]), fields: [.init(key: "relayToken", label: "中继令牌", kind: .secure)], command: "lab_set_relay_token", parameter: nil) }
                    Button(localized("退出课题组…")) { confirmation = .init(title: "退出课题组", command: "lab_leave", args: ["forget": .bool(false)]) }
                    Button(localized("忘记此课题组…")) { confirmation = .init(title: "忘记此课题组", command: "lab_leave", args: ["forget": .bool(true)]) }
                }
                if status["leader"].bool {
                    Button(localized("生成邀请码")) { Task { if let value = await model.run("lab_create_invite", refresh: false, success: "") { inviteCode = value.string } } }
                    if !inviteCode.isEmpty { Text(inviteCode).font(.system(.body, design: .monospaced)).textSelection(.enabled) }
                }
                ForEach(Array(status["members"].array.enumerated()), id: \.offset) { _, member in
                    HStack {
                        Text(member["name"].string)
                        Text(localized(member["pending"].bool ? "待批准" : member["leader"].bool ? "组长" : "成员")).font(.caption).foregroundStyle(.secondary)
                        Spacer()
                        if status["leader"].bool && !member["leader"].bool {
                            if member["pending"].bool { Button(localized("批准")) { Task { _ = await model.run("lab_approve_member", ["memberId": member["id"]]) } } }
                            Button(localized("移除…")) { confirmation = .init(title: "移除成员", command: "lab_remove_member", args: ["memberId": member["id"]]) }
                        }
                    }
                }
                Divider()
                Text(localized("课题组知识库与接收项目")).font(.headline)
                if status["knowledge_base"] != .null {
                    Text(status["knowledge_base"]["project_name"].string)
                    Button(localized("加入知识库")) { Task { _ = await model.run("lab_join_knowledge_base") } }
                }
                if status["leader"].bool {
                    Button(localized("发布当前设置中选定的项目为知识库…")) {
                        if let project = model.projectID { confirmation = .init(title: "发布知识库", command: "lab_publish_knowledge_base", args: ["projectId": .string(project)]) }
                    }.disabled(model.projectID == nil)
                }
                Text(localized("接收项目") + "：" + status["inbox_project_id"].string).font(.caption)
                HStack {
                    Button(localized("使用当前选定项目接收")) { Task { if let id = model.projectID { _ = await model.run("lab_set_inbox_project", ["projectId": .string(id)]) } } }.disabled(model.projectID == nil)
                    Button(localized("不接收课题组提问")) { Task { _ = await model.run("lab_set_inbox_project", ["projectId": .string("")]) } }
                }
            }
        }.disabled(model.busy || status == .null)
            .confirmationDialog(confirmation?.title ?? "", isPresented: Binding(get: { confirmation != nil }, set: { if !$0 { confirmation = nil } })) {
                if let action = confirmation {
                    Button(localized(action.title), role: .destructive) { confirmation = nil; Task { _ = await model.run(action.command, action.args); inviteCode = "" } }
                }
                Button(localized("取消"), role: .cancel) { confirmation = nil }
            }
    }
}
