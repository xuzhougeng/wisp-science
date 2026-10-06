import AppKit
import SwiftUI
import XCTest
import WispProjectBrowser
@testable import WispProjectBrowserUI

private actor MessageImageClient: NativeConversationQuerying {
    var held: CheckedContinuation<SettingsValue, Never>?
    var shouldHold = false
    var calls: [(String, [String: SettingsValue], String)] = []
    let reply: SettingsValue
    init(reply: SettingsValue) { self.reply = reply }
    func hold() { shouldHold = true }
    func isHeld() -> Bool { held != nil }
    func finish() { held?.resume(returning: reply); held = nil }
    func recorded() -> [(String, [String: SettingsValue], String)] { calls }
    func snapshot(projectID: String, sessionID: String, beforeSeq: Int64?) async throws -> ConversationSnapshot { throw ProjectBrowserError.invalidResponse }
    func invoke(_ command: String, args: [String: SettingsValue], projectID: String) async throws -> SettingsValue {
        calls.append((command, args, projectID))
        if shouldHold { shouldHold = false; return await withCheckedContinuation { held = $0 } }
        return reply
    }
}
final class NativeMessageImagesTests: XCTestCase {
    @MainActor private func image() -> NSImage {
        let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: 600, pixelsHigh: 300, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
        NSGraphicsContext.saveGraphicsState(); NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
        NSColor(calibratedRed: 0.72, green: 0.36, blue: 0.21, alpha: 1).setFill(); NSRect(x: 0, y: 0, width: 600, height: 300).fill()
        NSColor.white.setFill(); NSRect(x: 60, y: 40, width: 70, height: 80).fill(); NSRect(x: 180, y: 40, width: 70, height: 170).fill(); NSRect(x: 300, y: 40, width: 70, height: 110).fill()
        NSGraphicsContext.restoreGraphicsState()
        let image = NSImage(size: NSSize(width: 600, height: 300)); image.addRepresentation(bitmap); return image
    }
    private func resource(status: String = "ready", version: String = "v1") throws -> ConversationMessageResource {
        try JSONDecoder().decode(ConversationMessageResource.self, from: JSONEncoder().encode(SettingsValue.object([
            "id": .string("r"), "ordinal": .integer(0), "originalReference": .string("figures/图 1.png"),
            "artifactId": .string("a"), "artifactVersionId": .string(version), "displayName": .string("图 1.png"),
            "kind": .string("image"), "mimeType": .string("image/png"), "status": .string(status)
        ])))
    }
    @MainActor func testMarkdownImagesKeepCopyHighlightAndRealPreviewButtons() throws {
        let source = "Before **text**.\n\n![**图 1**](<figures/图 1.png>)\n\nAfter $x^2$.\n\n`![code](code.png)` and [link](link.png).\n\n![](empty.png)"
        let requests = NativeMessageImageRequest.requests(markdown: source, resources: [try resource()])
        XCTAssertEqual(requests.count, 2)
        let key = requests[0].reference
        let images = [key: image(), "empty.png": image()]
        let content = NativeMarkdownContent.render(source, saved: ["图 1 After"], scheme: .light, width: 300, images: images)
        XCTAssertEqual(NativeMathContent.plainText(content), "Before text.\n图 1\nAfter $x^2$.\n![code](code.png) and link.\n\n")
        var imageRanges: [NSRange] = []
        content.enumerateAttribute(NativeImageContent.previewKey, in: NSRange(location: 0, length: content.length)) { value, range, _ in if value != nil { imageRanges.append(range) } }
        XCTAssertEqual(imageRanges.count, 2)
        XCTAssertNotNil(content.attribute(.underlineStyle, at: try XCTUnwrap(imageRanges.first).location, effectiveRange: nil))
        for range in imageRanges {
            let cell = try XCTUnwrap((content.attribute(.attachment, at: range.location, effectiveRange: nil) as? NSTextAttachment)?.attachmentCell as? NativeMessageImageCell)
            XCTAssertLessThanOrEqual(cell.cellSize().width, 268); XCTAssertEqual(cell.cellSize().width / cell.cellSize().height, 2)
        }
        let view = NativeMessageTextView(frame: NSRect(x: 0, y: 0, width: 300, height: 1000))
        view.textContainer?.containerSize = NSSize(width: 300, height: CGFloat.greatestFiniteMagnitude)
        view.apply(content); view.layoutCopyButtons()
        var opened: [String] = []; view.openImage = { opened.append($0) }
        let buttons = view.subviews.compactMap { $0 as? NSButton }.filter { $0.identifier?.rawValue.hasPrefix("message-image-") == true }
        XCTAssertEqual(buttons.count, 2); XCTAssertTrue(buttons.allSatisfy { view.bounds.contains($0.frame) && $0.frame.height > 0 })
        buttons.forEach { $0.performClick(nil) }; XCTAssertEqual(Set(opened), Set(images.keys))
        view.setSelectedRange(NSRange(location: 0, length: content.length)); var copied = ""; view.copyBlock = { copied = $0 }; view.copy(nil)
        XCTAssertEqual(copied, NativeMathContent.plainText(content)); XCTAssertFalse(copied.contains("\u{fffc}"))
        view.apply(NSAttributedString(string: "new message")); view.layoutCopyButtons(); XCTAssertTrue(view.subviews.isEmpty)
    }
    @MainActor func testBindingsAndLocalPathsAvoidRemoteReadsAndLateResults() async throws {
        let bitmap = try XCTUnwrap(image().representations.first as? NSBitmapImageRep)
        let bytes = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        let client = MessageImageClient(reply: .object(["path": .string("snapshot"), "mime": .string("image/png"), "base64": .string(bytes.base64EncodedString()), "truncated": .bool(false)]))
        let requests = NativeMessageImageRequest.requests(markdown: "![bound](<figures/图 1.png>) ![remote](https://example.com/a.png) ![local](uploads/photo.jpg)", resources: [try resource()])
        XCTAssertEqual(requests.count, 3); XCTAssertEqual(requests[0].resource?.artifactVersionId, "v1"); XCTAssertNil(requests[0].path)
        let model = NativeMessageImagesModel()
        await model.load(requests, client: client, project: "p", session: "s")
        let calls = await client.recorded(); XCTAssertEqual(calls.count, 2)
        XCTAssertEqual(calls[0].1["resource_id"], .string("r")); XCTAssertEqual(calls[0].1["session_id"], .string("s")); XCTAssertEqual(calls[0].2, "p")
        XCTAssertEqual(calls[1].1["path"], .string("uploads/photo.jpg")); XCTAssertEqual(model.images.count, 2); XCTAssertTrue(model.unavailable.contains(requests[1].reference))
        let failed = NativeMessageImageRequest.requests(markdown: "![bound](<figures/图 1.png>)", resources: [try resource(status: "failed", version: "v2")])
        await model.load(failed, client: client, project: "p", session: "s")
        XCTAssertTrue(model.images.isEmpty, "A failed binding must not keep an old image or fall back to the path")
        let after = await client.recorded(); XCTAssertEqual(after.count, 2)
        let late = NativeMessageImageRequest.requests(markdown: "![late](late.png)", resources: [])
        await client.hold(); let loading = Task { await model.load(late, client: client, project: "old-project", session: "old-session") }
        while !(await client.isHeld()) { await Task.yield() }
        model.clear(); await client.finish(); await loading.value
        XCTAssertTrue(model.images.isEmpty); XCTAssertTrue(model.unavailable.isEmpty)
    }
    func testAttachmentEscapingAndCodeDoNotCreateSpuriousReads() {
        let source = NativeMessageImageRequest.attachmentMarkdown(["uploads/a [1] #.png", "table.csv", "uploads/photo.JPG", "uploads/100%20.png"])
        let requests = NativeMessageImageRequest.requests(markdown: source + "\n\n```\n![no](fake.png)\n```\n\n[link](link.png)", resources: [])
        XCTAssertEqual(requests.map(\.path), ["uploads/a [1] #.png", "uploads/photo.JPG", "uploads/100%20.png"])
        XCTAssertNil(NativeMessageImageRequest.localPath("//example.com/file.png")); XCTAssertNil(NativeMessageImageRequest.localPath("data:image/png;base64,a"))
        XCTAssertEqual(NativeMessageImageRequest.localPath("file:///tmp/a%20b.png"), "/tmp/a b.png")
        XCTAssertEqual(NativeMessageImageRequest.requests(markdown: "![cost](<plots/$value$.png>)", resources: []).map(\.path), ["plots/$value$.png"])
        XCTAssertTrue(NativeMathContent.prepare("![caption \\[1\\]](<plots/$value$.png>)").formulas.isEmpty)
    }
    func testGeneratedImagesAppearOnlyForCompletedImageTools() throws {
        func item(_ name: String, _ ok: Bool, _ input: String) throws -> ConversationItem {
            try JSONDecoder().decode(ConversationItem.self, from: JSONEncoder().encode(SettingsValue.object(["role": .string("tool"), "text": .string("saved"), "tool_name": .string(name), "ok": .bool(ok), "input": .string(input)])))
        }
        XCTAssertEqual(NativeMessageImageRequest.generatedPath(try item("generate_image", true, "figures/counts.png")), "figures/counts.png")
        XCTAssertNil(NativeMessageImageRequest.generatedPath(try item("generate_image", false, "figures/counts.png")))
        XCTAssertNil(NativeMessageImageRequest.generatedPath(try item("shell", true, "figures/counts.png")))
        XCTAssertNil(NativeMessageImageRequest.generatedPath(try item("generate_image", true, "https://example.com/counts.png")))
    }
    @MainActor func testLoadingImagePreservesExistingSelection() throws {
        let view = NativeMessageTextView(frame: .zero)
        let source = "Before ![figure](plot.png) after text."
        view.apply(NativeMarkdownContent.render(source, saved: [], scheme: .light))
        view.setSelectedRange((view.string as NSString).range(of: "after text"))
        view.apply(NativeMarkdownContent.render(source, saved: [], scheme: .light, images: ["plot.png": image()]))
        var copied = ""; view.copyBlock = { copied = $0 }; view.copy(nil)
        XCTAssertEqual(copied, "after text")
    }
    @MainActor func testImmediateEscapeDismissesImageBeforeItsParent() throws {
        _ = NSApplication.shared
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 700, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false; defer { window.close() }
        let parentView = NSView(frame: window.contentView!.bounds); window.contentView = parentView
        var parentClosed = 0, imageClosed = 0
        let parent = NativeSettingsEscape.Coordinator(enabled: true) { parentClosed += 1 }; parent.view = parentView; parent.install(); defer { parent.remove() }
        let hosted = NSHostingView(rootView: NativeMessageImageSheet(preview: .init(reference: "plot.png", image: image())) { imageClosed += 1 })
        hosted.frame = parentView.bounds; parentView.addSubview(hosted); hosted.layoutSubtreeIfNeeded()
        let focus = window.firstResponder
        let escape = NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0, windowNumber: window.windowNumber, context: nil, characters: "\u{1b}", charactersIgnoringModifiers: "\u{1b}", isARepeat: false, keyCode: 53)!
        XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil)); XCTAssertEqual(imageClosed, 1); XCTAssertEqual(parentClosed, 0); XCTAssertTrue(window.firstResponder === focus)
        hosted.removeFromSuperview(); XCTAssertTrue(NativeEscapeStack.shared.consume(escape, keyWindow: window, modalWindow: nil)); XCTAssertEqual(parentClosed, 1)
    }
    @MainActor func testRenderInlineImages() throws {
        guard let directory = ProcessInfo.processInfo.environment["WISP_NATIVE_SNAPSHOT_DIR"] else { throw XCTSkip("Opt-in rendering") }
        let source = "# Sample comparison\n\n**Figure 1.** Counts remain visible in the message.\n\n![Counts by sample](plot.png)\n\nAfter the figure: $x^2 + y^2$.\n\n![Missing figure](missing.png)\n\n```python\nprint('next step')\n```"
        for (name, scheme, width) in [("message-images-light", ColorScheme.light, 720.0), ("message-images-dark-narrow", ColorScheme.dark, 310.0)] {
            let root = NativeSelectableMessage(text: AttributedString(""), saved: ["Counts by sample"], quote: nil, save: nil, markdown: source, images: ["plot.png": image()], unavailableImages: ["missing.png"], openImage: { _ in })
                .padding(20).frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading).background(WispDesign.color("bg-app", scheme)).environment(\.colorScheme, scheme)
            let view = NSHostingView(rootView: root); view.frame = NSRect(x: 0, y: 0, width: width, height: 860); view.layoutSubtreeIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds)); view.cacheDisplay(in: view.bounds, to: bitmap)
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:])).write(to: URL(fileURLWithPath: directory).appendingPathComponent(name + ".png"))
        }
    }
}
