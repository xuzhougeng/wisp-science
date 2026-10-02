using Wisp.ProjectBrowser;

internal static class NativeWorkspaceLayoutTests
{
    public static void Run()
    {
        foreach (var width in new[] { 320d, 600, 800, 1024, 1280, 1440, 1920, 3440 })
        foreach (var sidebar in new[] { false, true })
        foreach (var panel in new[] { false, true })
        foreach (var preference in new[] { -5d, 320, 360, 640, 900, double.NaN })
        {
            var layout = PreviewLayout.Workspace(width, sidebar, panel, preference);
            Check(layout.ContentWidth >= Math.Min(width, PreviewLayout.MinimumConversationWidth), "center remains usable at every supported width");
            Check(layout.PanelWidth <= width && layout.PanelWidth >= 0, "inspector never extends outside client area");
            Check(Math.Abs(layout.ContentWidth + layout.SidebarWidth + (layout.PanelOverlay ? 0 : layout.PanelWidth) - width) < .01,
                "docked columns fit exactly without horizontal overflow");
            if (!panel) Check(layout.PanelWidth == 0 && !layout.PanelOverlay, "closing the panel restores the entire center width");
            if (!sidebar) Check(layout.SidebarWidth == 0, "user collapsed sidebar stays collapsed");
        }
        Check(PreviewLayout.Workspace(800, true, true, 360).PanelOverlay, "800 DIP uses an inspector drawer");
        Check(!PreviewLayout.Workspace(1440, true, true, 360).PanelOverlay, "desktop docks inspector alongside conversation");
        Check(PreviewLayout.Workspace(600, true, false, 360).SidebarWidth == 0, "small windows preserve reading width by hiding sidebar");
        Check(PreviewLayout.Workspace(1440, true, true, 480).PanelWidth == 480, "preferred inspector width survives returning from narrow window");
        Console.WriteLine("Native workspace column budget passed (width/sidebar/panel/preference matrix).");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
