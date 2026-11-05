namespace Wisp.ProjectBrowser;

/// <summary>Breakpoints are WinUI device-independent pixels, not physical screen pixels.</summary>
public readonly record struct PreviewLayout(bool StackHomeColumns, bool StackHeader, bool CompactWorkspace, bool ShortWindow)
{
    public static IReadOnlyList<(string Label, string Icon)> WorkspaceActions { get; } =
    [ ("会话大纲", "list"), ("分享", "share"), ("运行轨迹", "timeline"), ("研究归档", "archive"), ("待查看", "bell"), ("终端", "terminal"), ("切换侧面板", "panel") ];
    public static PreviewLayout ForSize(double width, double height)
    {
        var home = HomeLayout.ForWidth(width);
        return new(home.StackColumns, home.ActionsBelowBrand || home.StackActions, IsSidebarRail(width), height < 580);
    }

    // Tauri requests a 1100 × 760 logical client area. AppWindow takes physical
    // outer bounds, which must also leave room for the frame and taskbar.
    public static (int Width, int Height) InitialWindowSize(uint dpi, int workWidth, int workHeight, int frameWidth, int frameHeight)
    {
        double scale = dpi == 0 ? 1 : dpi / 96d;
        int Fit(int logical, int frame, int available) =>
            (int)Math.Min(available, Math.Round(logical * scale) + Math.Max(0, frame));
        return (Fit(1100, frameWidth, workWidth), Fit(760, frameHeight, workHeight));
    }

    public const double ConversationMaxWidth = 1280;
    // chat.css --chat-col-max includes the thread's own 16-DIP side padding.
    public static double ConversationThreadWidth(double viewportWidth)
        => double.IsFinite(viewportWidth) ? Math.Clamp(viewportWidth - 32, 0, ConversationMaxWidth) : 0;
    public static double UserMessageMaxWidth(double threadContentWidth)
        => double.IsFinite(threadContentWidth) ? Math.Clamp(threadContentWidth * .78, 0, 760) : 0;
    public static double ComposerViewportHeight(double availableHeight, double manualTextHeight = double.NaN)
    {
        if (!double.IsFinite(availableHeight) || availableHeight <= 0) return 0;
        // Keep room for the transcript normally. A user deliberately enlarging
        // the input can use that room before its toolbar needs to scroll.
        var reserve = Math.Min(120, availableHeight * .3);
        var extra = double.IsFinite(manualTextHeight) ? Math.Max(0, ResizeComposer(manualTextHeight, 0) - ComposerAutoHeightLimit) : 0;
        return availableHeight - Math.Max(0, reserve - extra);
    }
    // Until the first drag, 220 is the auto-growing textarea's ceiling.
    // WebView uses that same value as the starting height of the first drag.
    public const double ComposerAutoHeightLimit = 220;
    public const double ComposerMaxTextHeight = 400;
    public static double ResizeComposer(double? savedHeight, double upwardDistance)
        => Math.Clamp((savedHeight is { } h && double.IsFinite(h) ? Math.Clamp(h, 80, ComposerMaxTextHeight) : ComposerAutoHeightLimit)
            + (double.IsFinite(upwardDistance) ? upwardDistance : 0), 80, ComposerMaxTextHeight);
    public const double SidebarWidth = 248;
    public const double SidebarRailWidth = 56;
    public static bool IsSidebarRail(double width) => double.IsFinite(width) && width > 0 && width <= 900;
    public const double MinimumConversationWidth = 360;
    public const double DefaultPanelWidth = 400;
    public const double PanelResizerWidth = 5;
    public static bool CompactConversationToolbar(double contentWidth) => contentWidth <= 720;
    public static bool CompactComposer(double paneWidth) => paneWidth <= 440;
    public static (double Model, double Effort, bool Wrap) FitComposerPickers(double width, double leading, double fixedTrailing, double model, double effort)
    {
        // Leave space for a chevron, and for the effort picker's brain icon.
        var minModel = Math.Min(model, 36); var minEffort = Math.Min(effort, 54);
        var wrap = width - leading - fixedTrailing < minModel + minEffort;
        var room = Math.Max(0, width - fixedTrailing - (wrap ? 0 : leading));
        if (model + effort <= room) return (model, effort, wrap);
        if (room < minModel + minEffort)
            return (room * minModel / (minModel + minEffort), room * minEffort / (minModel + minEffort), wrap);
        var flex = model - minModel + effort - minEffort;
        var share = flex > 0 ? Math.Clamp((room - minModel - minEffort) / flex, 0, 1) : 0;
        return (minModel + (model - minModel) * share, minEffort + (effort - minEffort) * share, wrap);
    }
    // --text-xl is 1.65rem; WebView leaves the root at 16px, independently of body size.
    public const double WelcomeHeadingSize = 1.65 * 16;
    public static int WelcomeHeadingTracking(string? locale) => NativeLocalization.Normalize(locale) == "zh" ? 10 : -20;
    public static WelcomeGlowBounds WelcomeGlow(double width, double height)
    {
        width = double.IsFinite(width) ? Math.Max(0, width) : 0;
        height = double.IsFinite(height) ? Math.Max(0, height) : 0;
        // chat.css .empty::before: inset 12% 18% auto, height 54%.
        // The default centered CSS ellipse reaches the farthest corner.
        var glowWidth = width * .64; var glowHeight = height * .54;
        return new(width * .18, height * .12, glowWidth, glowHeight,
            glowWidth / Math.Sqrt(2), glowHeight / Math.Sqrt(2));
    }
    // WebView sidebar.css folds by viewport height, independently of width.
    // Research entries leave the inline nav first; folders/files follow at 640px.
    public static bool SidebarEntryFolded(int tier, double viewportHeight)
        => double.IsFinite(viewportHeight) && viewportHeight > 0 && (tier switch
        {
            1 => viewportHeight <= 820,
            2 => viewportHeight <= 640,
            _ => false
        });
    public static double ClampPanelWidth(double width) => double.IsFinite(width) ? Math.Clamp(width, 320, 900) : DefaultPanelWidth;
    // Retain space for the conversation header, composer and a short transcript.
    public static double TerminalMaxHeight(double height) => double.IsFinite(height) ? Math.Clamp(height - 380, 180, 600) : 300;

    /// <summary>Budget columns in client DIPs. An inspector must never squeeze
    /// the conversation below its usable width; narrow windows use an overlay.</summary>
    public static WorkspaceColumns Workspace(double width, bool sidebarVisible, bool panelVisible, double panelWidth)
    {
        width = double.IsFinite(width) ? Math.Max(0, width) : 0;
        var sidebar = !sidebarVisible || width == 0 ? 0 : Math.Min(width, IsSidebarRail(width) ? SidebarRailWidth : SidebarWidth);
        var overlay = panelVisible && width <= 960;
        var panel = !panelVisible ? 0 : overlay
            ? width <= 640 ? width : Math.Min(440, Math.Max(0, width - 24))
            : Math.Min(ClampPanelWidth(panelWidth), Math.Max(320, width - sidebar - MinimumConversationWidth - PanelResizerWidth)) + PanelResizerWidth;
        return new(sidebar, panel, overlay, Math.Max(0, width - sidebar - (overlay ? 0 : panel)));
    }
}

public readonly record struct WorkspaceColumns(double SidebarWidth, double PanelWidth, bool PanelOverlay, double ContentWidth);
public readonly record struct WelcomeGlowBounds(double Left, double Top, double Width, double Height, double RadiusX, double RadiusY);
