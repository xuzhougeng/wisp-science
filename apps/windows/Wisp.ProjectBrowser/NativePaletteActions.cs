namespace Wisp.ProjectBrowser;

public sealed record NativePaletteAction(string Id, string Title, string Icon, string Keywords, bool Project = false, bool Session = false);
public static class NativePaletteActions
{
    private static readonly NativePaletteAction[] All = [
        new("new", "新建会话", "plus", "new conversation session", true),
        new("new-window", "新建窗口", "expand", "new window gui"),
        new("search", "搜索项目与会话", "search", "find project artifact message"),
        new("settings", "设置", "gear", "preferences config"),
        new("project-settings", "项目设置", "adjustments", "project preferences", true),
        new("skills", "管理技能", "sparkles", "manage skills", true),
        new("projects", "返回项目列表", "folder", "open switch projects"),
        new("library", "收藏", "star", "favorites library"),
        new("calendar", "研究日历", "calendar", "research calendar"),
        new("import-session", "导入会话归档", "archive", "import restore archive zip conversation session", true),
        new("import-cli", "导入 Codex / Claude 会话", "sync", "import codex claude cli external session", true),
        new("artifacts", "查看产物", "grid", "artifacts outputs results", true, true),
        new("notebook", "笔记本", "edit", "notebook", true, true),
        new("files", "文件", "doc", "files browser", true, true),
        new("provenance", "溯源", "timeline", "provenance history lineage", true, true),
        new("contexts", "执行环境", "server", "contexts hosts environments", true, true),
        new("side-chat", "侧边对话", "bubble", "ask side chat", true, true),
        new("close-panel", "关闭右侧面板", "close", "hide right panel", true, true),
        new("toggle-sidebar", "显示／隐藏侧边栏", "panel", "show hide sidebar", true),
        new("theme-light", "浅色主题", "sun", "light color appearance"),
        new("theme-dark", "深色主题", "moon", "dark color appearance"),
        new("theme-system", "跟随系统主题", "monitor", "system auto color appearance"),
        new("terminal", "切换终端", "terminal", "terminal shell", true, true),
        new("docs", "教程与帮助", "book", "docs documentation tutorials help"),
        new("issues", "准备问题反馈", "chat", "issue bug feedback report", true, true),
    ];
    public static IEnumerable<NativePaletteAction> Match(string query, bool project, bool session, bool commandsOnly) =>
        All.Where(a => (!a.Project || project) && (!a.Session || session)
            && (commandsOnly || a.Id is "new" or "settings" or "project-settings" or "skills")
            && query.Split(' ', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
                .All(token => (a.Title + " " + a.Keywords).Contains(token, StringComparison.OrdinalIgnoreCase)));
}
