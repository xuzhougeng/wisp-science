import Foundation

struct NativeSearchCommand: Identifiable, Equatable {
    let id: String
    let title: String
    let icon: String
    let keywords: String
    var project = false
    var session = false
    static let all: [Self] = [
        .init(id: "new-project", title: "新建项目", icon: "folder-plus", keywords: "new project"),
        .init(id: "new-session", title: "新建会话", icon: "plus", keywords: "new conversation session", project: true),
        .init(id: "settings", title: "设置", icon: "gear", keywords: "preferences config"),
        .init(id: "project-settings", title: "项目设置", icon: "adjustments", keywords: "project preferences", project: true),
        .init(id: "skills", title: "管理技能", icon: "sparkles", keywords: "manage skills", project: true),
        .init(id: "workflows", title: "工作流", icon: "plan", keywords: "workflows"),
        .init(id: "projects", title: "返回项目列表", icon: "folder", keywords: "home projects"),
        .init(id: "library", title: "收藏", icon: "star", keywords: "library favorites"),
        .init(id: "calendar", title: "研究日历", icon: "calendar", keywords: "research calendar"),
        .init(id: "journey", title: "研究历程", icon: "research-trail", keywords: "journey history", project: true),
        .init(id: "publication", title: "论文证据", icon: "book", keywords: "publication evidence", project: true),
        .init(id: "import-project", title: "导入项目", icon: "upload", keywords: "import project zip"),
        .init(id: "artifacts", title: "查看产物", icon: "grid", keywords: "artifacts outputs", project: true, session: true),
        .init(id: "notebook", title: "笔记本", icon: "edit", keywords: "notebook", project: true, session: true),
        .init(id: "files", title: "文件", icon: "doc", keywords: "files browser", project: true, session: true),
        .init(id: "provenance", title: "溯源", icon: "timeline", keywords: "provenance lineage", project: true, session: true),
        .init(id: "contexts", title: "执行环境", icon: "server", keywords: "contexts environments hosts", project: true, session: true),
        .init(id: "sidechat", title: "侧边对话", icon: "bubble", keywords: "side chat", project: true, session: true),
        .init(id: "close-panel", title: "关闭右侧面板", icon: "close", keywords: "hide panel", project: true, session: true),
        .init(id: "toggle-sidebar", title: "显示／隐藏侧边栏", icon: "panel", keywords: "sidebar", project: true),
        .init(id: "terminal", title: "切换终端", icon: "terminal", keywords: "terminal shell", project: true, session: true)
    ]
    static func matching(_ query: String, project: Bool, session: Bool) -> [Self] {
        let only = query.hasPrefix(">")
        let terms = (only ? String(query.dropFirst()) : query).lowercased().split(whereSeparator: \.isWhitespace)
        return all.filter { action in
            (!action.project || project) && (!action.session || session)
                && (only || ["new-project", "new-session", "settings", "project-settings", "skills"].contains(action.id))
                && terms.allSatisfy { (action.title + " " + localized(action.title) + " " + action.keywords).lowercased().contains($0) }
        }
    }
}

struct NativeWorkspaceCommand {
    let id = UUID()
    let action: String
    let project: String
    let session: String?
}
