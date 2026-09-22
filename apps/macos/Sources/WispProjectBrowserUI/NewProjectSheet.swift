import AppKit
import SwiftUI
import WispProjectBrowser

public struct NewProjectDraft: Equatable {
    public var name = ""
    public var directory = ""
    public var description = ""
    public var agentContext = ""
    public var standardLayout = false
    public init() {}
}

enum NewProjectLayout {
    static let chinese = """
本项目采用标准科研目录结构。产物请写入以下路径（相对项目根目录）：
- 图表 -> figures/
- 表格 -> results/tables/
- 拟合模型 -> results/models/
- 报告 -> results/reports/
- 脚本与 notebook -> analysis/scripts/、analysis/notebooks/
- 数据 -> data/raw/（原始输入，不修改）、data/processed/（衍生数据）
- 从远程主机拉取的文件 -> remote/<服务器>/，每台服务器一个子目录，用该执行上下文的名称命名
- 文献与 PDF -> literature/
不要把生成的文件留在项目根目录。
"""
    static let english = """
This project uses the standard research layout. Write outputs to these paths, relative to the project root:
- figures -> figures/
- tables -> results/tables/
- fitted models -> results/models/
- reports -> results/reports/
- scripts and notebooks -> analysis/scripts/, analysis/notebooks/
- data -> data/raw/ (untouched inputs), data/processed/ (derived)
- files pulled off a compute host -> remote/<server>/, one folder per server, named after that execution context's label
- papers and PDFs -> literature/
Do not leave generated files in the project root.
"""

    static func currentBlock() -> String {
        UserDefaults.standard.string(forKey: "nativeSettings.locale") == "en" ? english : chinese
    }

    static func apply(_ context: String, enabled: Bool, block: String) -> String {
        let rest = context.replacingOccurrences(of: block, with: "").trimmingCharacters(in: .whitespacesAndNewlines)
        if !enabled { return rest }
        return rest.isEmpty ? block : rest + "\n\n" + block
    }
}

enum NewProjectError {
    static func message(for text: String) -> String {
        if text.contains("Project name is required") { return "请填写项目名称。" }
        if text.contains("working directory is required") { return "请填写工作目录。" }
        if text.contains("already registered") { return "这个文件夹已经登记为项目。" }
        if text.contains("not writable") { return "工作目录不可写。\n" + text }
        if text.contains("Failed to create working directory") { return "无法创建工作目录。\n" + text }
        return "创建结果尚未确认。请刷新项目列表核对；不会自动重试。\n" + text
    }

    static func importMessage(for text: String) -> String {
        if text.contains("already present") { return "这个项目已经在这台设备上。" }
        if text.contains("not a valid project archive") || text.contains("no manifest") {
            return "这不是有效的项目归档。\n" + text
        }
        return "导入结果尚未确认。请刷新项目列表核对；不会自动重试。\n" + text
    }
}

struct NewProjectSheet: View {
    @ObservedObject var model: ProjectBrowserModel
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("新建项目").font(WispDesign.font(size: 18, weight: .semibold))
            field("名称", text: name)
            VStack(alignment: .leading, spacing: 6) {
                Text("工作目录").font(WispDesign.font(size: 12, weight: .medium))
                HStack(spacing: 8) {
                    TextField("项目目录", text: directory).textFieldStyle(.roundedBorder)
                    Button("选择文件夹") { chooseDirectory() }
                        .disabled(model.createBusy)
                }
            }
            field("说明", text: details)
            VStack(alignment: .leading, spacing: 6) {
                Text("Agent Context").font(WispDesign.font(size: 12, weight: .medium))
                TextEditor(text: agentContext).font(WispDesign.font(size: 13)).frame(minHeight: 120)
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(WispDesign.color("border", scheme)))
                    .disabled(model.createBusy)
            }
            Toggle("标准布局", isOn: layout)
                .disabled(model.createBusy)
            Text("使用标准科研目录结构。关闭后不预建 data/、figures/ 等目录。")
                .font(WispDesign.font(size: 12)).foregroundStyle(WispDesign.color("text-faint", scheme))
            if let error = model.createError {
                Text(error).font(WispDesign.font(size: 12)).foregroundStyle(.red).textSelection(.enabled)
            }
            HStack {
                Spacer()
                Button("取消") { model.dismissNewProject() }.disabled(model.createBusy)
                Button(model.createBusy ? "正在创建…" : "创建") { Task { await model.submitNewProject() } }
                    .buttonStyle(WispButtonStyle(primary: true))
                    .disabled(model.createBusy)
                    .accessibilityIdentifier("create-project-submit")
            }
        }
        .padding(24).frame(width: 560)
        .background(WispDesign.color("bg-app", scheme))
        .interactiveDismissDisabled(model.createBusy)
        .background(NativeSettingsEscape(enabled: !model.createBusy) { model.dismissNewProject() })
    }

    private var name: Binding<String> { binding(\.name) }
    private var directory: Binding<String> { binding(\.directory) }
    private var details: Binding<String> { binding(\.description) }
    private var agentContext: Binding<String> { binding(\.agentContext) }
    private var layout: Binding<Bool> {
        Binding(get: { model.createDraft.standardLayout }, set: { model.setCreateStandardLayout($0) })
    }

    private func binding(_ keyPath: WritableKeyPath<NewProjectDraft, String>) -> Binding<String> {
        Binding(get: { model.createDraft[keyPath: keyPath] }, set: { value in
            var draft = model.createDraft
            draft[keyPath: keyPath] = value
            model.createDraft = draft
        })
    }

    private func field(_ title: String, text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(WispDesign.font(size: 12, weight: .medium))
            TextField(title, text: text).textFieldStyle(.roundedBorder).disabled(model.createBusy)
        }
    }

    private func chooseDirectory() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "选择"
        panel.message = "选择或创建一个工作目录。宿主会检查它是否可写。"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        var draft = model.createDraft
        draft.directory = url.path
        model.createDraft = draft
    }
}
