import SwiftUI
import PhotosUI
import UIKit
import WispRemote

struct NotebookView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: NotebookModel
    @ObservedObject var connection: RemoteConnection

    var body: some View {
        NavigationStack(path: $workspace.notebookPath) {
            List {
                Section {
                    NavigationLink(value: NotebookDestination.create(.photos)) {
                        NotebookEntryLabel(icon: .camera, title: "拍下纸质实验记录", detail: "保留原图，识别并校对文字")
                    }.disabled(!model.loaded).accessibilityIdentifier("notebook-start-photos")
                    NavigationLink(value: NotebookDestination.create(.observation)) {
                        NotebookEntryLabel(icon: .edit, title: "记下刚才的观察", detail: "写过程、现象，也可以附上照片")
                    }.disabled(!model.loaded).accessibilityIdentifier("notebook-add")
                    NavigationLink(value: NotebookDestination.example) {
                        NotebookEntryLabel(icon: .book, title: "先看一条示例", detail: "了解一份完整记录是什么样")
                    }.accessibilityIdentifier("notebook-example")
                } header: {
                    Text("你现在想做什么？").font(.title2.weight(.semibold)).textCase(nil)
                } footer: {
                    Text("无需连接主机。文字和照片先存在手机，完成后再决定是否交给 Wisp 整理。")
                }
                if let error = model.error { Section { Text(error).foregroundStyle(WispStyle.warning) } }
                Section("本机实验记录") {
                    ForEach(model.records) { record in
                        NavigationLink(value: NotebookDestination.record(record.id)) {
                            VStack(alignment: .leading, spacing: 6) {
                                Text(record.title).font(.headline)
                                if !record.notes.isEmpty { Text(record.notes).font(.subheadline).foregroundStyle(.secondary).lineLimit(2) }
                                Text("\(record.day) · \(record.photos.count) 张照片").font(.caption).foregroundStyle(.secondary)
                                if let transfer = record.transfer { Text(transferLabel(transfer.state)).font(.caption).foregroundStyle(.secondary) }
                            }.padding(.vertical, 4)
                        }.accessibilityIdentifier("notebook-record-\(record.id)")
                    }
                    if model.records.isEmpty {
                        Text("第一条记录会出现在这里。从上方选择拍照或写观察，就能开始。")
                            .foregroundStyle(.secondary).accessibilityIdentifier("notebook-empty")
                    }
                }
            }
            .listStyle(.insetGrouped)
            .navigationTitle("实验记录本").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                if workspace.notebookPath.isEmpty {
                    ToolbarItem(placement: .topBarTrailing) {
                        WispToolbarButton(title: "怎么用", icon: .circleHelp) { workspace.notebookPath.append(.guide) }
                            .accessibilityIdentifier("notebook-help")
                    }
                }
            }
            .navigationDestination(for: NotebookDestination.self) { destination in
                switch destination {
                case .create(let entry): NotebookEditor(workspace: workspace, model: model, id: nil, entry: entry)
                case .guide: NotebookGuideView(workspace: workspace, example: false)
                case .example: NotebookGuideView(workspace: workspace, example: true)
                case .record(let id): NotebookEditor(workspace: workspace, model: model, id: id, entry: .observation)
                case .photo(let record, let photo): NotebookPhotoView(workspace: workspace, model: model, recordID: record, photoID: photo)
                case .saved(let id): NotebookSavedView(workspace: workspace, model: model, id: id)
                case .send(let id): NotebookSendView(workspace: workspace, model: model, connection: connection, id: id)
                }
            }
        }.accessibilityElement(children: .contain).accessibilityIdentifier("notebook-page")
    }
}

private struct NotebookEntryLabel: View {
    let icon: WispIcon.Kind
    let title: String
    let detail: String
    var body: some View {
        HStack(alignment: .top, spacing: 14) {
            WispIcon(icon, size: 24).foregroundStyle(WispStyle.accent).padding(.top, 3)
            VStack(alignment: .leading, spacing: 5) {
                Text(title).font(.headline)
                Text(detail).font(.subheadline).foregroundStyle(.secondary)
            }
        }.padding(.vertical, 12).frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct NotebookStep: View {
    let number: String
    let title: String
    let detail: String
    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            Text(number).font(.headline).foregroundStyle(WispStyle.accent)
                .frame(minWidth: 28, minHeight: 28).background(WispStyle.accent.opacity(0.1), in: Circle())
            VStack(alignment: .leading, spacing: 6) {
                Text(title).font(.headline)
                Text(detail).font(.subheadline).foregroundStyle(.secondary)
            }
        }.padding(.vertical, 6)
    }
}

private struct NotebookGuideView: View {
    @ObservedObject var workspace: RemoteWorkspace
    let example: Bool
    var body: some View {
        List {
            if example {
                Section {
                    Text("RNA 提取 · 第 2 批").font(.title2.weight(.semibold))
                    Text("教学示例，不会加入你的本机记录。").font(.caption).foregroundStyle(.secondary)
                    Text("今天提取 A01–A06 的总 RNA。\n\nA03 出现少量沉淀，补做一次离心。其余样本状态正常。\n\n下一步：复测 A03，核对浓度。")
                    Text("可附上：纸质实验记录、仪器读数照片；每张照片都能查看和校对识别文字。")
                        .font(.subheadline).foregroundStyle(.secondary)
                } header: { Text("一条记录可以包含什么") }
            }
            Section("从开始到保存") {
                NotebookStep(number: "1", title: "拍照或写观察", detail: "日期会自动填写，标题可以稍后补。可以只写文字，也可以拍照或从相册选图。")
                NotebookStep(number: "2", title: "核对照片上的文字", detail: "识别在手机上完成。重点检查样本编号、数值和单位；暂不校对也能保留原图。")
                NotebookStep(number: "3", title: "完成并保存", detail: "文字写好后点「完成」。原图在导入时就已保存；回到「实验记录本」即可再次打开。")
            }
            Section("需要 Wisp 帮忙时") {
                Text("本机保存与发送是两件事。连接电脑端 Wisp 主机后，选择内容、项目和对话，再确认发送；整理结果出现在所选对话里。")
                Text("记录不会因为恢复联网而自动上传。").font(.caption).foregroundStyle(.secondary)
            }
            Section {
                Button("开始写自己的记录") { workspace.notebookPath = [.create(.observation)] }
                    .disabled(!workspace.notebook.loaded).accessibilityIdentifier("notebook-example-start")
                Button("开始拍照或选图") { workspace.notebookPath = [.create(.photos)] }
                    .disabled(!workspace.notebook.loaded)
            }
        }.navigationTitle(example ? "记录示例" : "记录本怎么用").navigationBarTitleDisplayMode(.inline)
    }
}

private struct NotebookEditor: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: NotebookModel
    let id: String?
    let entry: NotebookEntry
    @State private var savedID: String?
    @State private var title = ""
    @State private var date = Date()
    @State private var notes = ""
    @State private var picks: [PhotosPickerItem] = []
    @State private var importing = false
    @State private var initialized = false
    @State private var visible = false
    private var recordID: String? { savedID ?? id }
    private var record: NotebookRecord? { recordID.flatMap { model.record($0) } }
    private var canAddPhoto: Bool { model.loaded && !importing && (record?.photos.count ?? 0) < 50 }

    var body: some View {
        Form {
            if entry == .photos {
                Section {
                    NotebookStep(number: "1", title: "拍下记录，或从相册选择", detail: "先把纸质记录或仪器读数收进手机，接下来再核对识别文字。")
                    photoActions
                } footer: { Text("原图导入后就会保存在手机。取消相机或相册不会新建空记录。") }
            } else {
                Section {
                    Text("直接写下过程或观察，也可以在下方添加照片。写好后点右上角「完成」。")
                        .font(.subheadline).foregroundStyle(.secondary)
                }
            }
            Section {
                TextField("实验名称（可稍后补）", text: $title, axis: .vertical).accessibilityIdentifier("notebook-title")
                DatePicker("实验日期", selection: $date, displayedComponents: .date)
                ZStack(alignment: .topLeading) {
                    if notes.isEmpty {
                        Text("做了什么？观察到了什么？\n例如：A03 出现沉淀，补做一次离心。")
                            .foregroundStyle(.secondary).padding(.horizontal, 5).padding(.vertical, 8)
                            .allowsHitTesting(false).accessibilityHidden(true)
                    }
                    TextEditor(text: $notes).scrollContentBackground(.hidden).frame(minHeight: 160)
                        .accessibilityLabel("实验记录文字").accessibilityIdentifier("notebook-notes")
                }
            } header: { Text("过程与观察") } footer: { Text("标题留空时，会使用第一行观察作为标题。文字修改需要点「完成」保存。") }
            Section {
                if entry != .photos { photoActions }
                if importing { ProgressView("正在保存原图并识别文字…").accessibilityIdentifier("notebook-importing") }
                ForEach(Array((record?.photos ?? []).enumerated()), id: \.element.id) { index, photo in
                    Button {
                        if let id = save() { workspace.notebookPath = [.record(id), .photo(id, photo.id)] }
                    } label: {
                        HStack(alignment: .top, spacing: 12) {
                            NotebookThumbnail(model: model, photoID: photo.id).frame(width: 72, height: 90).clipped()
                            VStack(alignment: .leading, spacing: 6) {
                                Text("第 \(index + 1) 页").font(.headline)
                                Text(model.recognizing.contains(photo.id) ? "正在本地识别…" : "查看原图与校对文字")
                                    .font(.caption).foregroundStyle(.secondary)
                                if !photo.text.isEmpty { Text(photo.text).font(.caption).foregroundStyle(.secondary).lineLimit(2) }
                            }
                            Spacer(minLength: 0)
                            WispIcon(.chevronRight, size: 14).foregroundStyle(.secondary)
                        }
                    }.buttonStyle(.plain).disabled(importing).accessibilityIdentifier("notebook-photo-\(photo.id)")
                }
            } header: { Text("这条记录的照片") } footer: {
                Text("文字识别在设备上完成，不上传照片。每条记录最多 50 张；原图已保存，识别失败也可以稍后校对。")
            }
            if let error = model.error { Section { Text(error).foregroundStyle(WispStyle.warning).accessibilityIdentifier("notebook-error") } }
        }
        .scrollDismissesKeyboard(.interactively)
        .navigationTitle("实验记录").navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("完成") {
                    if let id = save() { workspace.notebookPath = [.record(id), .saved(id)] }
                }.disabled(!model.loaded || importing).accessibilityIdentifier("notebook-save")
            }
        }
        .sheet(isPresented: $workspace.notebookCameraPresented) {
            NotebookCamera { data in
                workspace.notebookCameraPresented = false
                guard let data else { return }
                let origin = workspace.notebookPath
                importing = true
                Task {
                    let imported = await importPhoto(data)
                    importing = false
                    openImported(imported, from: origin)
                }
            }.ignoresSafeArea()
        }
        .photosPicker(isPresented: $workspace.notebookLibraryPresented, selection: $picks,
                      maxSelectionCount: max(1, min(10, 50 - (record?.photos.count ?? 0))),
                      selectionBehavior: .ordered, matching: .images, preferredItemEncoding: .current)
        .onAppear {
            visible = true
            guard !initialized else { return }
            if let record { title = record.title; date = ChecklistDate.date(record.day) ?? Date(); notes = record.notes }
            initialized = true
        }
        .onDisappear { visible = false }
        .onChange(of: picks) { _, selected in
            guard !selected.isEmpty else { return }
            let origin = workspace.notebookPath
            importing = true; picks = []
            Task {
                var first: (record: String, photo: String)?
                for pick in selected {
                    do {
                        guard let data = try await pick.loadTransferable(type: Data.self) else { throw RemoteError.service("无法读取所选照片。") }
                        let imported = await importPhoto(data)
                        if first == nil { first = imported }
                    } catch { model.error = error.localizedDescription }
                }
                importing = false
                openImported(first, from: origin)
            }
        }
    }

    private var photoActions: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button { workspace.notebookCameraPresented = true } label: {
                HStack { WispIcon(.camera); Text("拍下实验记录") }.frame(minHeight: 44)
            }.disabled(!canAddPhoto || !UIImagePickerController.isSourceTypeAvailable(.camera))
                .accessibilityIdentifier("notebook-take-photo")
            Button { workspace.notebookLibraryPresented = true } label: {
                HStack { WispIcon(.image); Text("从相册选择照片") }.frame(minHeight: 44)
            }.disabled(!canAddPhoto).accessibilityIdentifier("notebook-import-photos")
            if !UIImagePickerController.isSourceTypeAvailable(.camera) {
                Text("这台设备无法使用相机，请从相册选择照片。").font(.caption).foregroundStyle(.secondary)
            }
        }.buttonStyle(.borderless)
    }
    private func save() -> String? {
        if let id = recordID {
            let name = title.trimmingCharacters(in: .whitespacesAndNewlines)
            return model.saveNotes(id, title: name.isEmpty ? (record?.title ?? "实验记录") : name,
                                   day: ChecklistDate.string(date), notes: notes) ? id : nil
        }
        savedID = model.createObservation(title: title, day: ChecklistDate.string(date), notes: notes)
        return savedID
    }
    private func importPhoto(_ data: Data) async -> (record: String, photo: String)? {
        if let id = recordID {
            guard save() != nil, let photo = await model.addPhoto(data, record: id) else { return nil }
            return (id, photo)
        }
        let result = await model.createFromPhoto(data, title: title, day: ChecklistDate.string(date), notes: notes)
        if let result { savedID = result.record }
        return result
    }
    private func openImported(_ imported: (record: String, photo: String)?, from origin: [NotebookDestination]) {
        guard let imported, visible, workspace.tab == .notebook, workspace.notebookPath == origin else { return }
        workspace.notebookPath = [.record(imported.record), .photo(imported.record, imported.photo)]
    }
}

private struct NotebookPhotoView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: NotebookModel
    let recordID: String
    let photoID: String
    @State private var text = ""
    @State private var saved = ""
    private var photo: NotebookPhoto? { model.record(recordID)?.photos.first { $0.id == photoID } }
    var body: some View {
        Form {
            Section {
                NotebookStep(number: "2", title: "核对识别文字", detail: "原图已经保存在手机。核对样本编号、数值和单位；也可以先完成记录，稍后再校对。")
            }
            Section("原始照片") { NotebookThumbnail(model: model, photoID: photoID, maxPixelSize: 1600).frame(maxWidth: .infinity).frame(minHeight: 220) }
            Section("设备识别文字 · 可直接修改") {
                TextEditor(text: $text).frame(minHeight: 180).accessibilityLabel("照片识别文字").accessibilityIdentifier("notebook-ocr-text")
                Button("保存校对文字并完成") {
                    if model.saveText(record: recordID, photo: photoID, text: text) {
                        saved = text; workspace.notebookPath = [.record(recordID), .saved(recordID)]
                    }
                }.accessibilityIdentifier("notebook-save-ocr")
                if photo?.text.isEmpty == true {
                    Button("重新识别这张照片") { Task { await model.recognize(record: recordID, photo: photoID) } }
                        .disabled(model.recognizing.contains(photoID)).accessibilityIdentifier("notebook-recognize")
                }
                if model.recognizing.contains(photoID) { ProgressView("正在本地识别…") }
            }
            Section {
                Button("暂不校对，先完成记录") { workspace.notebookPath = [.record(recordID), .saved(recordID)] }
                    .accessibilityIdentifier("notebook-skip-ocr")
                Text("原图与已有识别文字会保留；当前未保存的校对修改不会保留。").font(.caption).foregroundStyle(.secondary)
            }
            if let error = model.error { Text(error).foregroundStyle(WispStyle.warning) }
        }.navigationTitle("照片与文字").navigationBarTitleDisplayMode(.inline)
            .onAppear { saved = photo?.text ?? ""; text = saved }
            .onChange(of: photo?.text) { _, value in if text == saved { saved = value ?? ""; text = saved } }
    }
}

private struct NotebookSavedView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: NotebookModel
    let id: String
    var body: some View {
        List {
            if let record = model.record(id) {
                Section {
                    HStack(spacing: 10) { WispIcon(.check, size: 28).foregroundStyle(WispStyle.success); Text("这条记录，已经收好了").font(.title2.weight(.semibold)) }
                    Text(record.title).font(.headline)
                    Text("\(record.day) · \(record.photos.count) 张照片").font(.subheadline).foregroundStyle(.secondary)
                    Text("已保存在这台设备。回到「实验记录本」即可找到，也可以继续补充文字和照片。")
                        .accessibilityIdentifier("notebook-saved-local")
                    Text(record.transfer == nil ? "尚未上传到 Wisp 主机。" : "本次修改已在本机保存；之前的发送状态可在发送页面核对。")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Section {
                    Button("回到实验记录本") { workspace.notebookPath = [] }.accessibilityIdentifier("notebook-return-list")
                    Button("继续编辑这条记录") { workspace.notebookPath = [.record(id)] }.accessibilityIdentifier("notebook-continue-editing")
                }
                Section {
                    Text("连接电脑端 Wisp 主机后，选择要发送的文字、照片及目标对话，让 Wisp 帮你整理实验过程。")
                        .font(.subheadline).foregroundStyle(.secondary)
                    Button("选择内容，交给 Wisp 整理") { workspace.notebookPath.append(.send(id)) }
                        .accessibilityIdentifier("notebook-review-send")
                    Text("这是可选操作。联网后也不会自动上传记录。").font(.caption).foregroundStyle(.secondary)
                } header: { Text("下一步 · 可选") }
            }
        }.navigationTitle("已保存到本机").navigationBarTitleDisplayMode(.inline)
    }
}

private struct NotebookSendView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: NotebookModel
    @ObservedObject var connection: RemoteConnection
    let id: String
    @State private var selected: Set<String> = []
    @State private var includeNotes = true
    @State private var includeText = true
    @State private var project: String?
    @State private var session: String?
    @State private var sessions: [RemoteValue] = []
    @State private var loadingSessions = false
    @State private var targetError: String?
    private var route: ConversationRoute? {
        guard let project, let session else { return nil }; return ConversationRoute(project: project, session: session)
    }
    private var online: Bool { !workspace.isDemo && workspace.configuration != nil && connection.state == .online }
    var body: some View {
        Form {
            if let record = model.record(id) {
                Section {
                    Text("这一步会把所选内容发送到电脑端 Wisp 对话，整理结果也会出现在那里。手机上的原始记录仍会保留。")
                        .font(.subheadline).foregroundStyle(.secondary)
                }
                Section("1 · 选择要整理的内容") {
                    Toggle("实验记录文字", isOn: $includeNotes)
                    Toggle("附上所选照片的校对文字", isOn: $includeText)
                    ForEach(Array(record.photos.enumerated()), id: \.element.id) { index, photo in
                        Toggle(isOn: Binding(get: { selected.contains(photo.id) }, set: { value in
                            if value && selected.count < 4 { selected.insert(photo.id) } else { selected.remove(photo.id) }
                        })) {
                            HStack { NotebookThumbnail(model: model, photoID: photo.id).frame(width: 52, height: 65).clipped(); Text("第 \(index + 1) 页") }
                        }.disabled(!selected.contains(photo.id) && selected.count >= 4)
                            .accessibilityIdentifier("notebook-select-\(photo.id)")
                    }
                    Text("每次最多发送 4 张照片。发送副本会缩小以适配远程通道，原图保留在手机。发送后 Wisp 将在所选对话中整理记录。").font(.caption).foregroundStyle(.secondary)
                }
                Section("2 · 选择结果要出现的对话") {
                    if !online {
                        Text("先连接 Wisp 主机，才能选择项目与对话。本机记录已保存，暂时不连接也不会丢失。")
                            .foregroundStyle(.secondary).accessibilityIdentifier("notebook-send-offline")
                        Text("在电脑端开启远程访问并复制连接链接，再回到手机工作区的连接页粘贴；已配对的设备可从连接设置重新连接。")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    else {
                        Text(workspace.hostName).font(.headline)
                        ForEach(workspace.projects, id: \.recordID) { row in
                            Button { project = row["id"].string; session = nil; sessions = [] } label: {
                                HStack { WispIcon(project == row["id"].string ? .checkSquare : .square); Text(row["name"].string) }
                            }
                        }
                        if loadingSessions { ProgressView("正在读取对话…") }
                        ForEach(sessions, id: \.recordID) { row in
                            Button { session = row["id"].string } label: {
                                HStack { WispIcon(session == row["id"].string ? .checkSquare : .square); Text(row["title"].string.isEmpty ? "未命名对话" : row["title"].string) }
                            }
                        }
                        if project != nil && !loadingSessions && sessions.isEmpty { Text("此项目没有可选对话，请先创建对话。").foregroundStyle(.secondary) }
                        if let targetError { Text(targetError).foregroundStyle(WispStyle.warning) }
                    }
                }
                Section("3 · 核对后确认发送") {
                    DisclosureGroup("查看将发送的文字") {
                        Text((try? record.message(selected: selected, includeNotes: includeNotes, includeText: includeText)) ?? "请选择要发送的内容。").textSelection(.enabled)
                    }
                    if record.transfer?.state == "uncertain" || record.transfer?.state == "sending" {
                        Text("上次发送结果待确认，请先在目标主机的对话中核对。").foregroundStyle(WispStyle.warning)
                        Button("已核对未收到，允许重新发送") { model.acknowledgeUnknown(id) }
                    } else {
                        Button(model.sending ? "正在发送…" : "确认发送所选内容") {
                            guard let route, let configuration = workspace.configuration else { return }
                            let generation = connection.readSessionID
                            Task { await model.send(record: id, selected: selected, includeNotes: includeNotes, includeText: includeText,
                                route: route, scope: RemoteReadCache.scope(configuration), hostName: workspace.hostName,
                                connection: connection, expectedSession: generation) }
                        }.disabled(!online || model.sending || route == nil || (selected.isEmpty && (!includeNotes || record.notes.isEmpty)))
                            .accessibilityIdentifier("notebook-confirm-send")
                    }
                    if let transfer = record.transfer { Text(transferLabel(transfer.state)).font(.caption) }
                    if let error = model.error { Text(error).foregroundStyle(WispStyle.warning) }
                }
            }
        }.navigationTitle("发送实验记录").navigationBarTitleDisplayMode(.inline)
            .task(id: project) {
                guard let project, online else { return }
                let generation = connection.readSessionID
                loadingSessions = true; targetError = nil
                defer { loadingSessions = false }
                do {
                    let result = try await connection.call("remote_sessions", project: project)
                    try Task.checkCancellation()
                    guard self.project == project, generation == connection.readSessionID else { return }
                    sessions = RemoteWorkspace.ordered(result.array)
                } catch { if !Task.isCancelled { targetError = error.localizedDescription } }
            }
            .onChange(of: connection.readSessionID) { _, _ in project = nil; session = nil; sessions = [] }
    }
}

private struct NotebookThumbnail: View {
    let model: NotebookModel
    let photoID: String
    var maxPixelSize = 320
    var body: some View {
        if let image = model.thumbnail(photoID, maxPixelSize: maxPixelSize) { Image(uiImage: image).resizable().scaledToFit().accessibilityLabel("实验记录照片") }
        else { Text("无法读取本机照片").font(.caption).foregroundStyle(.secondary) }
    }
}

private func transferLabel(_ state: String) -> String {
    ["uploading": "正在上传所选照片", "sending": "发送结果待确认", "sent": "所选内容已发送",
     "failed": "本机记录已保存，尚未发送", "uncertain": "发送结果待确认"][state] ?? state
}

private struct NotebookCamera: UIViewControllerRepresentable {
    let finished: (Data?) -> Void
    func makeCoordinator() -> Coordinator { Coordinator(finished) }
    func makeUIViewController(context: Context) -> UIImagePickerController {
        let picker = UIImagePickerController(); picker.sourceType = .camera; picker.delegate = context.coordinator
        return picker
    }
    func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}
    final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
        let finished: (Data?) -> Void
        init(_ finished: @escaping (Data?) -> Void) { self.finished = finished }
        func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { finished(nil) }
        func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
            finished((info[.originalImage] as? UIImage)?.jpegData(compressionQuality: 1))
        }
    }
}
