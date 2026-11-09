import AppKit
import SwiftUI
import WispProjectBrowser

enum NativeSharePagination {
    /// Boundaries fall only on blank scanlines, so text baselines are never cut.
    static func ranges(height: Int, pageHeight: Int, blank: (Int) -> Bool) throws -> [Range<Int>] {
        guard height > 0, pageHeight > 0 else { throw ProjectBrowserError.invalidResponse }
        var start = 0, result: [Range<Int>] = []
        while start < height {
            var end = min(height, start + pageHeight)
            if end < height {
                while end > start && !blank(end) { end -= 1 }
                guard end > start else { throw ProjectBrowserError.unavailable(localized("单个图像或内容块超过分页高度，请减少图片宽度或导出 HTML。")) }
            }
            result.append(start..<end); start = end
        }
        return result
    }
    @MainActor static func pages(rows: [NativeShareRow], width: Int, scheme: ColorScheme) throws -> [Data] {
        let data = try NativeSharePage.png(rows: rows, width: width, scheme: scheme)
        guard let image = NSBitmapImageRep(data: data)?.cgImage else { throw ProjectBrowserError.invalidResponse }
        return try render(image: image, scheme: scheme).map(\.png)
    }
    struct Page {
        let png: Data
        let source: Range<Int>
    }
    @MainActor static func render(image: CGImage, scheme: ColorScheme) throws -> [Page] {
        let width = image.width
        let pageHeight = width * 4 / 3
        let space = CGColorSpaceCreateDeviceRGB()
        var pixels = [UInt8](repeating: 0, count: image.width * image.height * 4)
        let rendered = pixels.withUnsafeMutableBytes { bytes -> Bool in
            guard let context = CGContext(data: bytes.baseAddress, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: image.width * 4, space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height)); return true
        }
        guard rendered else { throw ProjectBrowserError.invalidResponse }
        let backgrounds = ["bg-app", "bg-sunken", "bg-elev"].compactMap { token -> [Int]? in
            guard let color = NSColor(WispDesign.color(token, scheme)).usingColorSpace(.deviceRGB) else { return nil }
            return [Int(color.redComponent * 255), Int(color.greenComponent * 255), Int(color.blueComponent * 255)]
        }
        let spans = try ranges(height: image.height, pageHeight: pageHeight - 32) { y in
            // Ignore the outer margin. Any non-background pixel is content.
            (16..<max(17, image.width - 16)).allSatisfy { x in
                let offset = (y * image.width + x) * 4
                return backgrounds.contains { color in (0..<3).allSatisfy { abs(Int(pixels[offset + $0]) - color[$0]) <= 8 } }
            }
        }
        return try spans.map { span in
            guard let crop = image.cropping(to: CGRect(x: 0, y: span.lowerBound, width: image.width, height: span.count)),
                  let context = CGContext(data: nil, width: width, height: pageHeight, bitsPerComponent: 8, bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { throw ProjectBrowserError.invalidResponse }
            context.setFillColor(NSColor(WispDesign.color("bg-app", scheme)).cgColor)
            context.fill(CGRect(x: 0, y: 0, width: width, height: pageHeight))
            context.draw(crop, in: CGRect(x: 0, y: pageHeight - span.count - 16, width: width, height: span.count))
            guard let page = context.makeImage(), let png = NSBitmapImageRep(cgImage: page).representation(using: .png, properties: [:]) else { throw ProjectBrowserError.invalidResponse }
            return Page(png: png, source: span)
        }
    }
}
