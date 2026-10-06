import AppKit
import SwiftUI
import UniformTypeIdentifiers
import WispProjectBrowser

enum NativeFileDropSources {
    static func url(_ value: NSSecureCoding?) throws -> URL {
        let url: URL?
        if let value = value as? URL { url = value }
        else if let value = value as? Data { url = URL(dataRepresentation: value, relativeTo: nil) }
        else if let value = value as? String { url = URL(string: value) }
        else { url = nil }
        guard let url, url.isFileURL, url.path.hasPrefix("/"), !url.path.contains("\0") else { throw ProjectBrowserError.unavailable("拖放内容必须是本地文件或文件夹。") }
        return url
    }
    static func paths(_ providers: [NSItemProvider]) async throws -> [String] {
        var paths: [String] = []
        for provider in providers {
            guard !Task.isCancelled, provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) else { throw CancellationError() }
            let path: String = try await withCheckedThrowingContinuation { continuation in
                provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { value, error in
                    do {
                        if let error { throw error }
                        continuation.resume(returning: try url(value).path)
                    } catch { continuation.resume(throwing: error) }
                }
            }
            if !paths.contains(path) { paths.append(path) }
        }
        try Task.checkCancellation()
        return paths
    }
}

/// Panels belong to the Files window that opened them. Their callbacks retain
/// the captured location and never look up a newly focused window/session.
@MainActor final class NativeFilesChooser: ObservableObject {
    weak var window: NSWindow?
    @Published private(set) var busy = false
    private(set) var panel: NSSavePanel?
    private weak var panelWindow: NSWindow?
    private var escapeMonitor: Any?
    private func watchEscape(_ picker: NSSavePanel, window: NSWindow) {
        panelWindow = window
        escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            self?.consume(event) == true ? nil : event
        }
    }
    /// Native sheets take precedence over the Files Escape stack, including
    /// before the system file browser has installed its field editor.
    func consume(_ event: NSEvent) -> Bool {
        guard event.type == .keyDown, event.keyCode == 53, let panel,
              NativeEscapeStack.shared.menuDepth == 0, panel.attachedSheet == nil,
              NSApp.modalWindow == nil || NSApp.modalWindow === panel,
              event.window === panel || event.window === panelWindow && panelWindow?.attachedSheet === panel else { return false }
        if let editor = panel.firstResponder as? NSTextInputClient, editor.hasMarkedText() { return false }
        if !event.isARepeat { panel.cancel(nil) }
        return true
    }
    private func finish() {
        if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) }
        escapeMonitor = nil; panelWindow = nil; panel = nil; busy = false
    }
    func owns(_ origin: NSWindow) -> Bool { window === origin }
    func upload(_ model: NativeFilesModel) {
        guard !busy, let window, window.attachedSheet == nil, let target = model.transferTarget(upload: true) else { return }
        let picker = NSOpenPanel()
        picker.canChooseFiles = true; picker.canChooseDirectories = !model.local; picker.allowsMultipleSelection = true
        picker.prompt = localized("上传"); panel = picker; busy = true; watchEscape(picker, window: window)
        picker.beginSheetModal(for: window) { [weak self, weak model, weak window] response in
            guard let self, self.panel === picker else { return }
            self.finish()
            guard response == .OK, let window, self.owns(window), let model, model.owns(target) else { return }
            let sources = picker.urls.map(\.path)
            Task { await model.upload(sources, target: target) }
        }
    }
    func download(_ row: NativeFileBrowserRow, model: NativeFilesModel) {
        guard !busy, let window, window.attachedSheet == nil, let target = model.transferTarget(upload: false), !row.directory else { return }
        let picker = NSSavePanel(); picker.nameFieldStringValue = row.name; picker.prompt = localized("保存副本")
        panel = picker; busy = true; watchEscape(picker, window: window)
        picker.beginSheetModal(for: window) { [weak self, weak model, weak window] response in
            guard let self, self.panel === picker else { return }
            self.finish()
            guard response == .OK, let destination = picker.url, let window, self.owns(window), let model, model.owns(target) else { return }
            Task { await model.download(row, destination: destination.path, target: target) }
        }
    }
    func close() { let picker = panel; finish(); picker?.cancel(nil); window = nil }
    deinit { if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) } }
}

struct NativeFilesWindowAnchor: NSViewRepresentable {
    let owner: NativeFilesChooser
    final class View: NSView {
        weak var owner: NativeFilesChooser?
        override func viewDidMoveToWindow() { super.viewDidMoveToWindow(); owner?.window = window }
    }
    func makeNSView(context: Context) -> View { let view = View(); view.owner = owner; return view }
    func updateNSView(_ view: View, context: Context) { view.owner = owner; owner.window = view.window }
    static func dismantleNSView(_ view: View, coordinator: ()) { view.owner?.window = nil }
}
