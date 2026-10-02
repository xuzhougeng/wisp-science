import SwiftUI
import WispProjectBrowser

struct NativeArtifactGroup: Identifiable {
    let id: String
    let registered: [NativePanelArtifact]
    let messages: [NativeTranscriptArtifact]
    var count: Int { registered.count + messages.count }
    var title: String { localized(["table": "表格", "latex": "公式", "image": "图片", "chart": "图表", "file": "文件"][id] ?? id) }
    static func collect(registered: [NativePanelArtifact], messages: [NativeTranscriptArtifact], query: String) -> [Self] {
        let registered = registered.filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) || $0.kind.localizedCaseInsensitiveContains(query) }
        let messages = messages.filter { query.isEmpty || $0.title.localizedCaseInsensitiveContains(query) || $0.source.localizedCaseInsensitiveContains(query) || $0.kind.localizedCaseInsensitiveContains(query) || localized($0.kind == "table" ? "表格" : "公式").localizedCaseInsensitiveContains(query) }
        let kinds = Set(registered.map(\.kind) + messages.map(\.kind)).sorted()
        return kinds.map { kind in Self(id: kind, registered: registered.filter { $0.kind == kind }, messages: messages.filter { $0.kind == kind }) }
    }
}

struct NativeArtifactCollection: View {
    let registered: [NativePanelArtifact]
    let messages: [NativeTranscriptArtifact]
    let query: String
    let grid: Bool
    let loading: Bool
    let failed: Bool
    let openRegistered: (String) -> Void
    let openMessage: (NativeTranscriptArtifact) -> Void
    let provenance: () -> Void
    private var groups: [NativeArtifactGroup] { NativeArtifactGroup.collect(registered: registered, messages: messages, query: query) }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(localized("文件来自当前会话；表格和公式来自当前显示的消息页。"))
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            ForEach(groups) { group in
                VStack(alignment: .leading, spacing: 8) {
                    HStack { Text(group.title).font(.subheadline.weight(.semibold)); Text("\(group.count)").font(.caption).foregroundStyle(.secondary); Spacer() }
                    LazyVGrid(columns: grid ? [GridItem(.adaptive(minimum: 140), alignment: .top)] : [GridItem(.flexible(), alignment: .leading)], alignment: .leading, spacing: 8) {
                        ForEach(group.registered) { artifact in
                            Button { openRegistered(artifact.id) } label: {
                                NativePanelTile(title: artifact.name, subtitle: artifact.logical_path ?? artifact.path, icon: "doc", grid: grid)
                            }.buttonStyle(.plain).contextMenu {
                                Button(localized("打开预览")) { openRegistered(artifact.id) }
                                Button(localized("查看溯源"), action: provenance)
                            }
                        }
                        ForEach(group.messages) { artifact in
                            Button { openMessage(artifact) } label: {
                                NativePanelTile(title: localized(artifact.kind == "table" ? "表格" : "公式") + " " + (artifact.title.split(separator: " ").last.map(String.init) ?? ""), subtitle: summary(artifact), icon: "doc", grid: grid)
                            }.buttonStyle(.plain)
                        }
                    }
                }
            }
            if groups.isEmpty && !loading && !failed {
                VStack(alignment: .leading, spacing: 6) {
                    Text(localized(query.isEmpty ? "暂无可显示的产物" : "没有匹配的产物")).font(.headline)
                    Text(localized(query.isEmpty ? "生成的文件会显示在这里；当前消息页中的完整表格和公式也可预览。" : "尝试其他名称或清除筛选。"))
                        .font(.caption).foregroundStyle(.secondary)
                }.padding(.vertical, 12)
            }
        }.frame(maxWidth: .infinity, alignment: .leading)
    }
    private func summary(_ artifact: NativeTranscriptArtifact) -> String {
        if let dimensions = artifact.tableDimensions {
            return String(format: localized("%d 行 × %d 列 · 当前消息页"), dimensions.rows, dimensions.columns)
        }
        return localized("LaTeX · 当前消息页")
    }
}
