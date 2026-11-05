namespace Wisp.ProjectBrowser;

public sealed record NativeWindowMenuItem(string Action, string Key, string Shortcut = "");

/// The Windows title bar uses the same context-specific menus as window_titlebar.rs.
public static class NativeWindowMenus
{
    public const double Height = 38;
    public static IReadOnlyList<string> Groups { get; } = Array.AsReadOnly(new[] { "file", "edit", "view", "help" });
    private static readonly NativeWindowMenuItem[] File = [
        new("new", "command.new_session", "Ctrl+N"), new("new-window", "command.new_window"),
        new("projects", "command.projects"), new("files", "command.files"), new("export-current-project", "command.export_current_project"),
        new("settings", "command.settings", "Ctrl+,"), new("", ""), new("quit", "menu.quit")];
    private static readonly NativeWindowMenuItem[] HomeFile = [
        new("new-project", "projects.new", "Ctrl+N"), new("new-window", "command.new_window"), new("import-project", "projects.import"),
        new("settings", "command.settings", "Ctrl+,"), new("", ""), new("quit", "menu.quit")];
    private static readonly NativeWindowMenuItem[] Edit = [
        new("search", "command.search", "Ctrl+K"), new("commands", "menu.commands", "Ctrl+P"),
        new("import-codex", "command.import_codex"), new("import-claude", "command.import_claude"), new("import-session", "command.import_session"),
        new("project-settings", "command.project_settings"), new("skills", "command.skills")];
    private static readonly NativeWindowMenuItem[] HomeEdit = [
        new("search", "command.search", "Ctrl+K"), new("commands", "menu.commands", "Ctrl+P")];
    private static readonly NativeWindowMenuItem[] View = [
        new("toggle-sidebar", "command.toggle_sidebar", "Ctrl+B"), new("artifacts", "command.artifacts"),
        new("notebook", "command.notebook"), new("files", "command.files"), new("runs", "command.runs"),
        new("provenance", "command.provenance"), new("contexts", "command.contexts"), new("side-chat", "command.side_chat"),
        new("close-panel", "command.close_panel"), new("", ""), new("theme-light", "command.theme_light"),
        new("theme-dark", "command.theme_dark"), new("theme-system", "command.theme_system")];
    private static readonly NativeWindowMenuItem[] HomeView = [new("theme-light", "command.theme_light"),
        new("theme-dark", "command.theme_dark"), new("theme-system", "command.theme_system")];
    private static readonly NativeWindowMenuItem[] Help = [new("check-updates", "settings.check_updates"), new("", ""),
        new("docs", "menu.docs"), new("star-us", "menu.star_us"), new("issues", "menu.issues")];
    public static IReadOnlyList<NativeWindowMenuItem> Items(string group, bool home) => group switch {
        "file" => home ? HomeFile : File, "edit" => home ? HomeEdit : Edit,
        "view" => home ? HomeView : View, "help" => Help, _ => [] };
    public static bool Enabled(string action, bool hasProject) => action is not
        ("export-current-project" or "import-codex" or "import-claude") || hasProject;
}
