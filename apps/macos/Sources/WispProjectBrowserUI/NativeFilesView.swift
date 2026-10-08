import AppKit
import SwiftUI
import UniformTypeIdentifiers
import WispProjectBrowser

struct NativeFilesView: View {
    @StateObject private var model: NativeFilesModel
    @StateObject private var chooser = NativeFilesChooser()
    @Environment(\.colorScheme) private var scheme
    @AppStorage("native.workspace.panel.grid") private var grid = false
    @AppStorage("native.workspace.file.sort") private var savedSort = "name"
    @State private var pathDraft = ""
    @State private var action: NativeFileActionSelection?
    @State private var exportError: String?
    @State private var dropTargeted = false
    @State private var dropLoading = false
    @State private var dropTask: Task<Void, Never>?
    let readOnly: Bool
    var quote: ((NativeSideChatQuote) -> Void)?
    var environments: () -> Void = {}
    var runs: (String) -> Void = { _ in }
    var transfersSupported = false
    init(client: any NativeConversationQuerying, projectID: String, sessionID: String, readOnly: Bool, quote: ((NativeSideChatQuote) -> Void)? = nil, environments: @escaping () -> Void = {}, runs: @escaping (String) -> Void = { _ in }, transfersSupported: Bool = false, model: NativeFilesModel? = nil) {
        _model = StateObject(wrappedValue: model ?? NativeFilesModel(client: client, projectID: projectID, sessionID: sessionID, readOnly: readOnly, transfersSupported: transfersSupported))
        self.readOnly = readOnly; self.quote = quote; self.environments = environments
        self.runs = runs; self.transfersSupported = transfersSupported
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            location
            HStack(spacing: 4) {
                Button { Task { await model.navigate(model.parent) } } label: { WispIcon(name: "arrow-up", size: 16).frame(width: 28, height: 28) }
                    .buttonStyle(.plain).disabled(model.loading || model.path == model.parent).help(localized("上级目录"))
                if model.local {
                    Text(model.path == "." ? localized("当前项目 /") : model.path).font(.caption).lineLimit(1).truncationMode(.head).textSelection(.enabled)
                } else {
                    TextField(localized("远程目录"), text: $pathDraft).textFieldStyle(.roundedBorder).font(.system(size: 12, design: .monospaced))
                        .onSubmit { Task { await model.navigate(pathDraft) } }.disabled(model.loading)
                }
                Spacer(minLength: 0)
                Button { Task { await model.refresh() } } label: { WispIcon(name: "refresh", size: 16).frame(width: 28, height: 28) }
                    .buttonStyle(.plain).disabled(model.loading || model.catalogLoading).help(localized("刷新"))
                Button { model.sorting.toggle() } label: { WispIcon(name: "sort", size: 16).frame(width: 28, height: 28) }
                    .buttonStyle(.plain).help(localized("排序文件")).accessibilityLabel(localized("排序文件"))
                if model.transfersSupported {
                    Button { chooser.upload(model) } label: { WispIcon(name: "upload", size: 16).frame(width: 28, height: 28) }
                        .buttonStyle(.plain).disabled(!model.canUpload || chooser.busy || dropLoading)
                        .help(localized("上传到当前目录")).accessibilityLabel(localized("上传到当前目录"))
                }
                if model.local {
                    Menu {
                        Button(localized("新建文件")) { action = .init(action: .createFile, directory: model.path) }
                        Button(localized("新建文件夹")) { action = .init(action: .createDirectory, directory: model.path) }
                    } label: { WispIcon(name: "plus", size: 16, menuScheme: scheme).frame(width: 28, height: 28) }
                        .menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize().accessibilityLabel(localized("新建文件或文件夹"))
                        .disabled(!model.canWrite || model.loading || !model.query.isEmpty)
                }
            }
            if model.local {
                HStack(spacing: 6) {
                    TextField(localized("搜索当前项目全部目录"), text: $model.query).textFieldStyle(.roundedBorder).accessibilityLabel(localized("搜索项目文件"))
                    Button { model.toggleSelectionMode() } label: { WispIcon(name: "check", size: 16).frame(width: 28, height: 28).background(model.selecting ? WispDesign.color("surface-hover", scheme) : .clear, in: RoundedRectangle(cornerRadius: 5)) }
                        .buttonStyle(.plain).help(localized("多选文件")).accessibilityLabel(localized("多选文件"))
                    NativePanelDisplayControls(grid: $grid).fixedSize()
                }
            }
            if model.selecting {
                HStack(spacing: 8) {
                    Button(localized("全选")) { model.selectAll() }.buttonStyle(.plain).font(.caption)
                    Text(localized("已选") + " \(model.selection.count)").font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    copyButton(absolute: false)
                    copyButton(absolute: true)
                }
            }
            if model.loading || model.catalogLoading || model.searchLoading || model.previewLoading { ProgressView().controlSize(.small) }
            if let error = exportError ?? model.error {
                Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled)
                HStack {
                    Button(localized("重新读取")) { Task { await model.refresh() } }
                    if !model.local { Button(localized("环境设置"), action: environments) }
                }.font(.caption)
            }
            transferResults
            ScrollView {
                LazyVGrid(columns: grid && model.local ? [GridItem(.adaptive(minimum: 130), alignment: .top)] : [GridItem(.flexible(), alignment: .leading)], alignment: .leading, spacing: 6) {
                    ForEach(model.rows) { row in fileRow(row) }
                    if model.rows.isEmpty && !model.loading && !model.catalogLoading && !model.searchLoading && model.error == nil {
                        Text(localized(model.query.isEmpty ? "目录为空" : "没有匹配的项目文件")).foregroundStyle(.secondary).padding()
                    }
                }
            }
            if !model.query.isEmpty { Text(localized("当前项目 · 最多 200 项")).font(.caption).foregroundStyle(.secondary) }
        }.frame(maxHeight: .infinity)
            .background(NativeFilesWindowAnchor(owner: chooser).frame(width: 0, height: 0))
            .overlay { RoundedRectangle(cornerRadius: 8).stroke(dropTargeted && model.canUpload ? WispDesign.color("clay", scheme) : .clear, lineWidth: 2).allowsHitTesting(false) }
            .onDrop(of: [UTType.fileURL.identifier], isTargeted: $dropTargeted, perform: drop)
            .overlay(alignment: .topTrailing) { if model.sorting { sortMenu.padding(.top, 66) } }
            .task { model.sort = NativeFileSort(rawValue: savedSort) ?? .name; await model.open(); pathDraft = model.path }
            .task(id: model.query) { await model.search() }
            .onChange(of: model.path) { pathDraft = $0 }
            .onChange(of: model.sort) { savedSort = $0.rawValue }
            .onChange(of: readOnly) { model.readOnly = $0 }
            .onChange(of: transfersSupported) { model.transfersSupported = $0 }
            .task(id: model.transferPollKey) {
                while model.hasActiveTransfers && !model.closed && !Task.isCancelled {
                    await model.refreshTransferRuns()
                    if model.hasActiveTransfers { try? await Task.sleep(nanoseconds: 2_000_000_000) }
                }
            }
            .sheet(item: $action, onDismiss: { Task { await model.refresh() } }) { selection in
                NativeFileActionView(selection: selection, model: model.legacy, perform: { try await model.performAction($0, path: $1, newPath: $2) }) { action = nil }
            }
            .sheet(isPresented: Binding(get: { model.preview != nil }, set: { if !$0 { model.dismissPreview() } })) {
                if let preview = model.preview {
                    let identity = model.previewIdentity
                    NativePanelFilePreview(content: preview.content, close: model.dismissPreview,
                        save: model.canWrite && preview.context_id == "local" && !preview.content.truncated && preview.content.text != nil ? { try await model.save($0, original: preview.content) } : nil,
                        quote: quote == nil ? nil : { text in
                            guard let source = model.quote(text, source: preview.content.path) else { return }
                            quote?(source); model.dismissPreview()
                        }, documentQuote: quote == nil ? nil : { selection in
                            guard let source = model.documentQuote(selection, original: preview.content, identity: identity) else { return false }
                            quote?(source); model.dismissPreview(); return true
                        }, loadImage: { try await model.readPreviewImage($0, original: preview.content) }).id(identity)
                }
            }
            .onDisappear { dropTask?.cancel(); chooser.close(); model.close() }
    }
    private var location: some View {
        Picker(localized("文件位置"), selection: Binding(get: { model.contextID }, set: { context in Task { await model.chooseLocation(context) } })) {
            ForEach(model.catalog?.locations ?? []) { location in
                Text(location.id == "local" ? localized("当前项目") : location.label).tag(location.id)
            }
        }.labelsHidden().disabled(model.catalogLoading || model.fileActionBusy || model.saving || model.transferSubmitting || chooser.busy || dropLoading).accessibilityLabel(localized("文件位置"))
    }
    private var sortMenu: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(NativeFileSort.allCases, id: \.self) { choice in
                Button { model.sort = choice; model.sorting = false } label: {
                    HStack {
                        Text(localized(choice == .name ? "按名称" : choice == .size ? "按大小" : "按修改时间"))
                        Spacer()
                        if model.sort == choice { WispIcon(name: "check", size: 14) }
                    }.padding(8).contentShape(Rectangle())
                }.buttonStyle(.plain)
            }
        }.padding(4).frame(width: 180).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(WispDesign.color("border", scheme)))
            .shadow(radius: 8).background(NativeSettingsEscape { model.sorting = false })
    }
    private func copyButton(absolute: Bool, clicked: String? = nil) -> some View {
        Button { copy(absolute: absolute, clicked: clicked) } label: {
            WispIcon(name: absolute ? "link" : "copy", size: 16).frame(width: 28, height: 28)
        }.buttonStyle(.plain).disabled(model.pathsForCopy(clicked: clicked).isEmpty || model.copyBusy)
            .help(localized(absolute ? "复制绝对路径" : "复制相对路径"))
            .accessibilityLabel(localized(absolute ? "复制绝对路径" : "复制相对路径"))
    }
    private func copy(absolute: Bool, clicked: String?) {
        Task { if let paths = await model.copyPaths(absolute: absolute, clicked: clicked) { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(paths, forType: .string) } }
    }
    private func fileRow(_ row: NativeFileBrowserRow) -> some View {
        let layout = grid && model.local ? AnyLayout(VStackLayout(alignment: .leading, spacing: 0)) : AnyLayout(HStackLayout(alignment: .center, spacing: 4))
        return layout {
            Button {
                if model.selecting { model.toggle(row) }
                else if row.directory { Task { await model.navigate(row.path) } }
                else { Task { await model.read(row) } }
            } label: {
                HStack(alignment: .top, spacing: 8) {
                    WispIcon(name: model.selecting && model.selection.contains(row.id) ? "check" : row.directory ? "folder" : "doc", size: 18)
                    VStack(alignment: .leading, spacing: 3) {
                        Text(row.name).font(.system(size: 12, weight: .medium)).lineLimit(2).truncationMode(.middle)
                        Text(metadata(row)).font(.caption2).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                    }
                    Spacer(minLength: 0)
                }.padding(8).contentShape(Rectangle())
            }.buttonStyle(.plain).disabled(model.loading)
                .accessibilityLabel(row.name).accessibilityAddTraits(model.selection.contains(row.id) ? .isSelected : [])
            if model.local {
                HStack(spacing: 0) {
                    if !row.directory {
                        Button { saveAs(row) } label: { WispIcon(name: "download", size: 16).frame(width: 28, height: 28) }.buttonStyle(.plain).help(localized("保存副本"))
                    }
                    Menu {
                        Button { copy(absolute: false, clicked: row.id) } label: { Label { Text(localized("复制相对路径")) } icon: { WispIcon(name: "copy") } }
                        Button { copy(absolute: true, clicked: row.id) } label: { Label { Text(localized("复制绝对路径")) } icon: { WispIcon(name: "link") } }
                        Button { selectAction(.rename, row) } label: { Label { Text(localized("重命名")) } icon: { WispIcon(name: "edit") } }.disabled(!model.canWrite)
                        Button(role: .destructive) { selectAction(.delete, row) } label: { Label { Text(localized("删除")) } icon: { WispIcon(name: "trash") } }.disabled(!model.canWrite)
                    } label: { WispIcon(name: "more", size: 16, menuScheme: scheme).frame(width: 28, height: 28) }.menuStyle(.borderlessButton).menuIndicator(.hidden).fixedSize()
                }
            } else if model.transfersSupported && !row.directory {
                Button { chooser.download(row, model: model) } label: { WispIcon(name: "download", size: 16).frame(width: 28, height: 28) }
                    .buttonStyle(.plain).disabled(!model.canDownload || chooser.busy || dropLoading).help(localized("保存副本"))
            }
        }.background(WispDesign.color(model.selection.contains(row.id) ? "surface-hover" : "bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
            .contextMenu {
                if model.local {
                    Button(localized("复制相对路径")) { copy(absolute: false, clicked: row.id) }
                    Button(localized("复制绝对路径")) { copy(absolute: true, clicked: row.id) }
                }
            }
    }
    @ViewBuilder private var transferResults: some View {
        if model.transferSubmitting || dropLoading { HStack { ProgressView().controlSize(.small); Text(localized(dropLoading ? "读取拖放文件…" : "正在提交传输…")).font(.caption) } }
        if model.transferUnconfirmed && !model.transferSubmitting {
            Text(localized("传输结果未确认，未自动重试。请重新读取后再决定是否提交。")) .font(.caption).foregroundStyle(.orange)
            if let context = model.transferContext, context != "local" {
                Button(localized("查看传输任务")) { if !model.closed { runs(context) } }.font(.caption)
            }
        }
        if let error = model.transferError { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
        if let transfer = model.transfer {
            VStack(alignment: .leading, spacing: 5) {
                HStack {
                    Text(localized("传输结果")).font(.caption.bold())
                    Spacer()
                    if transfer.context_id != "local" {
                        Button(localized("查看传输任务")) { if !model.closed { runs(transfer.context_id) } }.font(.caption)
                    }
                }
                Text((transfer.context_id == "local" ? localized("当前项目") : model.catalog?.locations.first(where: { $0.id == transfer.context_id })?.label ?? transfer.context_id) + " · " + transfer.path)
                    .font(.caption2).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                ScrollView {
                    VStack(alignment: .leading, spacing: 5) {
                        ForEach(transfer.items) { item in
                            let run = model.transferRuns[item.id]
                            VStack(alignment: .leading, spacing: 3) {
                                HStack(alignment: .top) {
                                    Text(item.destination_path ?? item.source_path).font(.caption).lineLimit(2).truncationMode(.middle)
                                    Spacer(minLength: 4)
                                    Text(status(run?.status ?? item.status)).font(.caption2).foregroundStyle(.secondary)
                                }
                                if let error = item.error { Text(error).font(.caption2).foregroundStyle(.orange).textSelection(.enabled) }
                                if let run { transferProgress(run) }
                            }
                        }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }.frame(height: min(120, CGFloat(transfer.items.reduce(0) { $0 + 28 + ($1.error == nil ? 0 : 20) + ($1.run_id == nil ? 0 : 28) })))
            }.padding(8).background(WispDesign.color("bg-elev", scheme), in: RoundedRectangle(cornerRadius: 8))
        }
    }
    private func status(_ value: String) -> String {
        localized(["submitted": "已提交", "running": "正在传输", "succeeded": "传输完成", "failed": "传输失败", "cancelled": "已取消", "cancelling": "正在取消" ][value] ?? "状态未知")
    }
    @ViewBuilder private func transferProgress(_ run: NativeRun) -> some View {
        if let progress = try? JSONDecoder().decode(SettingsValue.self, from: Data(run.progress_json.utf8)),
           case let .integer(total) = progress["total_bytes"], case let .integer(completed) = progress["completed_bytes"], total > 0, completed >= 0 {
            if progress["indeterminate"] != .bool(true) { ProgressView(value: Double(min(completed, total)), total: Double(total)).controlSize(.small) }
            Text(formattedBytes(completed) + " / " + formattedBytes(total)).font(.caption2).foregroundStyle(.secondary)
        }
    }
    private func formattedBytes(_ count: Int64) -> String {
        var format = ByteCountFormatStyle(style: .file)
        format.locale = Locale(identifier: UserDefaults.standard.string(forKey: "nativeSettings.locale") == "en" ? "en_US" : "zh_CN")
        return format.format(count)
    }
    private func drop(_ providers: [NSItemProvider]) -> Bool {
        guard !dropLoading, !chooser.busy, let target = model.transferTarget(upload: true), !providers.isEmpty else { return false }
        dropLoading = true; exportError = nil
        dropTask = Task {
            defer { dropLoading = false }
            do { let sources = try await NativeFileDropSources.paths(providers); await model.upload(sources, target: target) }
            catch { if !model.closed, !Task.isCancelled, model.owns(target) { exportError = error.localizedDescription } }
        }
        return true
    }
    private func metadata(_ row: NativeFileBrowserRow) -> String {
        if !model.query.isEmpty { return row.path }
        let locale = Locale(identifier: UserDefaults.standard.string(forKey: "nativeSettings.locale") == "en" ? "en_US" : "zh_CN")
        if model.sort == .modified, let millis = row.modified, millis > 0 {
            return Date(timeIntervalSince1970: Double(millis) / 1000).formatted(Date.FormatStyle(date: .numeric, time: .shortened).locale(locale))
        }
        return row.directory ? localized("文件夹") : formattedBytes(Int64(clamping: row.size))
    }
    private func selectAction(_ kind: NativePanelFileAction, _ row: NativeFileBrowserRow) {
        let parent = (row.path as NSString).deletingLastPathComponent
        action = .init(action: kind, directory: parent.isEmpty ? "." : parent, name: row.name)
    }
    private func saveAs(_ row: NativeFileBrowserRow) {
        exportError = nil
        Task {
            do {
                let source = try await model.exportSource(row)
                let panel = NSSavePanel(); panel.nameFieldStringValue = source.name; panel.prompt = localized("保存副本")
                guard panel.runModal() == .OK, let destination = panel.url, !model.closed else { return }
                try await NativePanelExportCopy.copy(source, to: destination)
            } catch { if !model.closed { exportError = localized("保存副本失败：") + error.localizedDescription } }
        }
    }
}
