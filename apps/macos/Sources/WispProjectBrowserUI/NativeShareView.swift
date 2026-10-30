import AppKit
import SwiftUI
import UniformTypeIdentifiers
import WispProjectBrowser

struct NativeShareView: View {
    @Environment(\.colorScheme) private var scheme
    @StateObject private var model: NativeShareModel
    let close: () -> Void
    @State private var format = "PNG"
    @State private var width = "840"
    @State private var exporting = false
    @State private var preview = false
    @State private var error: String?
    init(client: any NativeConversationQuerying, projectID: String, sessionID: String, close: @escaping () -> Void) {
        _model = StateObject(wrappedValue: NativeShareModel(client: client, projectID: projectID, sessionID: sessionID)); self.close = close
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack { Text("分享对话").font(.title2.bold()); Spacer(); Button("关闭", action: close).disabled(exporting) }
            Text("选择要导出的消息。思考内容默认不选中；编辑和脱敏只影响导出副本。").font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("全选") { model.selectAll(true) }; Button("全不选") { model.selectAll(false) }
                Text("已选 \(model.selected.count)/\(model.rows.count)").font(.caption)
                Spacer(); Toggle("预览", isOn: $preview).toggleStyle(.switch)
            }
            if model.loading { ProgressView() }
            if let error = error ?? model.error { Text(error).foregroundStyle(.orange).textSelection(.enabled) }
            ScrollView {
                if preview { NativeSharePage(rows: model.selected, scheme: scheme).frame(maxWidth: .infinity) }
                else {
                    LazyVStack(alignment: .leading, spacing: 12) {
                        ForEach($model.rows) { $entry in
                            VStack(alignment: .leading, spacing: 6) {
                                Toggle(NativeSharePage.label(entry.row.role), isOn: $entry.selected)
                                TextEditor(text: $entry.row.text).frame(minHeight: 80)
                                if !model.keywords.isEmpty { Text(NativeShare.redact(entry.row.text, keywords: NativeShare.keywords(model.keywords))).font(.caption).foregroundStyle(.secondary).textSelection(.enabled) }
                            }.padding(10).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
                        }
                        if model.rows.isEmpty && !model.loading { Text("没有可分享的消息").foregroundStyle(.secondary) }
                    }
                }
            }
            TextField("脱敏关键词，以逗号分隔", text: $model.keywords)
            HStack {
                Picker("导出格式", selection: $format) { Text("PNG").tag("PNG"); Text("HTML").tag("HTML") }.pickerStyle(.segmented).frame(width: 180)
                if format == "PNG" { TextField("图片宽度", text: $width).frame(width: 100); Text("320–2400 px").font(.caption) }
                Spacer()
                Button(exporting ? "正在导出…" : "导出 \(format)") { Task { await export() } }.buttonStyle(WispButtonStyle(primary: true)).disabled(exporting || model.selected.isEmpty)
            }
        }.padding(20).frame(minWidth: 620, idealWidth: 820, minHeight: 500, idealHeight: 700)
            .background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).tint(WispDesign.color("clay", scheme))
            .interactiveDismissDisabled(exporting)
            .background(NativeSettingsEscape(enabled: !exporting, close: close))
            .task { await model.load() }
    }
    private func export() async {
        exporting = true; error = nil
        defer { exporting = false }
        let rows = model.selected; let isPNG = format == "PNG"
        do {
            let data: Data
            if isPNG { data = try NativeSharePage.png(rows: rows, width: NativeShare.width(width), scheme: scheme) }
            else { data = Data(try await model.html(rows: rows, dark: scheme == .dark).utf8) }
            let panel = NSSavePanel(); panel.allowedContentTypes = [isPNG ? .png : .html]; panel.nameFieldStringValue = "wisp-share.\(isPNG ? "png" : "html")"
            guard await panel.begin() == .OK, let url = panel.url else { return }
            try data.write(to: url, options: .atomic)
        } catch { self.error = error.localizedDescription }
    }
}
struct NativeSharePage: View {
    let rows: [NativeShareRow]
    let scheme: ColorScheme
    static func label(_ role: String) -> String { role == "user" ? "你" : role == "reasoning" ? "思考" : "Wisp Science" }
    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            Text("Wisp Science").font(WispDesign.font(size: 16, weight: .semibold))
            Divider()
            ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                VStack(alignment: .leading, spacing: 8) {
                    Text(Self.label(row.role)).font(WispDesign.font(size: 11, weight: .semibold)).foregroundStyle(WispDesign.color("text-muted", scheme))
                    if row.role == "assistant" { NativeShareMarkdown(text: row.text) }
                    else { Text(row.text).font(WispDesign.font(size: 14)).fixedSize(horizontal: false, vertical: true) }
                }.padding(row.role == "user" ? 12 : 0)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(row.role == "user" ? WispDesign.color("bg-sunken", scheme) : .clear, in: RoundedRectangle(cornerRadius: 12))
            }
            Divider(); Text(localized("来自 wisp-science 的分享")).font(.caption).foregroundStyle(.secondary)
        }.padding(24).background(WispDesign.color("bg-app", scheme)).foregroundStyle(WispDesign.color("text", scheme)).environment(\.colorScheme, scheme)
    }
    @MainActor static func png(rows: [NativeShareRow], width: Int, scheme: ColorScheme) throws -> Data {
        let view = NativeSharePage(rows: rows, scheme: scheme).frame(width: CGFloat(width))
        let sizing = NSHostingView(rootView: view)
        let height = sizing.fittingSize.height
        guard height.isFinite, height > 0, height <= 32768, height * Double(width) <= 40_000_000 else {
            throw ProjectBrowserError.unavailable("长图超出图片尺寸限制，请减少所选消息或导出 HTML。")
        }
        let renderer = ImageRenderer(content: view)
        renderer.scale = 1
        guard let image = renderer.cgImage, let data = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:]) else { throw ProjectBrowserError.unavailable("图片渲染失败") }
        return data
    }
}
