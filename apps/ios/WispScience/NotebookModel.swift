import Foundation
import Combine
import UIKit
import Vision
import ImageIO
import WispRemote

enum NotebookDestination: Hashable {
    case create(NotebookEntry)
    case guide, example
    case record(String)
    case photo(String, String)
    case saved(String)
    case send(String)
}

enum NotebookEntry: Hashable { case observation, photos }

@MainActor final class NotebookModel: ObservableObject {
    private struct Document: Codable { var version = 1; var records: [NotebookRecord] = [] }
    @Published private var document = Document()
    @Published private(set) var loaded = false
    @Published private(set) var recognizing: Set<String> = []
    @Published private(set) var sending = false
    @Published var error: String?
    let directory: URL
    private var file: URL { directory.appendingPathComponent("notebook.json") }
    private let recognizeText: (Data) async throws -> String
    private let thumbnails = NSCache<NSString, UIImage>()
    var records: [NotebookRecord] { document.records.sorted { ($0.day, $0.id) > ($1.day, $1.id) } }

    init(directory: URL = OfflineResearchStorage.directory,
         recognizeText: @escaping (Data) async throws -> String = NotebookOCR.recognize) {
        self.directory = directory; self.recognizeText = recognizeText
        thumbnails.totalCostLimit = 32 * 1024 * 1024
        do {
            if var saved = try OfflineResearchStorage.load(Document.self, from: file) {
                guard saved.version == 1,
                      Set(saved.records.map(\.id)).count == saved.records.count,
                      Set(saved.records.flatMap { $0.photos.map(\.id) }).count == saved.records.reduce(0, { $0 + $1.photos.count }),
                      saved.records.allSatisfy({ UUID(uuidString: $0.id) != nil && $0.photos.allSatisfy { UUID(uuidString: $0.id) != nil } }) else {
                    throw RemoteError.incompatibleResponse
                }
                // A process exit after handing a message to the transport cannot
                // prove delivery. Leave it for the user to check on the host.
                for index in saved.records.indices {
                    if saved.records[index].transfer?.state == "sending" { saved.records[index].transfer?.state = "uncertain" }
                    if saved.records[index].transfer?.state == "uploading" { saved.records[index].transfer?.state = "failed" }
                }
                document = saved
            }
            loaded = true
        } catch { self.error = "无法读取本机实验记录，原文件已保留。解锁后重开 App。\n" + error.localizedDescription }
    }

    func record(_ id: String) -> NotebookRecord? { document.records.first { $0.id == id } }
    private func save(_ next: Document) throws {
        guard loaded else { throw RemoteError.service("实验记录尚未成功读取，无法覆盖保存。") }
        try OfflineResearchStorage.save(next, to: file); document = next
    }
    private func update(_ id: String, _ mutation: (inout NotebookRecord) throws -> Void) throws {
        var next = document
        guard let index = next.records.firstIndex(where: { $0.id == id }) else { throw RemoteError.service("实验记录不存在。") }
        try mutation(&next.records[index]); try save(next)
    }
    func add(title: String, day: String) -> String? {
        let title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        guard title.count <= 200, ChecklistFields(title: title, day: day).isValid else { error = "请填写 1–200 字的实验标题及有效日期。"; return nil }
        let record = NotebookRecord(title: title, day: day)
        var next = document; next.records.append(record)
        do { try save(next); error = nil; return record.id } catch { self.error = error.localizedDescription; return nil }
    }
    /// Keep an untitled first observation useful without creating empty records
    /// when someone merely opens the editor or cancels the photo picker.
    func createObservation(title: String, day: String, notes: String) -> String? {
        do {
            guard !title.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !notes.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
                throw RemoteError.service("先写下一句观察，或添加一张照片。")
            }
            let record = try draft(title: title, day: day, notes: notes)
            var next = document; next.records.append(record)
            try save(next); error = nil; return record.id
        } catch { self.error = error.localizedDescription; return nil }
    }
    private func draft(title: String, day: String, notes: String) throws -> NotebookRecord {
        var title = title.trimmingCharacters(in: .whitespacesAndNewlines)
        if title.isEmpty {
            let firstLine = notes.trimmingCharacters(in: .whitespacesAndNewlines).split(whereSeparator: \.isNewline).first.map(String.init) ?? ""
            title = String(firstLine.prefix(40))
            if title.isEmpty { title = "实验记录 · \(day)" }
        }
        guard title.count <= 200, ChecklistFields(title: title, day: day).isValid, notes.utf8.count <= 256 * 1024 else {
            throw RemoteError.service("标题最多 200 字，记录文字最多 256 KB，请检查实验日期。")
        }
        return NotebookRecord(title: title, day: day, notes: notes)
    }
    func createFromPhoto(_ data: Data, title: String, day: String, notes: String) async -> (record: String, photo: String)? {
        do {
            var record = try draft(title: title, day: day, notes: notes)
            let photo = try storePhoto(data)
            record.photos.append(photo)
            var next = document; next.records.append(record)
            try save(next); error = nil
            await recognize(record: record.id, photo: photo.id)
            return (record.id, photo.id)
        } catch { self.error = error.localizedDescription; return nil }
    }
    @discardableResult func saveNotes(_ id: String, title: String, day: String, notes: String) -> Bool {
        do {
            guard ChecklistFields(title: title, day: day).isValid, title.count <= 200, notes.utf8.count <= 256 * 1024 else {
                throw RemoteError.service("请填写实验标题，记录文字最多 256 KB。")
            }
            try update(id) { $0.title = title; $0.day = day; $0.notes = notes }; error = nil; return true
        } catch { self.error = error.localizedDescription; return false }
    }
    func photoURL(_ id: String) throws -> URL {
        guard UUID(uuidString: id) != nil else { throw RemoteError.incompatibleResponse }
        return directory.appendingPathComponent("photos", isDirectory: true).appendingPathComponent(id)
    }
    func photoData(_ id: String) throws -> Data { try Data(contentsOf: photoURL(id)) }
    func thumbnail(_ id: String, maxPixelSize: Int = 320) -> UIImage? {
        let key = "\(id):\(maxPixelSize)" as NSString
        if let cached = thumbnails.object(forKey: key) { return cached }
        guard let url = try? photoURL(id), let source = CGImageSourceCreateWithURL(url as CFURL, nil),
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [
                kCGImageSourceCreateThumbnailFromImageAlways: true,
                kCGImageSourceCreateThumbnailWithTransform: true,
                kCGImageSourceThumbnailMaxPixelSize: maxPixelSize
              ] as CFDictionary) else { return nil }
        let result = UIImage(cgImage: image)
        thumbnails.setObject(result, forKey: key, cost: image.bytesPerRow * image.height)
        return result
    }

    private func storePhoto(_ data: Data) throws -> NotebookPhoto {
        guard loaded else { throw RemoteError.service("实验记录尚未成功读取，无法覆盖保存。") }
        guard data.count <= 40 * 1024 * 1024, let source = CGImageSourceCreateWithData(data as CFData, nil),
              CGImageSourceGetCount(source) > 0 else { throw RemoteError.service("请选择不超过 40 MB 的有效照片。") }
        let photo = NotebookPhoto()
        // Persist original bytes first, then the manifest. An interrupted
        // import can leave an orphan file but never a reference to lost data.
        try OfflineResearchStorage.write(data, to: photoURL(photo.id))
        return photo
    }
    @discardableResult func addPhoto(_ data: Data, record id: String) async -> String? {
        do {
            guard let record = record(id), record.photos.count < 50 else { throw RemoteError.service("每条实验记录最多保存 50 张照片。") }
            let photo = try storePhoto(data)
            try update(id) { $0.photos.append(photo) }
            error = nil
            await recognize(record: id, photo: photo.id)
            return photo.id
        } catch { self.error = error.localizedDescription; return nil }
    }
    func recognize(record id: String, photo photoID: String) async {
        guard !recognizing.contains(photoID), let photo = record(id)?.photos.first(where: { $0.id == photoID }) else { return }
        guard photo.text.isEmpty else { error = "已有识别或编辑文字；为保留修改，不会自动覆盖。"; return }
        recognizing.insert(photoID); defer { recognizing.remove(photoID) }
        do {
            let text = try await recognizeText(photoData(photoID))
            try update(id) { record in
                guard let index = record.photos.firstIndex(where: { $0.id == photoID }), record.photos[index].text.isEmpty else { return }
                record.photos[index].text = text; record.photos[index].recognizedAt = Date()
            }
            if text.isEmpty { error = "未识别到文字，原图已保存。可以手动补充。" }
        } catch { self.error = "原图已保存在手机，文字识别未完成。\n" + error.localizedDescription }
    }
    @discardableResult func saveText(record id: String, photo: String, text: String) -> Bool {
        do {
            guard text.utf8.count <= 256 * 1024 else { throw RemoteError.service("单张照片的文字最多 256 KB。") }
            try update(id) { record in
                guard let index = record.photos.firstIndex(where: { $0.id == photo }) else { throw RemoteError.incompatibleResponse }
                record.photos[index].text = text
            }
            error = nil; return true
        } catch { self.error = error.localizedDescription; return false }
    }
    func acknowledgeUnknown(_ id: String) {
        guard record(id)?.transfer?.state == "uncertain" else { return }
        do { try update(id) { $0.transfer?.state = "failed" }; error = nil } catch { self.error = error.localizedDescription }
    }

    func send(record id: String, selected: Set<String>, includeNotes: Bool, includeText: Bool,
              route: ConversationRoute, scope: String, hostName: String, connection: RemoteConnection,
              expectedSession: UUID) async {
        guard !sending, connection.state == .online, let record = record(id),
              connection.readSessionID == expectedSession,
              !["sending", "uncertain"].contains(record.transfer?.state ?? "") else { return }
        let session = connection.readSessionID
        sending = true; defer { sending = false }
        var attemptedSend = false
        var startedTransfer: String?
        do {
            let message = try record.message(selected: selected, includeNotes: includeNotes, includeText: includeText)
            let photoIDs = record.photos.filter { selected.contains($0.id) }.map(\.id)
            let signature = RemoteReadCache.key(scope: scope, command: "notebook-send", project: route.project,
                args: ["session": .string(route.session), "message": .string(message), "photos": .array(photoIDs.map(RemoteValue.string))])
            guard record.transfer?.signature != signature || record.transfer?.state != "sent" else {
                error = "这些内容已发送，可在目标对话中查看。"; return
            }
            let snapshot = try RemoteSnapshot(try await connection.call("native_conversation_snapshot", project: route.project, args: route.args), route: route)
            guard connection.readSessionID == session, snapshot.writable, !snapshot.running else {
                throw RemoteError.service("目标对话不可写、正在运行或连接已改变，请重新选择。")
            }
            let transfer = NotebookTransfer(id: UUID().uuidString, signature: signature, host: hostName, project: route.project,
                session: route.session, photoIDs: photoIDs, state: "uploading")
            try update(id) { $0.transfer = transfer }
            startedTransfer = transfer.id
            var paths: [RemoteValue] = []
            for photo in photoIDs {
                guard connection.readSessionID == session else { throw RemoteError.disconnected }
                let jpeg = try PhotoPreparation.jpeg(photoData(photo))
                let result = try await connection.call("remote_attach_image", project: route.project,
                    args: route.args.merging(["base64": .string(jpeg.base64EncodedString())]) { _, value in value })
                guard connection.readSessionID == session, !result["path"].string.isEmpty else { throw RemoteError.disconnected }
                paths.append(result["path"])
            }
            guard connection.readSessionID == session else { throw RemoteError.disconnected }
            try update(id) { $0.transfer?.state = "sending" }
            attemptedSend = true
            _ = try await connection.call("native_conversation_send", project: route.project,
                args: route.args.merging(["request_id": .string(transfer.id), "message": .string(message), "attachments": .array(paths)]) { _, value in value })
            try update(id) { $0.transfer?.state = "sent" }; error = nil
        } catch {
            // After a send was attempted, any failure (including a failed local
            // acknowledgement write) leaves delivery uncertain, never auto-retried.
            if let startedTransfer {
                try? update(id) {
                    if $0.transfer?.id == startedTransfer { $0.transfer?.state = attemptedSend ? "uncertain" : "failed" }
                }
            }
            self.error = (attemptedSend ? "发送结果待确认，请在主机核对后再操作。\n" : "尚未发送到对话，本机原始记录已保留。\n") + error.localizedDescription
        }
    }
}

enum NotebookOCR {
    static func recognize(_ data: Data) async throws -> String {
        try await Task.detached(priority: .utility) {
            let request = VNRecognizeTextRequest()
            request.recognitionLevel = .accurate
            request.usesLanguageCorrection = false // Preserve scientific identifiers and units.
            let supported = try request.supportedRecognitionLanguages()
            request.recognitionLanguages = ["zh-Hans", "zh-Hant", "en-US"].filter { supported.contains($0) }
            request.automaticallyDetectsLanguage = true
            try VNImageRequestHandler(data: data, options: [:]).perform([request])
            return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: "\n")
        }.value
    }
}
