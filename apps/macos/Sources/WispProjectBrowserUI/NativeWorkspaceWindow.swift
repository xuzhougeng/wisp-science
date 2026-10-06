import Foundation
import SwiftUI
import WispProjectBrowser

/// Windows share transports by database, while all view state remains local.
/// Choosing another database must never reuse the previous database's host.
@MainActor
final class NativeWorkspaceTransports {
    private let override: (any NativeSettingsQuerying)?
    private var hosts: [URL: NativeSettingsClient] = [:]
    init(override: (any NativeSettingsQuerying)? = nil) { self.override = override }
    func client(database: URL) -> any NativeSettingsQuerying {
        if let override { return override }
        if let host = hosts[database] { return host }
        let host = NativeSettingsClient(databaseURL: database, executableURL: nativeDesktopHostURL())
        hosts[database] = host
        return host
    }
}

/// SwiftUI scene identity and navigation only; no drafts or host credentials.
public struct NativeWorkspaceWindowRequest: Codable, Hashable, Sendable {
    public let id: UUID
    public let databaseURL: URL
    public let projectID: String?
    public let sessionID: String?

    public init(databaseURL: URL, projectID: String? = nil, sessionID: String? = nil) {
        id = UUID(); self.databaseURL = databaseURL; self.projectID = projectID; self.sessionID = sessionID
    }
    public var valid: Bool {
        databaseURL.isFileURL && [nil, "", "localhost"].contains(databaseURL.host)
            && databaseURL.query == nil && databaseURL.fragment == nil && databaseURL.path.count > 1
            && databaseURL.path.hasPrefix("/")
            && (projectID.map { !$0.isEmpty && $0.utf8.count <= 512 } ?? true)
            && (sessionID.map { !$0.isEmpty && $0.utf8.count <= 512 && projectID != nil } ?? true)
    }
}

private struct ProjectBrowserFocusedKey: FocusedValueKey { typealias Value = ProjectBrowserModel }
public extension FocusedValues {
    var projectBrowser: ProjectBrowserModel? {
        get { self[ProjectBrowserFocusedKey.self] }
        set { self[ProjectBrowserFocusedKey.self] = newValue }
    }
}

/// Menu actions use the focused scene's model, including independent windows.
@MainActor
public struct NativeWorkspaceCommands: Commands {
    @FocusedValue(\.projectBrowser) private var model
    @Environment(\.openWindow) private var openWindow
    private var unavailable: Bool { model?.windowReady != true }
    public init() {}
    public var body: some Commands {
        CommandGroup(replacing: .appSettings) {
            Button(localized("设置…")) { model?.searchPresented = false; model?.settingsPresented = true }
                .keyboardShortcut(",").disabled(unavailable)
        }
        CommandGroup(replacing: .newItem) {
            Button(localized("新建窗口")) { if let request = model?.newWindowRequest() { openWindow(value: request) } }
                .keyboardShortcut("n").disabled(unavailable)
            Button(localized("选择数据库…")) { model?.chooseDatabase() }
                .keyboardShortcut("o").disabled(unavailable || model?.isLoading == true || model?.settingsPresented == true)
        }
        CommandGroup(after: .textEditing) {
            Button(localized("搜索项目与会话")) { model?.searchPresented = true }
                .keyboardShortcut("k").disabled(unavailable || model?.searchPresented == true || model?.settingsPresented == true)
        }
        CommandGroup(after: .newItem) {
            Button(localized("刷新项目")) { if let model { Task { await model.refresh() } } }
                .keyboardShortcut("r").disabled(unavailable || model?.isLoading == true || model?.settingsPresented == true)
        }
    }
}

@MainActor
public struct NativeIndependentWorkspaceView: View {
    @StateObject private var model: ProjectBrowserModel
    private let request: NativeWorkspaceWindowRequest
    @State private var attempt = UUID()
    public init(source: ProjectBrowserModel, request: NativeWorkspaceWindowRequest) {
        self.request = request
        _model = StateObject(wrappedValue: source.makeIndependentWorkspace(databaseURL: request.databaseURL))
    }
    public var body: some View {
        Group {
            if model.windowReady {
                ProjectBrowserView(model: model)
            } else {
                VStack(spacing: 16) {
                    if !model.windowOpening, let error = model.error ?? model.sessionError {
                        Text(error).textSelection(.enabled).foregroundStyle(.orange)
                        Button(localized("重新读取")) { attempt = UUID() }
                    } else { ProgressView(localized("正在打开窗口…")) }
                }.padding(24).frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 680, minHeight: 560)
        .focusedSceneValue(\.projectBrowser, model)
        .task(id: attempt) { _ = await model.loadWindow(request) }
    }
}
