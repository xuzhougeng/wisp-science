import Foundation

public struct NotebookPhoto: Codable, Identifiable, Sendable {
    public let id: String
    public var text: String
    public var recognizedAt: Date?
    public init(id: String = UUID().uuidString.lowercased(), text: String = "") { self.id = id; self.text = text }
}

public struct NotebookTransfer: Codable, Sendable {
    public let id: String
    public let signature: String
    public let host: String
    public let project: String
    public let session: String
    public let photoIDs: [String]
    /// uploading, sending, sent, failed, uncertain; never automatically replayed.
    public var state: String
    public let date: Date
    public init(id: String, signature: String, host: String, project: String, session: String, photoIDs: [String], state: String) {
        self.id = id; self.signature = signature; self.host = host; self.project = project; self.session = session
        self.photoIDs = photoIDs; self.state = state; date = Date()
    }
}

public struct NotebookRecord: Codable, Identifiable, Sendable {
    public let id: String
    public var title: String
    public var day: String
    public var notes: String
    public var photos: [NotebookPhoto]
    public var transfer: NotebookTransfer?
    public init(id: String = UUID().uuidString.lowercased(), title: String, day: String, notes: String = "") {
        self.id = id; self.title = title; self.day = day; self.notes = notes; photos = []
    }

    public func message(selected: Set<String>, includeNotes: Bool, includeText: Bool) throws -> String {
        guard selected.isSubset(of: Set(photos.map(\.id))), selected.count <= 4,
              (includeNotes && !notes.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty) || !selected.isEmpty else { throw RemoteError.service("请选择记录文字或最多 4 张照片。") }
        var parts = ["请根据以下手机实验记录整理实验过程；区分原始记录与推断，不补造样本编号、数值或结果。", "实验：\(title)\n日期：\(day)"]
        if includeNotes && !notes.isEmpty { parts.append("记录文字：\n" + notes) }
        for (index, photo) in photos.filter({ selected.contains($0.id) }).enumerated() {
            if includeText && !photo.text.isEmpty { parts.append("照片 \(index + 1) 的文字（可能含识别错误，请结合原图）：\n" + photo.text) }
        }
        let message = parts.joined(separator: "\n\n")
        guard message.utf8.count <= 64 * 1024 else { throw RemoteError.service("所选文字超过 64 KB，请精简后再发送。") }
        return message
    }
}
