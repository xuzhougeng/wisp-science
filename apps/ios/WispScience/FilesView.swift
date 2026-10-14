import SwiftUI
import WispRemote

private struct FileLocation: Hashable {
    let path: String
    let directory: Bool
}

struct FilesView: View {
    @ObservedObject var model: RemoteWorkspace
    let route: ConversationRoute
    let initialPath: String
    let isDirectory: Bool
    var body: some View {
        NavigationStack {
            FileContentView(model: model, connection: model.connection, route: route, location: FileLocation(path: initialPath, directory: isDirectory))
                .navigationDestination(for: FileLocation.self) { location in
                    FileContentView(model: model, connection: model.connection, route: route, location: location)
                }
                .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("完成") { model.sheet = nil } } }
        }.accessibilityElement(children: .contain).accessibilityIdentifier("files-sheet")
    }
}

private struct FileContentView: View {
    @ObservedObject var model: RemoteWorkspace
    @ObservedObject var connection: RemoteConnection
    let route: ConversationRoute
    let location: FileLocation
    @State private var value: RemoteValue?
    @State private var error: String?
    @State private var cacheDate: Date?
    @State private var refreshError: String?
    private var ext: String { (location.path as NSString).pathExtension.lowercased() }
    private var image: Bool { ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"].contains(ext) }
    var body: some View {
        VStack(spacing: 0) {
            if let cacheDate {
                Text("缓存预览 · \(cacheDate.formatted(date: .abbreviated, time: .shortened))\(refreshError == nil ? "" : " · 刷新失败")")
                    .font(.caption).foregroundStyle(.secondary).padding(8).accessibilityIdentifier("cached-file")
            }
        Group {
            if let error {
                VStack(spacing: 12) { Text(error); Button("重试") { Task { await load() } } }.padding()
            } else if location.directory, let value {
                List {
                    ForEach(value["entries"].array, id: \.entryName) { entry in
                        let path = location.path == "." ? entry["name"].string : location.path + "/" + entry["name"].string
                        NavigationLink(value: FileLocation(path: path, directory: entry["is_dir"].bool)) {
                            VStack(alignment: .leading, spacing: 4) {
                                Text(entry["name"].string)
                                Text(entry["is_dir"].bool ? "文件夹" : ByteCountFormatter.string(fromByteCount: Int64(clamping: entry["size"].uint ?? 0), countStyle: .file))
                                    .font(.caption).foregroundStyle(.secondary)
                            }.padding(.vertical, 3)
                        }
                    }
                    if value["entries"].array.isEmpty { Text("空文件夹").foregroundStyle(.secondary) }
                }.refreshable { await load() }
            } else if image {
                ScrollView { RemoteImageView(model: model, route: route, source: ["path": .string(location.path)], label: (location.path as NSString).lastPathComponent).padding() }
            } else if let value {
                ScrollView {
                    VStack(alignment: .leading, spacing: 12) {
                        if value["content"]["truncated"].bool { Text("文件较大，只显示开头部分。").font(.caption).foregroundStyle(.secondary) }
                        if case .string(let text) = value["content"]["text"] {
                            if ["md", "markdown", "pdf", "doc", "docx", "docm", "odt", "rtf", "epub", "ppt", "pptx", "odp", "xls", "xlsx", "ods"].contains(ext) {
                                MarkdownView(text: text, resources: [], model: model, route: route,
                                             directory: (location.path as NSString).deletingLastPathComponent)
                            } else { CodeText(text: text) }
                        } else { Text("此文件没有可用的文本预览，请在 Wisp 主机打开。") }
                    }.padding().frame(maxWidth: .infinity, alignment: .leading)
                }
            } else { ProgressView("读取文件…") }
        }
        }
        .navigationTitle(location.path == "." ? "项目文件" : (location.path as NSString).lastPathComponent)
        .navigationBarTitleDisplayMode(.inline)
        .task(id: "\(location.path):\(location.directory):\(connection.state)") { await load() }
    }
    private func load() async {
        error = nil
        refreshError = nil
        guard location.directory || !image else { return }
        let command = location.directory ? "native_conversation_panel_file_directory" : "native_conversation_panel_file_read"
        let args = route.args.merging(["path": .string(location.path)]) { _, v in v }
        if let cached = await model.cachedRead(command, project: route.project, args: args) {
            value = cached.value; cacheDate = cached.savedAt
        }
        guard connection.state == .online else {
            if value == nil { error = "此内容尚未缓存，请联网后打开。" }
            return
        }
        do {
            let result = try await model.read(command, project: route.project, args: args)
            try Task.checkCancellation()
            value = result; cacheDate = nil
        } catch is CancellationError { }
        catch {
            if value == nil { self.error = error.localizedDescription }
            else { refreshError = error.localizedDescription }
        }
    }
}

struct MarkdownView: View {
    let text: String
    let resources: [RemoteValue]
    @ObservedObject var model: RemoteWorkspace
    let route: ConversationRoute
    var directory = "."
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(Array(RemoteMarkdown.blocks(text).enumerated()), id: \.offset) { _, block in
                switch block {
                case .paragraph(let text): inline(text)
                case .heading(let level, let text): inline(text).font(level <= 2 ? .title3.bold() : .headline)
                case .code(let code): CodeText(text: code)
                case .quote(let text): inline(text).foregroundStyle(.secondary).padding(.leading, 12).overlay(alignment: .leading) { Rectangle().fill(.tertiary).frame(width: 3) }
                case .table(let rows):
                    ScrollView(.horizontal) {
                        Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 8) {
                            ForEach(Array(rows.enumerated()), id: \.offset) { row, cells in
                                GridRow {
                                    ForEach(Array(cells.enumerated()), id: \.offset) { _, cell in
                                        inline(cell).fontWeight(row == 0 ? .semibold : .regular).fixedSize(horizontal: true, vertical: false)
                                    }
                                }
                            }
                        }.padding(12)
                    }.background(.quaternary.opacity(0.4), in: RoundedRectangle(cornerRadius: 8))
                case .image(let reference, let label):
                    if let resource = resources.first(where: { $0["originalReference"].string == reference }) {
                        RemoteImageView(model: model, route: route, source: ["resource_id": resource["id"]], label: label)
                    } else if let path = RemoteMarkdown.localPath(reference, directory: directory) {
                        RemoteImageView(model: model, route: route, source: ["path": .string(path)], label: label)
                    } else { Text(label.isEmpty ? "外部图片" : label).foregroundStyle(.secondary) }
                }
            }
        }
        .font(.body).foregroundStyle(.primary)
        .lineSpacing(3).frame(maxWidth: .infinity, alignment: .leading)
        .textSelection(.enabled)
        .environment(\.openURL, OpenURLAction { url in
            if ["https", "http", "mailto"].contains(url.scheme?.lowercased() ?? "") { return .systemAction }
            if let path = RemoteMarkdown.localPath(url.absoluteString, directory: directory) {
                model.sheet = .files(route, path, false)
                return .handled
            }
            return .discarded
        })
    }
    private func inline(_ text: String) -> Text {
        Text((try? AttributedString(markdown: text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(text))
    }
}

struct CodeText: View {
    let text: String
    var body: some View {
        ScrollView(.horizontal) {
            Text(text).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading).padding(10)
        }.background(Color(uiColor: .tertiarySystemFill), in: RoundedRectangle(cornerRadius: 8))
    }
}

struct RemoteImageView: View {
    @ObservedObject var model: RemoteWorkspace
    @ObservedObject private var connection: RemoteConnection
    let route: ConversationRoute
    let source: [String: RemoteValue]
    let label: String
    @State private var image: UIImage?
    @State private var error: String?
    @State private var cacheDate: Date?
    init(model: RemoteWorkspace, route: ConversationRoute, source: [String: RemoteValue], label: String) {
        self.model = model; self.connection = model.connection
        self.route = route; self.source = source; self.label = label
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if let image { Image(uiImage: image).resizable().scaledToFit().accessibilityLabel(label) }
            else if let error { Text(error).font(.caption); Button("重试") { Task { await load() } } }
            else { ProgressView("读取图片…") }
            if !label.isEmpty { Text(label).font(.caption).foregroundStyle(.secondary) }
            if let cacheDate {
                Text("缓存图片 · \(cacheDate.formatted(date: .abbreviated, time: .shortened))")
                    .font(.caption2).foregroundStyle(.secondary)
            }
            if image != nil, let error { Text(error).font(.caption).foregroundStyle(.secondary) }
        }.task(id: "\(RemoteReadCache.key(scope: route.session, command: "native_conversation_image", project: route.project, args: source)):\(connection.state)") { await load() }
    }
    private func load() async {
        error = nil
        let args = route.args.merging(source) { _, v in v }
        if let cached = await model.cachedRead("native_conversation_image", project: route.project, args: args),
           let data = Data(base64Encoded: cached.value["base64"].string), let image = UIImage(data: data) {
            self.image = image; cacheDate = cached.savedAt
        }
        guard connection.state == .online else {
            if image == nil { error = "此图片尚未缓存，请联网后打开。" }
            return
        }
        do {
            let value = try await model.read("native_conversation_image", project: route.project, args: args)
            guard let data = Data(base64Encoded: value["base64"].string), let image = UIImage(data: data) else {
                throw RemoteError.service("图片无法显示。")
            }
            try Task.checkCancellation()
            self.image = image; cacheDate = nil
        } catch is CancellationError { }
        catch { self.error = error.localizedDescription }
    }
}

extension RemoteValue {
    var recordID: String { self["id"].string }
    var approvalID: String { self["approval_id"].string }
    var entryName: String { self["name"].string }
}
