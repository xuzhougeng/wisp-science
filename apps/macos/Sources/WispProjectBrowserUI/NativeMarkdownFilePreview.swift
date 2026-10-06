import AppKit
import ImageIO
import SwiftUI
import WispProjectBrowser

typealias NativeFileImageLoader = (String) async throws -> NativePanelFileContent

@MainActor final class NativeFileDocumentImages: ObservableObject {
    @Published private(set) var images: [String: NSImage] = [:]
    @Published private(set) var unavailable: Set<String> = []
    private var generation = UUID()
    func clear() { generation = UUID(); images = [:]; unavailable = [] }
    func load(_ requests: [NativeMessageImageRequest], loader: NativeFileImageLoader?) async {
        clear(); let current = generation
        for request in requests {
            guard generation == current, !Task.isCancelled else { return }
            do {
                guard let loader else { throw ProjectBrowserError.invalidResponse }
                let content = try await loader(request.reference)
                guard generation == current, !Task.isCancelled else { return }
                images[request.reference] = try Self.decode(content)
            } catch {
                guard generation == current, !Task.isCancelled else { return }
                unavailable.insert(request.reference)
            }
        }
    }
    static func decode(_ content: NativePanelFileContent) throws -> NSImage {
        guard content.mime.hasPrefix("image/"), !content.truncated, let base64 = content.base64, base64.utf8.count <= 45 * 1024 * 1024,
              let bytes = Data(base64Encoded: base64), bytes.count <= 32 * 1024 * 1024,
              let source = CGImageSourceCreateWithData(bytes as CFData, [kCGImageSourceShouldCache: false] as CFDictionary),
              let metadata = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = metadata[kCGImagePropertyPixelWidth] as? NSNumber, let height = metadata[kCGImagePropertyPixelHeight] as? NSNumber,
              width.doubleValue > 0, height.doubleValue > 0, width.doubleValue <= 8192, height.doubleValue <= 8192, width.doubleValue * height.doubleValue <= 32_000_000,
              let image = CGImageSourceCreateThumbnailAtIndex(source, 0, [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: 1024, kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceShouldCacheImmediately: true] as CFDictionary) else { throw ProjectBrowserError.invalidResponse }
        return NSImage(cgImage: image, size: NSSize(width: image.width, height: image.height))
    }
}

struct NativeMarkdownFilePreview: View {
    let markdown: String
    let quote: ((String) -> Void)?
    let loadImage: NativeFileImageLoader?
    @StateObject private var images = NativeFileDocumentImages()
    @State private var preview: NativeMessageImagePreview?
    var body: some View {
        GeometryReader { geometry in
            ScrollView {
                NativeSelectableMessage(text: AttributedString(""), saved: [], quote: quote, save: nil, markdown: markdown,
                                        images: images.images, unavailableImages: images.unavailable, openImage: { reference in
                    if let image = images.images[reference] { preview = .init(reference: reference, image: image) }
                }).frame(width: max(1, geometry.size.width), alignment: .topLeading)
            }
        }.task(id: markdown) { await images.load(NativeMessageImageRequest.requests(markdown: markdown, resources: []), loader: loadImage) }
            .onDisappear { images.clear(); preview = nil }
            .sheet(item: $preview) { image in NativeMessageImageSheet(preview: image) { preview = nil } }
    }
}
