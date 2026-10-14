import SwiftUI
import PhotosUI
import UIKit
import WispRemote

struct NotebookView: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: NotebookModel
    @ObservedObject var connection: RemoteConnection
    @State private var title = ""
    @State private var date = Date()
    var body: some View {
        NavigationStack(path: $workspace.notebookPath) {
            List {
                Section {
                    Text("照片和文字先保存在手机，选择目标对话并确认后才会发送。").font(.subheadline).foregroundStyle(.secondary)
                    TextField("实验标题", text: $title, axis: .vertical).accessibilityIdentifier("notebook-title")
                    DatePicker("实验日期", selection: $date, displayedComponents: .date)
                    Button("新建实验记录") {
                        if let id = model.add(title: title, day: ChecklistDate.string(date)) { title = ""; workspace.notebookPath.append(.record(id)) }
                    }.disabled(!model.loaded || title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                        .accessibilityIdentifier("notebook-add")
                }
                if let error = model.error { Section { Text(error).foregroundStyle(WispStyle.warning) } }
                Section("本机实验记录") {
                    ForEach(model.records) { record in
                        NavigationLink(value: NotebookDestination.record(record.id)) {
                            VStack(alignment: .leading, spacing: 5) {
                                Text(record.title).font(.headline)
                                Text("\(record.day) · \(record.photos.count) 张照片").font(.caption).foregroundStyle(.secondary)
                                if let transfer = record.transfer { Text(transferLabel(transfer.state)).font(.caption).foregroundStyle(.secondary) }
                            }
                        }.accessibilityIdentifier("notebook-record-\(record.id)")
                    }
                    if model.records.isEmpty { Text("拍下实验记录，或先记几句观察。无需联网。").foregroundStyle(.secondary) }
                }
            }.navigationTitle("实验记录本").navigationBarTitleDisplayMode(.inline)
                .toolbar { Button("完成") { workspace.sheet = nil } }
                .navigationDestination(for: NotebookDestination.self) { destination in
                    switch destination {
                    case .record(let id): NotebookEditor(workspace: workspace, model: model, id: id)
                    case .photo(let record, let photo): NotebookPhotoView(model: model, recordID: record, photoID: photo)
                    case .send(let id): NotebookSendView(workspace: workspace, model: model, connection: connection, id: id)
                    }
                }
        }.presentationDetents([.large])
            .sheet(isPresented: Binding(get: { workspace.notebookCameraRecord != nil }, set: { if !$0 { workspace.notebookCameraRecord = nil } })) {
                NotebookCamera { data in
                    let id = workspace.notebookCameraRecord
                    workspace.notebookCameraRecord = nil
                    if let data, let id { Task { await model.addPhoto(data, record: id) } }
                }.ignoresSafeArea()
            }
    }
}

private struct NotebookEditor: View {
    @ObservedObject var workspace: RemoteWorkspace
    @ObservedObject var model: NotebookModel
    let id: String
    @State private var title = ""
    @State private var date = Date()
    @State private var notes = ""
    @State private var picks: [PhotosPickerItem] = []
    @State private var importing = false
    @State private var initialized = false
    var body: some View {
        Form {
            if let record = model.record(id) {
                Section("实验文字") {
                    TextField("实验标题", text: $title, axis: .vertical)
                    DatePicker("实验日期", selection: $date, displayedComponents: .date)
                    TextEditor(text: $notes).frame(minHeight: 160).accessibilityLabel("实验记录文字").accessibilityIdentifier("notebook-notes")
                    Button("保存文字") { save() }.accessibilityIdentifier("notebook-save")
                }
                Section {
                    HStack {
                        Button {
                            save(); workspace.notebookCameraRecord = id
                        } label: { HStack { WispIcon(.camera); Text("拍照") } }
                            .disabled(!UIImagePickerController.isSourceTypeAvailable(.camera) || importing)
                        Spacer()
                        Button { workspace.notebookLibraryPresented = true } label: {
                            HStack { WispIcon(.image); Text("从相册添加") }
                        }.disabled(importing || record.photos.count >= 50).accessibilityIdentifier("notebook-import-photos")
                    }.buttonStyle(.borderless)
                    if importing { ProgressView("正在保存照片并识别文字…") }
                    ForEach(Array(record.photos.enumerated()), id: \.element.id) { index, photo in
                        NavigationLink(value: NotebookDestination.photo(id, photo.id)) {
                            HStack(alignment: .top) {
                                NotebookThumbnail(model: model, photoID: photo.id).frame(width: 72, height: 90).clipped()
                                VStack(alignment: .leading, spacing: 6) {
                                    Text("第 \(index + 1) 页").font(.headline)
                                    if model.recognizing.contains(photo.id) { Text("正在本地识别…").font(.caption) }
                                    else { Text(photo.text.isEmpty ? "点开补充文字" : photo.text).font(.caption).foregroundStyle(.secondary).lineLimit(3) }
                                }
                            }
                        }.accessibilityIdentifier("notebook-photo-\(photo.id)")
                    }
                } header: { Text("实验照片") } footer: {
                    Text("文字识别在设备上完成，不上传照片。可识别的语言和手写效果取决于系统；请核对样本编号、数值、单位与公式。相册中仅存于 iCloud 的照片需先下载到设备。")
                }
                Section {
                    Button("选择内容并发送到 Wisp") {
                        if model.saveNotes(id, title: title, day: ChecklistDate.string(date), notes: notes) { workspace.notebookPath.append(.send(id)) }
                    }.accessibilityIdentifier("notebook-review-send")
                    if let transfer = record.transfer {
                        Text(transferLabel(transfer.state) + " · " + transfer.host).font(.caption).foregroundStyle(.secondary)
                    }
                }
                if let error = model.error { Section { Text(error).foregroundStyle(WispStyle.warning) } }
            }
        }.navigationTitle("实验记录").navigationBarTitleDisplayMode(.inline)
            .photosPicker(isPresented: $workspace.notebookLibraryPresented, selection: $picks,
                          maxSelectionCount: max(1, min(10, 50 - (model.record(id)?.photos.count ?? 0))),
                          selectionBehavior: .ordered, matching: .images, preferredItemEncoding: .current)
            .onAppear {
                guard !initialized, let record = model.record(id) else { return }
                title = record.title; date = ChecklistDate.date(record.day) ?? Date(); notes = record.notes; initialized = true
            }
            .onChange(of: picks) { _, selected in
                guard !selected.isEmpty else { return }
                importing = true; picks = []
                Task {
                    defer { importing = false }
                    for pick in selected {
                        do {
                            guard let data = try await pick.loadTransferable(type: Data.self) else { throw RemoteError.service("无法读取所选照片。") }
                            await model.addPhoto(data, record: id)
                        } catch { model.error = error.localizedDescription }
                    }
                }
            }
    }
    private func save() { _ = model.saveNotes(id, title: title, day: ChecklistDate.string(date), notes: notes) }
}

private struct NotebookPhotoView: View {
    @ObservedObject var model: NotebookModel
    let recordID: String
    let photoID: String
    @State private var text = ""
    @State private var saved = ""
    private var photo: NotebookPhoto? { model.record(recordID)?.photos.first { $0.id == photoID } }
    var body: some View {
        Form {
            Section("照片") { NotebookThumbnail(model: model, photoID: photoID, maxPixelSize: 1600).frame(maxWidth: .infinity).frame(minHeight: 220) }
            Section("识别文字，可校对修改") {
                TextEditor(text: $text).frame(minHeight: 240).accessibilityIdentifier("notebook-ocr-text")
                Button("保存校对文字") { if model.saveText(record: recordID, photo: photoID, text: text) { saved = text } }
                if photo?.text.isEmpty == true {
                    Button("在手机上识别文字") { Task { await model.recognize(record: recordID, photo: photoID) } }
                        .disabled(model.recognizing.contains(photoID)).accessibilityIdentifier("notebook-recognize")
                }
                if model.recognizing.contains(photoID) { ProgressView("正在识别…") }
            }
            if let error = model.error { Text(error).foregroundStyle(WispStyle.warning) }
        }.navigationTitle("照片与文字").navigationBarTitleDisplayMode(.inline)
            .onAppear { saved = photo?.text ?? ""; text = saved }
            .onChange(of: photo?.text) { _, value in if text == saved { saved = value ?? ""; text = saved } }
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
                Section("选择发送内容") {
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
                Section("目标项目与对话") {
                    if !online { Text("连接到 Wisp 主机后可选择目标。本机记录已保存，照片不会自动上传。").foregroundStyle(.secondary) }
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
                Section("发送前核对") {
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
                    sessions = result.array
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
