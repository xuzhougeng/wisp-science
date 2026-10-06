import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeSessionExportSheet: View {
    @StateObject private var model: NativeSessionExportModel
    let close: () -> Void
    init(client: any NativeConversationQuerying, source: BrowserSession, writable: @escaping () -> Bool, close: @escaping () -> Void) {
        _model = StateObject(wrappedValue: NativeSessionExportModel(client: client, source: source, writable: writable)); self.close = close
    }
    init(model: NativeSessionExportModel, close: @escaping () -> Void) {
        _model = StateObject(wrappedValue: model); self.close = close
    }
    private var busy: Bool { model.writing || model.choosingDestination }
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Text(localized("导出会话 ZIP")).font(.headline)
                Spacer(); Button(localized("关闭"), action: close).disabled(busy)
            }
            Text(model.preview?.title ?? model.source.title).font(.subheadline).lineLimit(2)
            Text(localized("归档包含当前模型上下文、工具记录和终止事件。压缩前的历史消息不在此 ZIP 中。"))
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            Toggle(localized("包含已登记产物和溯源信息"), isOn: Binding(get: { model.includeArtifacts }, set: { files in
                model.chooseArtifacts(files); Task { await model.readPreview() }
            })).disabled(busy || model.reading || model.result != nil || model.uncertain)
            if model.reading || model.writing { ProgressView(localized(model.writing ? "正在写入归档…" : "正在校验会话与产物…")).controlSize(.small) }
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    if let error = model.error { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
                    if model.uncertain { Text(localized("重新预览不会再次提交。请在保存位置核对归档后关闭窗口。")) .font(.caption).foregroundStyle(.secondary) }
                    if model.result == nil, let path = model.savePath { Text(path).font(.caption).textSelection(.enabled) }
                    if let preview = model.preview {
                        Text("\(localized("当前上下文消息")): \(preview.message_count) · \(localized("工具调用")): \(preview.tool_call_count) · \(localized("终止事件")): \(preview.terminal_event_count)")
                            .font(.callout).fixedSize(horizontal: false, vertical: true)
                        if preview.include_artifacts {
                            Text("\(localized("产物数量")): \(preview.artifacts.count) · \(Self.size(preview.artifact_bytes))").font(.subheadline)
                            ForEach(Array(preview.artifacts.enumerated()), id: \.offset) { _, file in
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(file.path).textSelection(.enabled)
                                    Text(Self.size(file.bytes)).foregroundStyle(.secondary)
                                }.font(.caption)
                            }
                            if !preview.missing_artifacts.isEmpty {
                                Text(localized("以下文件不可用，仅在归档清单中记录原因")).font(.subheadline)
                                ForEach(Array(preview.missing_artifacts.enumerated()), id: \.offset) { _, file in
                                    Text(file.path + "\n" + file.error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                                }
                            }
                        }
                    }
                    if let result = model.result {
                        Text(localized("会话归档已保存")).font(.headline)
                        Text(result.destination_path).font(.caption).textSelection(.enabled)
                        Text(Self.size(result.bytes)).font(.caption).foregroundStyle(.secondary)
                        Text("SHA-256: " + result.checksum).font(.system(size: 11, design: .monospaced)).textSelection(.enabled)
                    }
                }.frame(maxWidth: .infinity, alignment: .leading)
            }.frame(maxWidth: .infinity, maxHeight: .infinity)
            ViewThatFits(in: .horizontal) {
                HStack { refresh; Spacer(); save }
                VStack(alignment: .trailing, spacing: 10) { refresh; save }
            }
        }.padding(24).frame(minWidth: 320, idealWidth: 560, maxWidth: 680, minHeight: 380, idealHeight: 600, maxHeight: 740)
            .interactiveDismissDisabled(busy)
            .background(NativeSettingsEscape(enabled: !busy, close: close))
            .task { if model.preview == nil && model.result == nil { await model.readPreview() } }.onDisappear { model.close() }
    }
    private var refresh: some View {
        Button(localized("重新预览")) { Task { await model.readPreview() } }.disabled(model.reading || busy || model.result != nil).fixedSize()
    }
    @ViewBuilder private var save: some View {
        if let result = model.result {
            Button(localized("在 Finder 中显示")) { NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: result.destination_path)]) }.fixedSize()
        } else {
            Button(localized("另存为 ZIP…"), action: model.chooseDestination).disabled(!model.canExport).buttonStyle(WispButtonStyle(primary: true)).fixedSize()
        }
    }
    private static func size(_ bytes: UInt64) -> String {
        if bytes < 1000 { return "\(bytes) B" }
        let formatter = ByteCountFormatter(); formatter.countStyle = .file
        formatter.allowedUnits = [.useKB, .useMB, .useGB, .useTB]; formatter.allowsNonnumericFormatting = false
        return formatter.string(fromByteCount: Int64(clamping: bytes))
    }
}
