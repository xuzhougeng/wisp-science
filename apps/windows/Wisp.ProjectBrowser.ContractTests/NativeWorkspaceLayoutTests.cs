using Wisp.ProjectBrowser;

internal static class NativeWorkspaceLayoutTests
{
    public static void Run()
    {
        Check(PreviewLayout.ResizeComposer(null, 52) == 272 && PreviewLayout.ResizeComposer(272, -52) == 220,
            "the first drag starts from WebView's auto limit, later drags start from the saved custom height");
        Check(PreviewLayout.ResizeComposer(270, -800) == 80 && PreviewLayout.ResizeComposer(100, 700) == 400
            && PreviewLayout.ResizeComposer(double.NaN, 12) == 232 && PreviewLayout.ResizeComposer(10, 20) == 100,
            "resizing and restoring ignore invalid geometry and enforce the WebView input limits");
        Check(PreviewLayout.ComposerViewportHeight(280) == 196
            && PreviewLayout.ComposerViewportHeight(800) == 680
            && PreviewLayout.ComposerViewportHeight(-1) == 0
            && PreviewLayout.ComposerViewportHeight(double.NaN) == 0,
            "short composer keeps transcript space and scrolls long quote banners rather than clipping send controls");
        Check(PreviewLayout.ComposerViewportHeight(560, 220) == 440 && PreviewLayout.ComposerViewportHeight(560, 280) == 500
            && PreviewLayout.ComposerViewportHeight(560, 400) == 560,
            "a tall manual input can use the reserved reading space to keep its bottom toolbar in view");
        Check(PreviewLayout.ConversationThreadWidth(1707) == 1280
            && PreviewLayout.ConversationThreadWidth(1188) == 1156
            && PreviewLayout.ConversationThreadWidth(620) == 588,
            "conversation thread caps with the composer on wide screens and preserves two 16-DIP outer gutters on narrow screens");
        Check(PreviewLayout.ConversationThreadWidth(1312) == 1280
            && PreviewLayout.ConversationThreadWidth(1311) == 1279
            && PreviewLayout.ConversationThreadWidth(20) == 0
            && PreviewLayout.ConversationThreadWidth(double.NaN) == 0,
            "resize crosses the column cap continuously and ignores unmeasured viewport sizes");
        Check(PreviewLayout.UserMessageMaxWidth(1248) == 760
            && Math.Abs(PreviewLayout.UserMessageMaxWidth(556) - 433.68) < .001
            && PreviewLayout.UserMessageMaxWidth(0) == 0
            && PreviewLayout.UserMessageMaxWidth(double.PositiveInfinity) == 0,
            "right-aligned user bubbles cap at 760 DIP or 78 percent of the measured thread and shrink safely on resize");
        Check(PreviewLayout.CompactConversationToolbar(720) && !PreviewLayout.CompactConversationToolbar(721),
            "topbar actions fold at the center pane's 720-DIP CSS boundary");
        Check(CenterTabLayout.Widths(1000, 1400, 0).SequenceEqual([280d])
            && Math.Abs(CenterTabLayout.Widths(1000, 600, 0)[0] - 216) < .001,
            "conversation tab caps at 280 DIP or 36% of the center pane");
        foreach (var pane in new[] { 320d, 600, 720, 800, 1459 })
        foreach (var available in new[] { 0d, 80, 180, 319.5, 500, 1200 })
        foreach (var count in new[] { 0, 1, 3, 10 })
        {
            var widths = CenterTabLayout.Widths(available, pane, count);
            Check(widths.Length == count + 1 && widths[0] <= Math.Min(280, pane * .36) + .001,
                "all file tabs retain a separate conversation tab bounded by the pane");
            Check(widths.Skip(1).All(w => w >= 72 && w <= 220), "file tabs shrink to their 72-DIP minimum before horizontal overflow");
            var minimum = 30 + count * 76;
            var maximum = Math.Min(280, pane * .36) + count * 224;
            Check(Math.Abs(widths.Sum() + count * 4 - Math.Clamp(available, minimum, maximum)) < .01,
                "flex widths consume the available strip exactly unless caps or scroll minimums apply");
        }
        Check(Math.Abs(PreviewLayout.WelcomeHeadingSize - 26.4) < .001,
            "welcome rem heading uses the browser root size, not the 14px body text");
        foreach (var (width, height) in new[] { (320d, 460d), (560d, 400d), (1000d, 300d) })
        {
            var glow = PreviewLayout.WelcomeGlow(width, height);
            Check(Math.Abs(glow.Left * 2 + glow.Width - width) < .001 && glow.Top >= 0 && glow.Top + glow.Height <= height,
                "welcome glow stays centered inside the content-sized surface across wrapping changes");
            var corner = Math.Pow(glow.Width / 2 / glow.RadiusX, 2) + Math.Pow(glow.Height / 2 / glow.RadiusY, 2);
            Check(Math.Abs(corner - 1) < .001,
                "CSS farthest-corner ellipse reaches the rectangle corners instead of fading too early at its side midpoints");
        }
        Check(PreviewLayout.WelcomeGlow(double.NaN, -1) == new WelcomeGlowBounds(0, 0, 0, 0, 0, 0),
            "unmeasured welcome geometry cannot pass invalid radii into the native brush");
        Check(PreviewLayout.WelcomeHeadingTracking("en") == -20 && PreviewLayout.WelcomeHeadingTracking("zh-CN") == 10,
            "Chinese welcome headings keep their positive tracking override instead of compressed Latin spacing");
        foreach (var width in new[] { 320d, 600, 640, 641, 800, 960, 961, 1024, 1280, 1440, 1920, 3440 })
        foreach (var sidebar in new[] { false, true })
        foreach (var panel in new[] { false, true })
        foreach (var preference in new[] { -5d, 320, 360, 640, 900, double.NaN })
        {
            var layout = PreviewLayout.Workspace(width, sidebar, panel, preference);
            Check(layout.ContentWidth >= Math.Min(width - layout.SidebarWidth, PreviewLayout.MinimumConversationWidth), "a docked inspector never takes the conversation's minimum width");
            Check(layout.PanelWidth <= width && layout.PanelWidth >= 0, "inspector never extends outside client area");
            Check(Math.Abs(layout.ContentWidth + layout.SidebarWidth + (layout.PanelOverlay ? 0 : layout.PanelWidth) - width) < .01,
                "docked columns fit exactly without horizontal overflow");
            if (!panel) Check(layout.PanelWidth == 0 && !layout.PanelOverlay, "closing the panel restores the entire center width");
            if (!sidebar) Check(layout.SidebarWidth == 0, "user collapsed sidebar stays collapsed");
        }
        Check(PreviewLayout.Workspace(800, true, true, 360).PanelOverlay, "800 DIP uses an inspector drawer");
        Check(!PreviewLayout.Workspace(1440, true, true, 360).PanelOverlay, "desktop docks inspector alongside conversation");
        Check(PreviewLayout.Workspace(1440, true, false, 360).SidebarWidth == 248,
            "desktop sidebar matches the WebView 248-DIP width");
        foreach (var width in new[] { 901d, 900, 810, 760, 470, 900, 901 })
        {
            var expected = width > 900 ? 248 : 56;
            Check(PreviewLayout.Workspace(width, true, false, 400).SidebarWidth == expected
                && PreviewLayout.Workspace(width, false, false, 400).ContentWidth == width
                && PreviewLayout.ForSize(width, 760).CompactWorkspace == (expected == 56),
                "the shell and mounted sidebar agree across the WebView breakpoint, without reopening a collapsed sidebar");
        }
        Check(PreviewLayout.Workspace(1440, true, true, 480).PanelWidth == 485, "docked width includes the five-DIP resizer separately from the preferred content width");
        Check(PreviewLayout.Workspace(1103, true, true, 400).PanelWidth == 405
            && !PreviewLayout.Workspace(1103, true, true, 400).PanelOverlay, "reference 1103-DIP window docks the default 400-DIP pane");
        Check(PreviewLayout.Workspace(960, true, true, 900).PanelOverlay
            && PreviewLayout.Workspace(960, true, true, 900).PanelWidth == 440
            && !PreviewLayout.Workspace(961, true, true, 900).PanelOverlay,
            "inspector switches at the CSS 960-DIP breakpoint independently of the preferred width");
        Check(PreviewLayout.Workspace(640, true, true, 320).PanelWidth == 640
            && PreviewLayout.Workspace(641, true, true, 900).PanelWidth == 440,
            "mobile inspector fills the window through 640 DIPs and uses a 440-DIP drawer above it");
        Check(PreviewLayout.Workspace(1103, true, true, 900).ContentWidth == 360
            && PreviewLayout.Workspace(1920, true, true, 900).PanelWidth == 905,
            "desktop inspector clamps to available center budget and expands again in a wide window");
        Check(PreviewLayout.ClampPanelWidth(double.NaN) == 400 && PreviewLayout.ClampPanelWidth(999) == 900,
            "default and maximum content widths match WebView");
        foreach (var height in new[] { 420d, 640, 641, 820, 821, 1000 })
        {
            Check(PreviewLayout.SidebarEntryFolded(1, height) == (height <= 820)
                && PreviewLayout.SidebarEntryFolded(2, height) == (height <= 640)
                && !PreviewLayout.SidebarEntryFolded(0, height),
                "height tiers preserve New Session/Search, fold research first, then folders/files in short windows");
            Check(!PreviewLayout.SidebarEntryFolded(2, height) || PreviewLayout.SidebarEntryFolded(1, height),
                "folders and files cannot fold while research entries remain inline");
        }
        var resizeSequence = new[] { 1000d, 820, 640, 641, 821, 1000 };
        Check(resizeSequence.Select(h => (PreviewLayout.SidebarEntryFolded(1, h), PreviewLayout.SidebarEntryFolded(2, h)))
            .SequenceEqual([(false, false), (true, false), (true, true), (true, false), (false, false), (false, false)]),
            "expanding after both compact tiers restores every inline entry rather than retaining More");
        Check(!PreviewLayout.SidebarEntryFolded(1, double.NaN) && !PreviewLayout.SidebarEntryFolded(1, 0),
            "unmeasured sidebar height does not hide navigation entries");
        Console.WriteLine("Native workspace column budget passed (width/sidebar/panel/preference matrix).");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
