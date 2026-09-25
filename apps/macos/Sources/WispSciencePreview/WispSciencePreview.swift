import AppKit
import SwiftUI
import WispProjectBrowserUI

@main
struct WispSciencePreview: App {
    @NSApplicationDelegateAdaptor(NativeApplicationDelegate.self) private var delegate
    @StateObject private var model = ProjectBrowserModel()

    var body: some Scene {
        Window("Wisp Science — 原生预览", id: "workspace") {
            WorkspaceRoot(model: model, delegate: delegate)
        }
        .defaultSize(width: 1120, height: 820)
        .commands {
            CommandGroup(replacing: .appSettings) {
                Button("设置…") { model.searchPresented = false; model.settingsPresented = true }.keyboardShortcut(",")
            }
            CommandGroup(replacing: .newItem) {
                Button("选择数据库…") { model.chooseDatabase() }
                    .keyboardShortcut("o")
                    .disabled(model.isLoading || model.settingsPresented)
            }
            CommandGroup(after: .textEditing) {
                Button("搜索项目与会话") { model.searchPresented = true }
                    .keyboardShortcut("k")
                    .disabled(model.searchPresented || model.settingsPresented)
            }
            CommandGroup(after: .newItem) {
                Button("刷新项目") { Task { await model.refresh() } }
                    .keyboardShortcut("r")
                    .disabled(model.isLoading || model.settingsPresented)
            }
        }
    }
}

private struct WorkspaceRoot: View {
    @ObservedObject var model: ProjectBrowserModel
    let delegate: NativeApplicationDelegate
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        ProjectBrowserView(model: model)
            .frame(minWidth: 680, minHeight: 560)
            .onAppear { delegate.openWorkspace = { openWindow(id: "workspace") } }
            .task { await model.refresh() }
    }
}
