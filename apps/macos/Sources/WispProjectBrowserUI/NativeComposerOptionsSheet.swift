import SwiftUI
import WispProjectBrowser

struct NativeComposerOptionsSheet: View {
    @ObservedObject var conversation: NativeConversationModel
    @StateObject private var model: NativeComposerOptionsModel
    let close: () -> Void
    private let loadOnAppear: Bool
    @State private var confirmPermission = false
    init(conversation: NativeConversationModel, model: NativeComposerOptionsModel, close: @escaping () -> Void) {
        self.conversation = conversation; self.close = close; loadOnAppear = false; _model = StateObject(wrappedValue: model)
    }
    init(conversation: NativeConversationModel, project: String, session: String, close: @escaping () -> Void) {
        self.conversation = conversation; self.close = close; loadOnAppear = true
        _model = StateObject(wrappedValue: NativeComposerOptionsModel(client: conversation.client, project: project, session: session, writable: {
            conversation.canEditComposerOptions && conversation.snapshot?.project_id == project && conversation.snapshot?.session_id == session
        }))
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack { Text(localized("会话选项")).font(.headline); Spacer(); Button(localized("关闭"), action: close).disabled(model.saving) }
            if model.busy { ProgressView().controlSize(.small) }
            if let error = model.error {
                Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                Button(localized("重新读取")) { Task { await model.load() } }.disabled(model.busy)
            }
            if let options = model.options {
                VStack(alignment: .leading, spacing: 16) {
                    Toggle(localized("完全权限"), isOn: Binding(get: { options.full_permission }, set: { enabled in
                        if enabled { confirmPermission = true }
                        else { Task { await model.setToggle("full_permission", enabled: false) } }
                    }))
                    Toggle(localized("子代理委派"), isOn: Binding(get: { options.delegation }, set: { enabled in Task { await model.setToggle("delegation", enabled: enabled) } }))
                    Picker(localized("结果返回方式"), selection: Binding(get: { options.completion.policy }, set: { policy in
                        Task { await model.setCompletion(policy, autoResume: options.completion.auto_resume) }
                    })) {
                        Text(localized("当前轮内返回")).tag("inline")
                        Text(localized("后台返回")).tag("background")
                    }.disabled(!options.delegation)
                    if options.completion.policy == "background" {
                        Toggle(localized("自动续接"), isOn: Binding(get: { options.completion.auto_resume }, set: { enabled in Task { await model.setCompletion("background", autoResume: enabled) } }))
                            .disabled(!options.delegation)
                    }
                    Toggle(localized("自动审查"), isOn: Binding(get: { options.auto_review }, set: { enabled in Task { await model.setToggle("auto_review", enabled: enabled) } }))
                    Picker(localized("专家"), selection: Binding(get: { options.specialist?["id"].string ?? "" }, set: { id in Task { await model.setSpecialist(id) } })) {
                        Text(localized("无")).tag("")
                        ForEach(model.specialists) { specialist in Text(specialist.name).tag(specialist.id) }
                        if let id = options.specialist?["id"].string, !id.isEmpty, !model.specialists.contains(where: { $0.id == id }) {
                            Text(options.specialist?["name"].string ?? id).tag(id)
                        }
                    }.disabled(options.specialist_locked || conversation.snapshot?.running != false)
                    if options.specialist_locked { Text(localized("会话已开始，专家选择已锁定。")).font(.caption).foregroundStyle(.secondary) }
                }.toggleStyle(.switch).disabled(!model.canEdit)
            }
            Spacer(minLength: 0)
        }.padding(24).frame(width: 460, height: 430)
            .interactiveDismissDisabled(model.saving)
            .background(NativeSettingsEscape(enabled: !model.saving && !confirmPermission, close: close))
            .sheet(isPresented: $confirmPermission) {
                NativeFullPermissionConfirmation(busy: model.saving, canConfirm: model.canEdit, cancel: { confirmPermission = false }) {
                    Task { await model.setToggle("full_permission", enabled: true, confirmed: true); confirmPermission = false }
                }
            }
            .task { if loadOnAppear { await model.load() } }.onDisappear { model.close() }
    }
}

struct NativeFullPermissionConfirmation: View {
    let busy: Bool
    let canConfirm: Bool
    let cancel: () -> Void
    let confirm: () -> Void
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(localized("开启完全权限")).font(.headline)
            Text(localized("开启后，当前会话中的工具调用、危险命令和 ACP 权限请求会自动批准，直到手动关闭或重启应用。项目路径限制和明确禁用的工具规则仍然有效。当前等待中的操作也会获准执行。"))
                .fixedSize(horizontal: false, vertical: true)
            HStack { Button(localized("取消"), action: cancel).disabled(busy); Spacer(); Button(localized("开启完全权限"), action: confirm).disabled(busy || !canConfirm).buttonStyle(WispButtonStyle(primary: true)) }
        }.padding(24).frame(width: 430)
            .interactiveDismissDisabled(busy).background(NativeSettingsEscape(enabled: !busy, close: cancel))
    }
}
