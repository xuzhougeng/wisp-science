namespace Wisp.ProjectBrowser;

/// <summary>Breakpoints are WinUI device-independent pixels, not physical screen pixels.</summary>
public readonly record struct PreviewLayout(bool StackHomeColumns, bool StackHeader, bool CompactWorkspace, bool ShortWindow)
{
    public static IReadOnlyList<(string Label, string Icon)> WorkspaceActions { get; } =
    [ ("会话大纲", "list"), ("分享", "share"), ("运行轨迹", "timeline"), ("研究归档", "archive"), ("待查看", "bell"), ("终端", "terminal"), ("切换侧面板", "panel") ];
    public static PreviewLayout ForSize(double width, double height) => new(width < 680, width < 1080, width < 1000, height < 580);

    public const double ConversationMaxWidth = 1280;
    public const double MinimumConversationWidth = 560;
    public static bool CompactConversationToolbar(double contentWidth) => contentWidth < 760;
    public static bool StackComposerSend(double actionWidth) => actionWidth < 480;
    public static double ClampPanelWidth(double width) => double.IsFinite(width) ? Math.Clamp(width, 320, 640) : 360;
    // Retain space for the conversation header, composer and a short transcript.
    public static double TerminalMaxHeight(double height) => double.IsFinite(height) ? Math.Clamp(height - 380, 180, 600) : 300;

    /// <summary>Budget columns in client DIPs. An inspector must never squeeze
    /// the conversation below its usable width; narrow windows use an overlay.</summary>
    public static WorkspaceColumns Workspace(double width, bool sidebarVisible, bool panelVisible, double panelWidth)
    {
        width = double.IsFinite(width) ? Math.Max(0, width) : 0;
        var sidebar = width < 1000 ? 218d : 244d;
        if (!sidebarVisible || width - sidebar < MinimumConversationWidth) sidebar = 0;
        var panel = panelVisible ? ClampPanelWidth(panelWidth) : 0;
        var overlay = panelVisible && width - sidebar - panel < MinimumConversationWidth;
        if (overlay) panel = Math.Min(panel, Math.Max(0, width - (width > 480 ? 32 : 0)));
        return new(sidebar, panel, overlay, Math.Max(0, width - sidebar - (overlay ? 0 : panel)));
    }
}

public readonly record struct WorkspaceColumns(double SidebarWidth, double PanelWidth, bool PanelOverlay, double ContentWidth);
