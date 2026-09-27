import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeCodexLoginSheet: View {
    @StateObject var model: NativeCodexLoginModel
    let saved: () -> Void
    let close: (String?) -> Void
    @State private var browserError = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(model.provider == "xai" ? "SuperGrok" : "ChatGPT").font(.headline)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if model.accountOnly {
                    Text(localized(model.provider == "xai" ? "通过 SuperGrok / X Premium+ 订阅登录。登录凭据保存在系统钥匙串中。" : "通过 ChatGPT 订阅登录。登录凭据保存在系统钥匙串中。"))
                        .font(.caption).foregroundStyle(.secondary)
                    Text(localized("登录账号")).font(.headline)
                    if model.provider != "xai" { Picker(localized("登录方式"), selection: $model.method) {
                        Text(localized("浏览器登录")).tag("browser")
                        Text(localized("设备码登录")).tag("device")
                    }.disabled(model.busy || model.phase == .pending) }
                    Text(localized(model.provider == "xai" ? "打开 xAI 验证页面并批准登录，需要时输入一次性代码。" : model.method == "device" ? "在 ChatGPT 安全设置或工作区权限中启用设备码登录，再打开验证页输入一次性代码。" : "会打开浏览器登录 ChatGPT。如果浏览器无法回到这台电脑，把最终跳转地址粘贴回来。")).font(.caption).foregroundStyle(.secondary)
                    if model.checkingSaved { ProgressView().controlSize(.small) }
                    if let account = model.savedAccount, account.signed_in {
                        Text(localized("本机已保存的账户：") + account.account_id).font(.caption).textSelection(.enabled)
                    }
                    Button(localized(model.phase == .failed || model.phase == .success ? "重新登录" : "开始登录")) {
                        Task {
                            browserError = false
                            if let url = await model.start() { browserError = !NSWorkspace.shared.open(url) }
                        }
                    }.disabled(model.busy || model.phase == .pending)
                    if let challenge = model.challenge, model.phase == .pending {
                        if challenge.method == "device" {
                            Text(localized("在验证页面输入以下设备码：")).font(.caption)
                            HStack {
                                Text(challenge.user_code).font(.system(.title2, design: .monospaced)).textSelection(.enabled)
                                Button(localized("复制设备码")) {
                                    NSPasteboard.general.clearContents()
                                    NSPasteboard.general.setString(challenge.user_code, forType: .string)
                                }
                            }
                        }
                        if let url = NativeCodexLoginModel.authorizationURL(challenge, provider: model.provider) {
                            Button(localized("打开登录页面")) { browserError = !NSWorkspace.shared.open(url) }
                            Text(url.absoluteString).font(.caption).foregroundStyle(.secondary).lineLimit(3).textSelection(.enabled)
                        }
                        if challenge.method == "browser" {
                            TextField(localized("粘贴浏览器回调地址或授权码"), text: $model.redirect)
                            Button(localized("提交回调地址")) { Task { await model.submitRedirect() } }
                                .disabled(model.busy || model.redirect.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        }
                    }
                    if model.phase == .success {
                        Text(localized("登录已完成，保存后即可用于对话。")).foregroundStyle(.green)
                        Text(model.accountID).font(.caption).textSelection(.enabled)
                    }
                    if !model.message.isEmpty { Text(model.message).font(.caption).textSelection(.enabled) }
                    if browserError { Text(localized("无法打开默认浏览器，请复制上方链接到浏览器。")).font(.caption).foregroundStyle(.orange) }
                    if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                    }
                    if !model.accountOnly {
                        if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange) }
                        Text(localized("添加模型")).font(.headline)
                        Text(localized("填写模型 ID，可自定义别名。连接和模型参数由 Wisp 管理，同一账号可添加多个模型。")).font(.caption).foregroundStyle(.secondary)
                        TextField(localized("模型 ID（留空使用默认模型）"), text: $model.modelID)
                        TextField(localized("显示名称"), text: $model.label)
                    }
                }.textFieldStyle(.roundedBorder).disabled(model.phase == .saving)
            }.frame(maxHeight: 480)
            HStack {
                if model.busy || model.phase == .pending { ProgressView().controlSize(.small) }
                Spacer()
                Button(localized("取消"), action: requestClose).disabled(!model.canClose)
                Button(localized(model.accountOnly ? "保存账号" : "保存模型")) { save(useSaved: !model.accountOnly) }.disabled(model.accountOnly ? !model.canSave : !model.canUseSaved)
            }
        }.padding(24).frame(width: 540)
            .task { await model.loadSavedAccount() }
            .interactiveDismissDisabled(true)
            // Always consume Escape here, including while a save cannot be dismissed.
            .background(NativeSettingsEscape(close: requestClose))
            .onDisappear { Task { _ = await model.cancel() } }
    }

    private func requestClose() {
        guard model.canClose, model.phase != .closed else { return }
        Task { close(await model.cancel()) }
    }
    private func save(useSaved: Bool) {
        Task {
            if await model.save(useSaved: useSaved) != nil { saved(); close(nil) }
        }
    }
}
