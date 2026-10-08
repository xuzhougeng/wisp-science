import AppKit
import SwiftUI
import WispProjectBrowserUI

@main
struct WispScience: App {
    @NSApplicationDelegateAdaptor(NativeApplicationDelegate.self) private var delegate
    @StateObject private var model = ProjectBrowserModel()

    var body: some Scene {
        Window("Wisp Science SwiftUI", id: "workspace") {
            WorkspaceRoot(model: model, delegate: delegate)
        }
        .defaultSize(width: 1120, height: 820)
        .commands { NativeWorkspaceCommands() }
        WindowGroup("Wisp Science", for: NativeWorkspaceWindowRequest.self) { $request in
            if let request {
                NativeIndependentWorkspaceView(source: model, request: request)
            }
        }
        .defaultSize(width: 1120, height: 820)
    }
}

private struct WorkspaceRoot: View {
    @ObservedObject var model: ProjectBrowserModel
    let delegate: NativeApplicationDelegate
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        ProjectBrowserView(model: model)
            .frame(minWidth: 680, minHeight: 560)
            .focusedSceneValue(\.projectBrowser, model)
            .onAppear { delegate.openWorkspace = { openWindow(id: "workspace") } }
            .task { await model.refresh() }
    }
}
