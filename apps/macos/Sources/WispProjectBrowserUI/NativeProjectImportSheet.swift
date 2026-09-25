import SwiftUI

struct NativeProjectImportSheet: View {
    @ObservedObject var model: ProjectBrowserModel
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("导入项目").font(.headline)
            Button(action: model.chooseProjectDirectory) {
                HStack(alignment: .top, spacing: 12) {
                    WispIcon(name: "folder")
                    VStack(alignment: .leading, spacing: 5) {
                        Text("打开项目文件夹").fontWeight(.semibold)
                        Text("使用原目录中的项目记录和文件，不复制大型工作区数据。").font(.caption).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }.disabled(model.importBusy)
            Button(action: model.chooseProjectArchive) {
                HStack(alignment: .top, spacing: 12) {
                    WispIcon(name: "archive")
                    VStack(alignment: .leading, spacing: 5) {
                        Text("导入 ZIP 归档").fontWeight(.semibold)
                        Text("解压到归档所在目录下的新文件夹，并登记项目。").font(.caption).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading)
                }
            }.disabled(model.importBusy)
            if model.importBusy { HStack { ProgressView().controlSize(.small); Text("正在导入…") } }
            if let error = model.importError { Text(error).font(.caption).foregroundStyle(.orange).textSelection(.enabled) }
            HStack { Spacer(); Button("取消") { model.importOptionsPresented = false }.disabled(model.importBusy) }
        }.buttonStyle(WispButtonStyle()).padding(24).frame(width: 470)
            .interactiveDismissDisabled(model.importBusy)
            .background(NativeSettingsEscape(enabled: !model.importBusy) { model.importOptionsPresented = false })
    }
}
