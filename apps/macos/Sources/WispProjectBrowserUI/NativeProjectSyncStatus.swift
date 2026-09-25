import Foundation
import WispProjectBrowser

struct NativeProjectSyncStatus: Equatable {
    let label: String
    let needsAttention: Bool
    init?(_ project: ProjectSummary) {
        if let state = project.folderSync {
            switch state {
            case "saved": label = "已保存到项目文件夹"
            case "unpublished": label = "更改尚未保存到项目文件夹"
            case "remote-newer": label = "项目文件夹有更新版本"
            case "waiting": label = "正在等待项目文件下载完成"
            case "conflict": label = "项目文件夹存在同步冲突"
            default: label = "项目文件夹状态：" + state
            }
            needsAttention = state == "conflict" || state == "remote-newer"
        } else if project.syncConfigured {
            label = "已配置项目同步"
            needsAttention = false
        } else { return nil }
    }
    static func lastSaved(_ project: ProjectSummary) -> String? {
        guard let time = project.lastSyncedAt, time > 0 else { return nil }
        return "上次同步：" + Date(timeIntervalSince1970: TimeInterval(time)).formatted(date: .abbreviated, time: .shortened)
    }
}
