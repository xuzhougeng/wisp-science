import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeSettingsField: View {
    let field: SettingsField
    @Binding var value: SettingsValue
    private var text: Binding<String> { Binding(get: { value.string }, set: { value = .string($0) }) }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if case .toggle = field.kind {
                HStack(alignment: .center, spacing: 20) {
                    VStack(alignment: .leading, spacing: 5) {
                        Text(localized(field.label)).fontWeight(.medium)
                        if !field.hint.isEmpty { Text(localized(field.hint)).font(WispDesign.font(size: 12)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
                    }
                    Spacer(); input.fixedSize()
                }.padding(.vertical, 6)
            } else {
                Text(localized(field.label)).font(WispDesign.font(size: 12, weight: .medium)).foregroundStyle(.secondary)
                input.frame(maxWidth: .infinity, alignment: .leading)
                if !field.hint.isEmpty { Text(localized(field.hint)).font(WispDesign.font(size: 12)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
            }
        }.accessibilityElement(children: .contain)
    }
    @ViewBuilder private var input: some View {
        switch field.kind {
        case .text: TextField(localized(field.label), text: text).textFieldStyle(NativeSettingsTextFieldStyle()).labelsHidden()
        case .secure: SecureField(localized(field.label), text: text).textFieldStyle(NativeSettingsTextFieldStyle()).labelsHidden()
        case .json: NativeSettingsJSON(value: $value, label: localized(field.label))
        case .multiline:
            TextEditor(text: text).font(WispDesign.font(size: 13, design: .monospaced)).frame(minHeight: 120)
                .overlay(RoundedRectangle(cornerRadius: 5).stroke(Color.secondary.opacity(0.25)))
                .accessibilityLabel(localized(field.label))
        case .lines:
            TextEditor(text: Binding(get: { value.array.map(\.string).joined(separator: "\n") }, set: { value = .array($0.components(separatedBy: "\n").filter { !$0.isEmpty }.map(SettingsValue.string)) }))
                .font(WispDesign.font(size: 13, design: .monospaced)).frame(minHeight: 90)
                .overlay(RoundedRectangle(cornerRadius: 5).stroke(Color.secondary.opacity(0.25)))
                .accessibilityLabel(localized(field.label))
        case .integer:
            TextField(localized(field.label), text: Binding(get: { value.string }, set: { value = ($0.isEmpty ? .null : (Int64($0).map(SettingsValue.integer) ?? .string($0))) }))
                .textFieldStyle(NativeSettingsTextFieldStyle()).frame(maxWidth: 160).labelsHidden()
        case .toggle:
            Toggle(localized(field.label), isOn: Binding(get: { value.bool }, set: { value = .bool($0) })).toggleStyle(.switch).labelsHidden().accessibilityLabel(localized(field.label))
        case .choice(let choices):
            NativeSettingsChoice(label: localized(field.label), selection: text, choices: choices).frame(maxWidth: 330)
        case .object(let fields):
            VStack(spacing: 14) {
                ForEach(fields) { child in
                    AnyView(NativeSettingsField(field: child, value: Binding(get: { value[child.key] }, set: { value[child.key] = $0 })))
                }
            }
        case .records(let fields):
            NativeSettingsRecords(value: $value, fields: fields)
        case .path, .file, .directory:
            HStack {
                TextField(localized(field.label), text: text).textFieldStyle(NativeSettingsTextFieldStyle()).labelsHidden()
                Button(localized("选择…")) {
                    let panel = NSOpenPanel(); panel.canChooseFiles = true; panel.canChooseDirectories = true
                    if case .directory = field.kind { panel.canChooseFiles = false }
                    panel.allowsMultipleSelection = false
                    if panel.runModal() == .OK, let url = panel.url { value = .string(url.path) }
                }
            }
        }
    }
}

struct NativeSettingsGroup<Content: View>: View {
    let title: String
    @ViewBuilder let content: () -> Content
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            if !title.isEmpty { Text(localized(title)).font(WispDesign.font(size: 15, weight: .semibold)) }
            content()
        }.padding(24).frame(maxWidth: .infinity, alignment: .leading)
            .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 16))
            .overlay(RoundedRectangle(cornerRadius: 16).stroke(WispDesign.color("border", scheme)))
    }
}

struct NativeSettingsEditorView: View {
    @ObservedObject var model: NativeSettingsModel
    @State var editor: SettingsEditor
    @State private var confirmDelete = false
    @Environment(\.colorScheme) private var scheme
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            VStack(spacing: 22) {
                if editor.command == "save_model" && !NativeModelSettings.isSubscription(editor.draft) {
                    editorFields(["api_url", "key"])
                    Divider().padding(.vertical, 4)
                    Text(localized("模型配置")).font(WispDesign.font(size: 15, weight: .semibold)).frame(maxWidth: .infinity, alignment: .leading)
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .top, spacing: 16) {
                            editorFields(["provider"]).frame(minWidth: 200)
                            editorFields(["model"]).frame(minWidth: 180)
                            editorFields(["label"]).frame(minWidth: 160)
                        }
                        editorFields(["provider", "model", "label"])
                    }
                    editorFields(["endpoint_suffix"])
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .top, spacing: 36) {
                            editorFields(["supports_vision", "use_for_vision"]).frame(minWidth: 230)
                            editorFields(["use_for_image_generation", "use_for_video_generation"]).frame(minWidth: 230)
                        }
                        editorFields(["supports_vision", "use_for_vision", "use_for_image_generation", "use_for_video_generation"])
                    }
                    DisclosureGroup(localized("容量与生成参数")) { editorFields(["context_window", "max_tokens", "reasoning_effort", "service_tier", "image_size", "image_quality", "image_aspect_ratio", "image_resolution", "video_duration_secs", "video_aspect_ratio", "video_resolution"]).padding(.top, 16) }
                    DisclosureGroup(localized("请求头设置")) { editorFields(["send_user_agent", "user_agent", "send_session_id", "session_header_name"]).padding(.top, 16) }
                } else {
                    editorFields(editor.fields.map(\.key))
                }
            }.disabled(editor.readOnly)
            if editor.command == "save_model" && !NativeModelSettings.isSubscription(editor.draft) { HStack {
                Button(localized("查询模型目录")) { Task {
                    if let result = await model.run("model_catalog_lookup", ["provider": editor.draft["provider"], "apiUrl": editor.draft["api_url"], "model": editor.draft["model"]], refresh: false, success: "模型目录已查询") {
                        model.message = result == .null ? "目录没有此精确模型 ID，请按服务商说明填写容量。" : "目录上限：" + result.object.keys.sorted().map { $0 + " " + result[$0].string }.joined(separator: " · ")
                    }
                } }
                Button(localized("测试 API")) { Task {
                    guard var settings = try? await model.client.invoke("get_settings", args: [:], projectID: model.projectID) else { model.error = "无法读取当前设置"; return }
                    for (key, value) in editor.draft.object where settings.object[key] != nil { settings[key] = value }
                    if let result = await model.run("validate_settings", ["settings": settings, "key": editor.draft["key"], "profileId": editor.draft["id"], "useForImageGeneration": editor.draft["use_for_image_generation"]], refresh: false, success: "API 测试成功") { model.message = result.string }
                } }
            }.disabled(model.busy) }
            if let message = model.message, !message.isEmpty { Text(message).font(.caption).textSelection(.enabled) }
            if let error = model.error { Text(error).foregroundStyle(.red).font(WispDesign.font(size: 12)).textSelection(.enabled) }
            HStack {
                if editor.destructiveCommand != nil { Button(localized("删除…"), role: .destructive) { confirmDelete = true } }
                Spacer()
                if model.busy { ProgressView().controlSize(.small) }
                Button(model.busy ? "取消授权" : "取消") {
                    if model.busy { Task { _ = try? await model.client.invoke("cancel_oauth_authorization", args: [:], projectID: model.projectID) } }
                    else { model.editor = nil }
                }.keyboardShortcut(.cancelAction).disabled(model.busy && editor.draft["transport"]["auth"].string != "oauth")
                if !editor.readOnly { Button(localized("保存")) { Task { await save() } }.buttonStyle(NativeSettingsButtonStyle(primary: true)).keyboardShortcut(.defaultAction).disabled(model.busy) }
            }
        }
        .padding(24).frame(maxWidth: .infinity, alignment: .leading)
        .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).stroke(WispDesign.color("border", scheme)))
        .confirmationDialog("确认删除？此操作会移除当前配置。", isPresented: $confirmDelete) {
            Button(localized("删除"), role: .destructive) { Task {
                if let command = editor.destructiveCommand,
                   await model.run(command, editor.destructiveArgs, success: "已删除") != nil { model.editor = nil }
            } }
            Button(localized("取消"), role: .cancel) {}
        }
        .background(NativeSettingsEscape(enabled: !confirmDelete) { model.editor = nil })
    }
    private func editorFields(_ keys: [String]) -> some View {
        VStack(spacing: 18) {
            ForEach(keys, id: \.self) { key in
                if let field = editor.fields.first(where: { $0.key == key }) {
                    NativeSettingsField(field: field, value: Binding(get: { editor.draft[field.key] }, set: { editor.draft[field.key] = $0 }))
                }
            }
        }
    }
    private func save() async {
        var draft = editor.draft.object
        if editor.parameter == nil {
            for field in editor.fields where draft[field.key] == nil || draft[field.key] == .null {
                if let value = field.initial { draft[field.key] = value }
            }
        }
        if editor.command == "save_workflow_template" {
            // Empty optional bindings must stay absent; an empty Specialist ID
            // would be interpreted as a dangling reference by the compiler.
            var proposal = editor.draft["proposal"]
            proposal["tasks"] = .array(proposal["tasks"].array.map { task in
                var task = task
                for key in ["specialist_id", "model_id"] where task[key].string.isEmpty { task[key] = .null }
                return task
            })
            draft["proposal"] = proposal
        }
        var args = editor.extra
        if editor.command == "install_plugin" && draft["expected_sha256"]?.string.isEmpty != false { draft["expected_sha256"] = .null }
        if editor.command == "save_model" {
            args["key"] = draft.removeValue(forKey: "key") ?? .null
            for key in ["use_for_vision", "use_for_image_generation", "use_for_video_generation"] {
                args[camelCase(key)] = draft[key] ?? .bool(false)
            }
        }
        if let parameter = editor.parameter { args[parameter] = .object(draft) }
        else { for (key, value) in draft { args[camelCase(key)] = value } }
        let command = (editor.command == "add_mcp_connection" || editor.command == "update_mcp_connection") && editor.draft["transport"]["auth"].string == "oauth" ? "authorize_http_connection" : editor.command
        if await model.run(command, args) != nil { model.editor = nil }
    }
}

func camelCase(_ text: String) -> String {
    let words = text.split(separator: "_"); return (words.first.map(String.init) ?? "") + words.dropFirst().map { $0.prefix(1).uppercased() + $0.dropFirst() }.joined()
}

/// Window-level Escape handling, including immediate dismissal before focus
/// enters a control. Native menus, file dialogs and child sheets remain on top.
struct NativeSettingsEscape: NSViewRepresentable {
    var enabled = true
    let close: () -> Void
    func makeCoordinator() -> Coordinator { Coordinator(enabled: enabled, close: close) }
    func makeNSView(context: Context) -> NSView {
        let view = NSView(); context.coordinator.view = view; context.coordinator.install(); return view
    }
    func updateNSView(_ view: NSView, context: Context) { context.coordinator.close = close; context.coordinator.enabled = enabled }
    static func dismantleNSView(_ view: NSView, coordinator: Coordinator) { coordinator.remove() }
    final class Coordinator {
        weak var view: NSView?
        var close: () -> Void
        var enabled: Bool
        init(enabled: Bool, close: @escaping () -> Void) { self.enabled = enabled; self.close = close }
        func install() { NativeEscapeStack.shared.register(self) }
        func remove() { NativeEscapeStack.shared.remove(self) }
        deinit { remove() }
    }
}

struct NativeSettingsRecords: View {
    @Binding var value: SettingsValue
    let fields: [SettingsField]
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            ForEach(Array(value.array.indices), id: \.self) { index in
                VStack(alignment: .leading, spacing: 12) {
                    HStack { Text("\(index + 1)").fontWeight(.semibold); Spacer(); Button(localized("移除")) { var rows = value.array; rows.remove(at: index); value = .array(rows) } }
                    ForEach(fields) { field in
                        AnyView(NativeSettingsField(field: field, value: Binding(get: { value.array.indices.contains(index) ? value.array[index][field.key] : .null }, set: { new in
                            var rows = value.array; guard rows.indices.contains(index) else { return }; rows[index][field.key] = new; value = .array(rows)
                        })))
                    }
                }.padding(12).background(Color.secondary.opacity(0.05), in: RoundedRectangle(cornerRadius: 8))
            }
            Button(localized("添加一项")) { value = .array(value.array + [.object(Dictionary(uniqueKeysWithValues: fields.compactMap { field in field.initial.map { (field.key, $0) } }))]) }
        }
    }
}

struct NativeSettingsJSON: View {
    @Binding var value: SettingsValue
    let label: String
    @State private var source = ""
    @State private var invalid = false
    var body: some View {
        VStack(alignment: .leading) {
            TextEditor(text: $source).font(WispDesign.font(size: 12, design: .monospaced)).frame(minHeight: 140).accessibilityLabel(label)
            if invalid { Text(localized("请输入有效的 JSON；清空表示不设置。")).foregroundStyle(.red).font(.caption) }
        }.onAppear {
            let encoder = JSONEncoder(); encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
            source = value == .null ? "" : (try? String(data: encoder.encode(value), encoding: .utf8)) ?? ""
        }.onChange(of: source) { text in
            if text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { value = .null; invalid = false }
            else if let decoded = try? JSONDecoder().decode(SettingsValue.self, from: Data(text.utf8)) { value = decoded; invalid = false }
            else { invalid = true; value = .string(text) }
        }
    }
}

struct NativeSettingsTextFieldStyle: TextFieldStyle {
    @Environment(\.colorScheme) private var scheme
    func _body(configuration: TextField<Self._Label>) -> some View {
        configuration.textFieldStyle(.plain).font(WispDesign.font(size: 14))
            .padding(.horizontal, 11).padding(.vertical, 9)
            .background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(WispDesign.color("border-strong", scheme)))
    }
}

struct NativeSettingsButtonStyle: ButtonStyle {
    var primary = false
    var compact = false
    @Environment(\.colorScheme) private var scheme
    @Environment(\.isEnabled) private var enabled
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.font(WispDesign.font(size: 13.5))
            .foregroundStyle(primary ? Color.white : WispDesign.color("text", scheme))
            .padding(.horizontal, compact ? 11 : 16).padding(.vertical, compact ? 5 : 9)
            .background(WispDesign.color(primary ? "clay" : "bg-elev", scheme), in: RoundedRectangle(cornerRadius: compact ? 20 : 10))
            .overlay(RoundedRectangle(cornerRadius: compact ? 20 : 10).stroke(primary ? .clear : WispDesign.color("border-strong", scheme)))
            .opacity(enabled ? (configuration.isPressed ? 0.7 : 1) : 0.45)
    }
}

struct NativeSettingsChoice: View {
    let label: String
    @Binding var selection: String
    let choices: [(String, String)]
    static func selectedTitle(_ selection: String, choices: [(String, String)]) -> String {
        choices.first(where: { $0.0 == selection })?.1 ?? selection
    }
    var body: some View {
        Menu {
            ForEach(choices, id: \.0) { key, title in Button(localized(title)) { selection = key } }
        } label: {
            HStack {
                Text(localized(Self.selectedTitle(selection, choices: choices))).lineLimit(1)
                Spacer(minLength: 8)
                WispIcon(name: "chevron-down", size: 13).foregroundStyle(.secondary)
            }
        }.menuStyle(.borderlessButton).menuIndicator(.hidden)
            .padding(.horizontal, 11).padding(.vertical, 10)
            .background(Color.primary.opacity(0.025), in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(Color.secondary.opacity(0.28)))
            .accessibilityLabel(label)
    }
}

struct NativeSettingsColumns<First: View, Second: View>: View {
    @ViewBuilder let first: () -> First
    @ViewBuilder let second: () -> Second
    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .top, spacing: 20) { first().frame(minWidth: 380, maxWidth: .infinity); second().frame(minWidth: 340, maxWidth: .infinity) }
            VStack(spacing: 20) { first(); second() }
        }
    }
}

struct NativePreferenceRow<Control: View>: View {
    let title: String
    var hint = ""
    @ViewBuilder let control: () -> Control
    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .center, spacing: 24) {
                explanation.frame(minWidth: 220, maxWidth: .infinity, alignment: .leading)
                control().fixedSize(horizontal: true, vertical: false)
            }
            VStack(alignment: .leading, spacing: 12) {
                explanation
                control().frame(maxWidth: .infinity, alignment: .leading)
            }
        }.padding(.vertical, 9)
    }
    private var explanation: some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(localized(title)).font(WispDesign.font(size: 14, weight: .medium))
            if !hint.isEmpty { Text(localized(hint)).font(WispDesign.font(size: 12)).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true) }
        }
    }
}
