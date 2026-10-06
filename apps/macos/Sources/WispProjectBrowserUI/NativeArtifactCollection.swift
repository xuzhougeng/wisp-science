import AppKit
import SwiftUI
import WispProjectBrowser

enum NativeWorkspacePath {
    static func display(_ path: String, root: String) -> String {
        if path.contains("://") { return path }
        var value = path.replacingOccurrences(of: "\\", with: "/")
        while value.hasPrefix("./") { value.removeFirst(2) }
        let base = root.replacingOccurrences(of: "\\", with: "/").trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        var normalizedRoot = root.replacingOccurrences(of: "\\", with: "/")
        while normalizedRoot.hasSuffix("/") { normalizedRoot.removeLast() }
        let windows = normalizedRoot.count > 1 && normalizedRoot.dropFirst().hasPrefix(":")
        let matches = windows ? value.lowercased().hasPrefix(normalizedRoot.lowercased() + "/") : value.hasPrefix(normalizedRoot + "/")
        if !base.isEmpty && matches { value = String(value.dropFirst(normalizedRoot.count + 1)) }
        return value.isEmpty ? "." : value
    }
    static func group(_ path: String, root: String) -> String {
        let value = display(path, root: root)
        let parts = value.split(separator: "/").map(String.init)
        if parts.count < 2 { return "." }
        let parent = parts.dropLast().joined(separator: "/")
        if value.hasPrefix("/") || value.contains(":") { return parts[parts.count - 2] + "/" }
        return parent + "/"
    }
    static func artifact(_ artifact: NativePanelArtifact, root: String) -> String { display(artifact.location ?? artifact.logical_path ?? artifact.path, root: root) }
}

struct NativeArtifactGroup: Identifiable {
    let id: String
    let registered: [NativePanelArtifact]
    let messages: [NativeTranscriptArtifact]
    var count: Int { registered.count + messages.count }
    var title: String { id == "." ? localized("项目根目录") : id.hasPrefix("@") ? localized(["@table": "表格", "@latex": "公式"][id] ?? String(id.dropFirst())) : id }
    static func collect(registered: [NativePanelArtifact], messages: [NativeTranscriptArtifact], query: String, root: String = "") -> [Self] {
        let registered = registered.filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) || $0.kind.localizedCaseInsensitiveContains(query) || NativeWorkspacePath.artifact($0, root: root).localizedCaseInsensitiveContains(query) }
        let messages = messages.filter { query.isEmpty || $0.title.localizedCaseInsensitiveContains(query) || $0.source.localizedCaseInsensitiveContains(query) || $0.kind.localizedCaseInsensitiveContains(query) || localized($0.kind == "table" ? "表格" : "公式").localizedCaseInsensitiveContains(query) }
        let groups = Dictionary(grouping: registered) { NativeWorkspacePath.group($0.location ?? $0.logical_path ?? $0.path, root: root) }
        return groups.keys.sorted().map { Self(id: $0, registered: groups[$0] ?? [], messages: []) }
            + Set(messages.map(\.kind)).sorted().map { kind in Self(id: "@" + kind, registered: [], messages: messages.filter { $0.kind == kind }) }
    }
}

struct NativeArtifactCollection: View {
    @Environment(\.colorScheme) private var scheme
    let registered: [NativePanelArtifact]
    let messages: [NativeTranscriptArtifact]
    let query: String
    let grid: Bool
    let loading: Bool
    let failed: Bool
    let openRegistered: (String) -> Void
    let openMessage: (NativeTranscriptArtifact) -> Void
    let provenance: () -> Void
    var projectRoot = ""
    var exportRegistered: (String) -> Void = { _ in }
    private var groups: [NativeArtifactGroup] { NativeArtifactGroup.collect(registered: registered, messages: messages, query: query, root: projectRoot) }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(localized("文件来自当前会话；表格和公式来自当前显示的消息页。"))
                .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
            ForEach(groups) { group in
                VStack(alignment: .leading, spacing: 8) {
                    HStack { Text(group.title).font(.subheadline.weight(.semibold)); Text("\(group.count)").font(.caption).foregroundStyle(.secondary); Spacer() }
                    LazyVGrid(columns: grid ? [GridItem(.adaptive(minimum: 140), alignment: .top)] : [GridItem(.flexible(), alignment: .leading)], alignment: .leading, spacing: 8) {
                        ForEach(group.registered) { artifact in
                            let path = NativeWorkspacePath.artifact(artifact, root: projectRoot)
                            let layout = grid ? AnyLayout(VStackLayout(alignment: .leading, spacing: 0)) : AnyLayout(HStackLayout(alignment: .top, spacing: 2))
                            layout {
                                Button { openRegistered(artifact.id) } label: {
                                    NativePanelTile(title: artifact.name, subtitle: path, icon: "doc", grid: grid)
                                }.buttonStyle(.plain)
                                HStack(spacing: 2) {
                                    if grid { Spacer(minLength: 0) }
                                    Button { exportRegistered(artifact.id) } label: { WispIcon(name: "download", size: 16).frame(width: 30, height: 30) }
                                        .buttonStyle(.plain).help("保存副本").accessibilityLabel("保存副本 " + artifact.name)
                                    Menu {
                                        Button { openRegistered(artifact.id) } label: { Label { Text("打开预览") } icon: { WispIcon(name: "expand") } }
                                        Button { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(path, forType: .string) } label: { Label { Text("复制路径") } icon: { WispIcon(name: "copy") } }
                                        Button(action: provenance) { Label { Text("查看溯源") } icon: { WispIcon(name: "research-trail") } }
                                    } label: { WispIcon(name: "more", size: 16).frame(width: 30, height: 30) }
                                        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().accessibilityLabel("产物操作 " + artifact.name)
                                }
                            }.background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
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
