import AppKit
import CoreImage.CIFilterBuiltins
import SwiftUI
import WispProjectBrowser

struct NativeChannelSettings: View {
    @ObservedObject var model: NativeSettingsModel
    @State private var binding: SettingsValue?
    @State private var bindKind = ""
    @State private var token = ""
    @State private var syncResult: SettingsValue?
    @State private var confirmation: SettingsOperation?
    var body: some View {
        VStack(spacing: 22) {
            switch model.detailSection {
            case "飞书机器人": feishu
            case "微信机器人": weixin
            case "StickS3 设备桥接": device
            default:
                NativeSettingsColumns {
                    sync
                } second: {
                    NativeSettingsGroup(title: "消息与设备接入") {
                        Text(localized("连接消息平台与设备，远程查看项目并发起对话。")).font(WispDesign.font(size: 12)).foregroundStyle(.secondary)
                        channelEntry("飞书机器人", detail: "飞书 / Lark", enabled: status["feishu_enabled"].bool)
                        Divider()
                        channelEntry("微信机器人", detail: "iLink", enabled: status["weixin_enabled"].bool)
                        Divider()
                        channelEntry("StickS3 设备桥接", detail: "局域网 / 本机", enabled: status["device"]["enabled"].bool)
                    }
                }
            }
        }
        .confirmationDialog(confirmation?.title ?? "确认操作", isPresented: Binding(get: { confirmation != nil }, set: { if !$0 { confirmation = nil } }), presenting: confirmation) { operation in
            Button(operation.title, role: .destructive) { Task { _ = await model.run(operation.command, operation.args, success: "操作完成"); token = "" } }
            Button(localized("取消"), role: .cancel) {}
        }
        .onDisappear { token = ""; if bindKind == "feishu", let flow = binding?["flow_id"] { Task { _ = try? await model.client.invoke("feishu_bind_cancel", args: ["flowId": flow], projectID: model.projectID) } } }
    }
    private func channelEntry(_ title: String, detail: String, enabled: Bool) -> some View {
        Button { model.detailSection = title } label: {
            HStack(spacing: 12) {
                VStack(alignment: .leading, spacing: 5) { Text(localized(title)).fontWeight(.medium); Text(detail).font(WispDesign.font(size: 12)).foregroundStyle(.secondary) }
                Spacer()
                Text(localized(enabled ? "已启用" : "未启用")).font(WispDesign.font(size: 11)).foregroundStyle(.secondary)
                WispIcon(name: "chevron-right", size: 14)
            }.padding(.vertical, 12).contentShape(Rectangle())
        }.buttonStyle(.plain)
    }
    private var status: SettingsValue { model.values["channels_status"] ?? .null }
    private var feishu: some View {
        NativeSettingsGroup(title: "飞书 / Lark") {
            Text(status["feishu_detail"].string.isEmpty ? status["feishu_state"].string : status["feishu_detail"].string).foregroundStyle(.secondary)
            HStack {
                Button(localized("配置")) {
                    model.editor = SettingsEditor(title: "飞书 / Lark", draft: .object(["enabled": status["feishu_enabled"], "international": status["feishu_international"], "app_id": status["feishu_app_id"], "app_secret": .string("")]), fields: [.init(key: "enabled", label: "启用", kind: .toggle), .init(key: "international", label: "使用 Lark", kind: .toggle), .init(key: "app_id", label: "App ID"), .init(key: "app_secret", label: "App Secret", kind: .secure, hint: "留空保留已保存的密钥。")], command: "set_feishu_channel", parameter: nil, destructiveCommand: status["feishu_bound"].bool ? "feishu_unbind" : nil)
                }
                Button(localized("扫码绑定")) { Task { bindKind = "feishu"; binding = await model.run("feishu_bind_start", ["international": status["feishu_international"]], refresh: false, success: "请扫码并确认绑定") } }
                Button(localized("设置所有者")) { model.editor = SettingsEditor(title: "飞书所有者", draft: .object(["open_id": status["feishu_owner_open_id"]]), fields: [.init(key: "open_id", label: "所有者 Open ID")], command: "set_feishu_owner", parameter: nil) }
            }
            if !status["feishu_pending_owner_open_id"].string.isEmpty {
                Text("待确认所有者：\(status["feishu_pending_owner_open_id"].string)")
                HStack { Button(localized("确认所有者")) { Task { _ = await model.run("confirm_feishu_pending_owner") } }; Button(localized("拒绝")) { Task { _ = await model.run("reject_feishu_pending_owner") } } }
            }
            if bindKind == "feishu" { qr }
        }
    }
    private var weixin: some View {
        NativeSettingsGroup(title: "微信") {
            Text(status["weixin_detail"].string.isEmpty ? status["weixin_state"].string : status["weixin_detail"].string).foregroundStyle(.secondary)
            NativeSettingsSwitch(title: "启用微信通道", isOn: Binding(get: { status["weixin_enabled"].bool }, set: { enabled in Task { _ = await model.run("set_weixin_channel", ["enabled": .bool(enabled)]) } }))
            HStack {
                Button(localized("扫码绑定")) { Task { bindKind = "weixin"; binding = await model.run("weixin_bind_start", refresh: false, success: "请扫码并确认绑定") } }
                Button(localized("解绑…")) { confirmation = .init(title: "解除微信绑定", command: "weixin_unbind") }.disabled(!status["weixin_bound"].bool)
            }
            if bindKind == "weixin" { qr }
        }
    }
    @ViewBuilder private var qr: some View {
        if let binding {
            if let image = qrImage(binding["qr_content"].string) { Image(nsImage: image).interpolation(.none).resizable().frame(width: 220, height: 220).accessibilityLabel("绑定二维码") }
            HStack {
                Button(localized("检查扫码状态")) { Task {
                    if bindKind == "feishu" {
                        let result = await model.run("feishu_bind_poll", ["flowId": binding["flow_id"]], refresh: false, success: "")
                        model.message = result?["state"].string
                        if result?["state"].string == "confirmed" { self.binding = nil; await model.load() }
                    } else {
                        let result = await model.run("weixin_bind_poll", ["qrcode": binding["qrcode"]], refresh: false, success: "")
                        model.message = result?.string
                        if result?.string == "confirmed" { self.binding = nil; await model.load() }
                    }
                } }
                Button(localized("取消绑定")) { Task { if bindKind == "feishu" { _ = await model.run("feishu_bind_cancel", ["flowId": binding["flow_id"]], refresh: false, success: "已取消") }; self.binding = nil } }
            }
        }
    }
    private var device: some View {
        NativeSettingsGroup(title: "设备桥接") {
            let device = status["device"]
            Text(device["detail"].string).foregroundStyle(.secondary)
            Text(device["url"].string).textSelection(.enabled)
            HStack {
                Button(localized("配置")) { model.editor = SettingsEditor(title: "设备桥接", draft: .object(["enabled": device["enabled"], "mode": device["mode"], "bind_ipv4": device["bindIpv4"], "port": device["port"]]), fields: [.init(key: "enabled", label: "启用", kind: .toggle), .init(key: "mode", label: "模式", kind: .choice([("lan", "局域网 / 本机")])), .init(key: "bind_ipv4", label: "监听 IPv4"), .init(key: "port", label: "端口", kind: .integer)], command: "set_device_bridge", parameter: nil) }
                Button(localized("显示访问令牌")) { Task { token = await model.run("get_device_bridge_token", refresh: false, success: "令牌已读取")?.string ?? "" } }
                Button(localized("重置令牌…")) { confirmation = .init(title: "重置设备访问令牌", command: "rotate_device_bridge_token") }
                Button(localized("撤销令牌…")) { confirmation = .init(title: "撤销设备访问令牌", command: "revoke_device_bridge_token") }
            }
            if !token.isEmpty { HStack { SecureField(localized("访问令牌"), text: $token).textFieldStyle(NativeSettingsTextFieldStyle()); Button(localized("复制")) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(token, forType: .string) }; Button(localized("隐藏")) { token = "" } } }
        }
    }
    private var sync: some View {
        NativeSettingsGroup(title: "项目同步") {
            ForEach([SettingsField(key: "sync_backend", label: "同步方式", kind: .choice([("relay", "中继服务"), ("folder", "同步文件夹")])), .init(key: "sync_relay_url", label: "中继 URL"), .init(key: "sync_relay_token", label: "中继令牌", kind: .secure, hint: "留空保留已有令牌。"), .init(key: "sync_folder", label: "同步文件夹", kind: .path)]) { field in
                if field.key == "sync_backend" || (field.key == "sync_folder" ? model.values["get_settings"]?["sync_backend"].string == "folder" : model.values["get_settings"]?["sync_backend"].string != "folder") {
                    NativeSettingsField(field: field, value: model.binding("get_settings", field.key))
                }
            }
            Button(localized("保存同步配置")) { Task { await model.saveSettings() } }
            HStack {
                Button(localized("立即同步")) { Task { if let id = model.projectID { syncResult = await model.run("sync_project", ["id": .string(id)], refresh: false, success: "同步完成") } } }.disabled(model.projectID == nil)
                Button(localized("生成加入代码")) { Task { if let id = model.projectID { syncResult = await model.run("project_sync_code", ["id": .string(id)], refresh: false, success: "加入代码已生成") } } }.disabled(model.projectID == nil)
                Button(localized("加入同步项目")) { model.editor = SettingsEditor(title: "加入同步项目", draft: .object(["code": .string("")]), fields: [.init(key: "code", label: "加入代码", kind: .multiline)], command: "join_synced_project", parameter: nil) }
            }
            if let syncResult { Text(syncResult.string).textSelection(.enabled); NativeSettingsSummary(value: syncResult) }
            if syncResult?["status"].string == "conflict" {
                HStack {
                    Button(localized("保留本地版本…")) { confirmation = .init(title: "使用本地版本解决冲突", command: "resolve_project_sync", args: ["id": .string(model.projectID ?? ""), "strategy": .string("local")]) }
                    Button(localized("采用远端版本…")) { confirmation = .init(title: "使用远端版本解决冲突", command: "resolve_project_sync", args: ["id": .string(model.projectID ?? ""), "strategy": .string("remote")]) }
                }
            }
        }
    }
    private func qrImage(_ content: String) -> NSImage? {
        guard !content.isEmpty else { return nil }
        let filter = CIFilter.qrCodeGenerator(); filter.message = Data(content.utf8)
        guard let image = filter.outputImage, let cg = CIContext().createCGImage(image, from: image.extent) else { return nil }
        return NSImage(cgImage: cg, size: NSSize(width: image.extent.width, height: image.extent.height))
    }
}
