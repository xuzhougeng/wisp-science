import AppKit
import SwiftUI
import WispProjectBrowser

struct NativeMessageImageRequest: Equatable, Sendable {
    let reference: String
    let resource: ConversationMessageResource?
    let path: String?
    var available: Bool { resource.map { $0.status == "ready" && $0.artifactVersionId != nil } ?? (path != nil) }
    var label: String { resource?.displayName ?? ((path ?? reference) as NSString).lastPathComponent }

    static func localPath(_ reference: String) -> String? {
        guard let url = URL(string: reference) else { return nil }
        if url.isFileURL { return url.host == nil || url.host == "" || url.host == "localhost" ? url.path : nil }
        guard url.scheme == nil, !reference.hasPrefix("//") else { return nil }
        return reference.removingPercentEncoding ?? reference
    }
    static func isImageFile(_ path: String) -> Bool {
        ["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"].contains((path as NSString).pathExtension.lowercased())
    }
    static func requests(markdown: String, resources: [ConversationMessageResource]) -> [Self] {
        guard let parsed = try? AttributedString(markdown: NativeMathContent.prepare(markdown).markdown, options: .init(interpretedSyntax: .full)) else { return [] }
        var seen: Set<String> = []
        return parsed.runs.compactMap { run in
            guard let url = run.imageURL, seen.insert(url.absoluteString).inserted else { return nil }
            let reference = url.absoluteString
            let path = localPath(reference)
            let resource = resources.first { $0.originalReference == reference || (path != nil && localPath($0.originalReference) == path) }
            return Self(reference: reference, resource: resource, path: resource == nil ? path : nil)
        }
    }
    static func attachmentMarkdown(_ paths: [String]) -> String {
        paths.filter(isImageFile).compactMap { path in
            guard let encoded = path.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed.subtracting(CharacterSet(charactersIn: "<>$\\%"))) else { return nil }
            let name = (path as NSString).lastPathComponent.unicodeScalars.map { scalar in
                "&<>[]\\$*_`".unicodeScalars.contains(scalar) ? "&#\(scalar.value);" : String(scalar)
            }.joined()
            return "![\(name)](<\(encoded)>)"
        }.joined(separator: "\n\n")
    }
    static func generatedPath(_ item: ConversationItem) -> String? {
        guard item.role == "tool", item.tool_name == "generate_image", item.ok == true,
              let input = item.input?.trimmingCharacters(in: .whitespacesAndNewlines), !input.isEmpty,
              localPath(input) != nil, isImageFile(input) else { return nil }
        return input
    }
}

@MainActor final class NativeMessageImagesModel: ObservableObject {
    @Published private(set) var images: [String: NSImage] = [:]
    @Published private(set) var unavailable: Set<String> = []
    private var revision = UUID()
    private var loadedRequests: [String: NativeMessageImageRequest] = [:]
    private static let cache: NSCache<NSString, NSImage> = {
        let cache = NSCache<NSString, NSImage>(); cache.countLimit = 32; cache.totalCostLimit = 64 * 1024 * 1024; return cache
    }()
    func clear() { revision = UUID(); images = [:]; unavailable = []; loadedRequests = [:] }
    func load(_ requests: [NativeMessageImageRequest], client: any NativeConversationQuerying, project: String, session: String) async {
        let current = UUID(); revision = current
        let next = Dictionary(uniqueKeysWithValues: requests.map { ($0.reference, $0) })
        images = images.filter { loadedRequests[$0.key] == next[$0.key] }; unavailable = []
        loadedRequests = next
        for request in requests {
            guard revision == current, !Task.isCancelled else { return }
            guard request.available else { unavailable.insert(request.reference); continue }
            let key = "\(project):\(session):\(request.resource?.id ?? ""):\(request.resource?.artifactVersionId ?? "")" as NSString
            if request.resource != nil, let cached = Self.cache.object(forKey: key) { images[request.reference] = cached; continue }
            do {
                let args: [String: SettingsValue] = ["session_id": .string(session), "resource_id": request.resource.map { .string($0.id) } ?? .null, "path": request.path.map(SettingsValue.string) ?? .null]
                let value = try await client.invoke("native_conversation_image", args: args, projectID: project)
                guard revision == current, !Task.isCancelled else { return }
                let content = try JSONDecoder().decode(NativePanelFileContent.self, from: JSONEncoder().encode(value))
                guard content.mime == "image/png", !content.truncated, let base64 = content.base64,
                      base64.utf8.count <= 8 * 1024 * 1024, let bytes = Data(base64Encoded: base64), let image = NSImage(data: bytes), image.isValid else { throw ProjectBrowserError.invalidResponse }
                images[request.reference] = image
                if request.resource != nil { Self.cache.setObject(image, forKey: key, cost: max(bytes.count, Int(image.size.width * image.size.height * 4))) }
            } catch {
                guard revision == current, !Task.isCancelled else { return }
                unavailable.insert(request.reference)
            }
        }
    }
}

/// One selectable document owns its image attachments as well as its text.
struct NativeMessageBody: View {
    let text: AttributedString
    let markdown: String?
    let resources: [ConversationMessageResource]
    let saved: [String]
    let revealed: String?
    let monospaced: Bool
    let client: any NativeConversationQuerying
    let project: String
    let session: String
    let quote: ((String) -> Void)?
    let save: ((String) -> Void)?
    @StateObject private var model = NativeMessageImagesModel()
    @State private var preview: NativeMessageImagePreview?
    private var requests: [NativeMessageImageRequest] { NativeMessageImageRequest.requests(markdown: markdown ?? "", resources: resources) }
    var body: some View {
        NativeSelectableMessage(text: text, saved: saved, quote: quote, save: save, monospaced: monospaced, markdown: markdown, revealed: revealed,
                                images: model.images, unavailableImages: model.unavailable, openImage: { reference in
            if let image = model.images[reference] { preview = NativeMessageImagePreview(reference: reference, image: image) }
        }).frame(maxWidth: .infinity, alignment: .leading)
            .task(id: requests) { await model.load(requests, client: client, project: project, session: session) }
            .onDisappear { model.clear(); preview = nil }
            .sheet(item: $preview) { image in NativeMessageImageSheet(preview: image) { preview = nil } }
    }
}
struct NativeMessageImagePreview: Identifiable {
    var id: String { reference }
    let reference: String
    let image: NSImage
}
struct NativeMessageImageSheet: View {
    let preview: NativeMessageImagePreview
    let close: () -> Void
    var body: some View {
        VStack(spacing: 12) {
            HStack {
                Text(((NativeMessageImageRequest.localPath(preview.reference) ?? preview.reference) as NSString).lastPathComponent).lineLimit(1).truncationMode(.middle).textSelection(.enabled)
                Spacer()
                Button(action: close) { WispIcon(name: "close", size: 16) }.buttonStyle(.plain).accessibilityLabel(localized("关闭图片预览"))
            }
            Image(nsImage: preview.image).resizable().scaledToFit().accessibilityLabel(localized("消息图片"))
        }.padding(20).frame(minWidth: 360, idealWidth: 760, maxWidth: 1000, minHeight: 300, idealHeight: 560, maxHeight: 760)
            .background(NativeSettingsEscape(close: close))
    }
}

enum NativeImageContent {
    static let referenceKey = NSAttributedString.Key("WispImageReference")
    static let previewKey = NSAttributedString.Key("WispImagePreview")
    static func replace(in value: NSMutableAttributedString, images: [String: NSImage], unavailable: Set<String>, width: CGFloat) {
        var ranges: [(String, NSRange)] = []
        value.enumerateAttribute(referenceKey, in: NSRange(location: 0, length: value.length)) { reference, range, _ in
            if let reference = reference as? String { ranges.append((reference, range)) }
        }
        for (reference, range) in ranges.reversed() {
            var attributes = value.attributes(at: range.location, effectiveRange: nil)
            attributes.removeValue(forKey: referenceKey)
            attributes.removeValue(forKey: .link)
            let alt = (value.string as NSString).substring(with: range).replacingOccurrences(of: "\u{fffc}", with: "")
            let replacement: NSMutableAttributedString
            if let image = images[reference] {
                let attachment = NSTextAttachment(); attachment.attachmentCell = NativeMessageImageCell(image: image, width: width)
                replacement = NSMutableAttributedString(attributedString: NSAttributedString(attachment: attachment))
                replacement.addAttributes(attributes, range: NSRange(location: 0, length: 1))
                replacement.addAttribute(previewKey, value: reference, range: NSRange(location: 0, length: 1))
            } else {
                let label = alt.isEmpty ? (NativeMessageImageRequest.localPath(reference) ?? reference) : alt
                replacement = NSMutableAttributedString(string: label + " · " + localized(unavailable.contains(reference) ? "图片不可用" : "正在载入图片…"), attributes: attributes)
            }
            replacement.addAttribute(NativeMathContent.sourceKey, value: alt, range: NSRange(location: 0, length: replacement.length))
            value.replaceCharacters(in: range, with: replacement)
        }
    }
}
final class NativeMessageImageCell: NSTextAttachmentCell {
    private let previewImage: NSImage
    private let size: NSSize
    init(image: NSImage, width: CGFloat) {
        previewImage = image
        let scale = min(1, max(1, width - 32) / max(1, image.size.width), 320 / max(1, image.size.height))
        size = NSSize(width: max(1, image.size.width * scale), height: max(1, image.size.height * scale))
        super.init(imageCell: image)
    }
    required init(coder: NSCoder) { fatalError("init(coder:) is not supported") }
    override func cellSize() -> NSSize { size }
    override func draw(withFrame frame: NSRect, in controlView: NSView?) {
        previewImage.draw(in: frame, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
    }
}
