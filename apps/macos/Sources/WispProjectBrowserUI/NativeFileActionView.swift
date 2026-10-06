import SwiftUI
import WispProjectBrowser

struct NativeFileActionSelection: Identifiable {
    let id = UUID()
    let action: NativePanelFileAction
    let directory: String
    var name = ""
    var title: String {
        switch action {
        case .createFile: return "新建文件"
        case .createDirectory: return "新建文件夹"
        case .rename: return "重命名"
        case .delete: return "删除"
        }
    }
}

struct NativeFileActionView: View {
    let selection: NativeFileActionSelection
    @ObservedObject var model: NativePanelModel
    let close: () -> Void
    let perform: (NativePanelFileAction, String, String?) async throws -> Void
    @State private var name: String
    @State private var error: String?
    @State private var submitting = false
    @State private var unconfirmed = false
    init(selection: NativeFileActionSelection, model: NativePanelModel, perform: ((NativePanelFileAction, String, String?) async throws -> Void)? = nil, close: @escaping () -> Void) {
        self.selection = selection; self.model = model; self.close = close
        self.perform = perform ?? { action, path, newPath in try await model.performFileAction(action, path: path, newPath: newPath) }
        _name = State(initialValue: selection.name)
    }
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(localized(selection.title)).font(.headline)
            if selection.action == .delete {
                Text(String(format: localized("永久删除“%@”？文件夹内的所有内容也会被删除。此操作无法撤销。"), selection.name))
            } else {
                TextField(localized("名称"), text: $name).textFieldStyle(.roundedBorder)
                    .disabled(submitting || unconfirmed)
            }
            Text(selection.directory).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
            if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                Spacer()
                Button(localized("取消"), action: close).disabled(submitting)
                Button(localized(selection.title), role: selection.action == .delete ? .destructive : nil) {
                    submitting = true; error = nil
                    Task { @MainActor in
                        defer { submitting = false }
                        do {
                            let target = try NativePanelFileAction.destination(directory: selection.directory, name: selection.action == .rename || selection.action == .delete ? selection.name : name)
                            let destination = selection.action == .rename ? try NativePanelFileAction.destination(directory: selection.directory, name: name) : nil
                            // Only validation can be retried in this sheet. Once a
                            // write is dispatched, an unknown result needs a new
                            // read and a fresh human decision, never a replay.
                            unconfirmed = true
                            try await perform(selection.action, target, destination)
                            close()
                        } catch { self.error = localized(unconfirmed ? "操作未确认成功，请关闭并刷新核对后再操作。" : "请输入有效的文件名称。") + "\n" + error.localizedDescription }
                    }
                }.disabled(submitting || unconfirmed || (selection.action != .delete && name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
            }
        }.padding(24).frame(width: 420)
            .interactiveDismissDisabled(submitting)
            .background(NativeSettingsEscape(enabled: !submitting, close: close))
    }
}
