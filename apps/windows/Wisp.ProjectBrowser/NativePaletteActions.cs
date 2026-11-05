namespace Wisp.ProjectBrowser;

public sealed record NativePaletteAction(string Id, string Title, string Icon, string Keywords, bool Project = false, bool Session = false, string? LabelKey = null)
{
    public string Label(NativeLocalization locale) => LabelKey != null ? locale.Text(LabelKey) : Id switch
    {
        "calendar" => locale.Choose(Title, "Research calendar"),
        "terminal" => locale.Choose(Title, "Toggle terminal"),
        _ => locale.Text(Id switch
        {
            "new-project" => "projects.new", "new" => "command.new_session",
            "docs" => "menu.docs", "issues" => "menu.issues",
            _ => "command." + Id.Replace('-', '_')
        })
    };
    public string Group => Id switch
    {
        "import-session" or "import-codex" or "import-claude" or "export-current-project" => "transfer",
        "theme-light" or "theme-dark" or "theme-system" or "font-ui-increase" or "font-ui-decrease" or "font-code-increase" or "font-code-decrease" => "appearance",
        "docs" or "issues" or "star-us" => "help",
        "projects" or "library" or "calendar" or "artifacts" or "notebook" or "files" or "provenance"
            or "contexts" or "runs" or "side-chat" or "close-panel" or "toggle-sidebar" or "terminal" => "navigate",
        _ => "general"
    };
}
public static class NativePaletteActions
{
    // Workspace search pins these commands below the matching documents, even
    // when no document matches. The command palette has its own searchable catalog.
    private static readonly NativePaletteAction[] Workspace = [
        new("new-project", "新建项目", "plus", "new project"),
        new("check-updates", "检查更新", "refresh", "updates"),
        new("star-us", "在 GitHub 上点亮 Star", "star", "github"),
        new("project-settings", "项目设置", "gear", "", Project: true, LabelKey: "proj_settings.title"),
        new("skills", "技能", "grid", "", Project: true, LabelKey: "settings.nav.skills")
    ];
    private static readonly NativePaletteAction[] All = [
        new("new", "新建会话", "plus", "new conversation session", true),
        new("new-window", "新建窗口", "expand", "new window gui"),
        new("search", "搜索项目与会话", "search", "find project artifact session 搜索 项目 会话"),
        new("settings", "设置", "gear", "preferences config"),
        new("privacy-mode", "隐私模式", "eye-off", "privacy mode hide restore projects 隐私 隐藏 录屏"),
        new("setup", "快速配置", "wrench", "quick setup onboarding model environment python r uv node 快速 配置 初始化 模型 环境"),
        new("import-codex", "导入 Codex 会话", "conversation-import", "import codex conversation session 导入 会话", true),
        new("import-claude", "导入 Claude Code 会话", "sparkles", "import claude code conversation session 导入 会话", true),
        new("import-session", "导入会话归档", "archive", "import restore archive zip conversation session 导入 恢复 归档 会话", true),
        new("export-current-project", "导出当前项目", "share", "export backup project archive zip 导出 备份 项目 归档", true),
        new("check-updates", "检查更新", "refresh", "update upgrade version 更新 升级 版本"),
        new("project-settings", "项目设置", "adjustments", "project preferences", true),
        new("skills", "管理技能", "skill", "manage skill 管理 技能", true),
        new("projects", "返回项目列表", "folder", "open switch projects"),
        new("library", "收藏", "star-filled", "favorites collection library 收藏 库"),
        new("toggle-sidebar", "显示／隐藏侧边栏", "panel", "show hide sidebar 显示 隐藏 侧栏", true),
        new("artifacts", "查看产物", "grid", "artifacts outputs results", true, true),
        new("notebook", "笔记本", "edit", "notebook", true, true),
        new("files", "文件", "doc", "files browser", true, true),
        new("runs", "运行记录", "activity-orbit", "runs background tasks transfers", true, true),
        new("provenance", "溯源", "timeline", "provenance history lineage", true, true),
        new("contexts", "执行环境", "server", "execution host runtime ssh wsl 执行 上下文 主机", true),
        new("side-chat", "侧边对话", "bubble", "ask side chat", true, true),
        new("close-panel", "关闭右侧面板", "close", "hide right panel", true, true),
        new("theme-light", "浅色主题", "sun", "light color appearance"),
        new("theme-dark", "深色主题", "moon", "dark color appearance"),
        new("theme-system", "跟随系统主题", "monitor", "system auto color appearance"),
        new("font-ui-increase", "增大界面字号", "type-increase", "font ui increase larger 字体 界面 增大"),
        new("font-ui-decrease", "缩小界面字号", "type-decrease", "font ui decrease smaller 字体 界面 缩小"),
        new("font-code-increase", "增大代码字号", "code-increase", "font code increase larger 字体 代码 增大"),
        new("font-code-decrease", "缩小代码字号", "code-decrease", "font code decrease smaller 字体 代码 缩小"),
        new("star-us", "去 GitHub 点个 Star", "star", "github star"),
        new("docs", "教程与帮助", "book", "docs documentation tutorials help"),
        new("issues", "反馈问题", "chat", "github issue bug feedback report 问题 反馈 报告"),
        // Keep these native shortcuts available after the shared command list.
        new("calendar", "研究日历", "calendar", "research calendar"),
        new("terminal", "切换终端", "terminal", "terminal shell", true, true),
        new("new-project", "新建项目", "folder-plus", "new project create"),
    ];
    public static IEnumerable<NativePaletteAction> Match(string query, bool project, bool session, bool commandsOnly, NativeLocalization? locale = null)
    {
        if (!commandsOnly) return Workspace.Where(a => !a.Project || project);
        var q = query.Trim();
        bool Has(string? text) => text?.Contains(q, StringComparison.OrdinalIgnoreCase) == true;
        return All.Where(a => (!a.Project || project) && (!a.Session || session)
            && (Has(a.Id) || Has(a.Title) || Has(a.Keywords) || locale != null && (Has(a.Label(locale)) || Has(locale.Text("command.group." + a.Group)))));
    }
}
