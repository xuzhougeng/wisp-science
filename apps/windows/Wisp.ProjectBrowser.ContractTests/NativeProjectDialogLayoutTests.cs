using Wisp.ProjectBrowser;

internal static class NativeProjectDialogLayoutTests
{
    public static void Run()
    {
        Check(Math.Abs(ProjectDialogLayout.ForProjectForm(588, 760).Width - 540.96) < .001
            && ProjectDialogLayout.ForProjectForm(400, 360) == new ProjectDialogLayout(360, 320)
            && ProjectDialogLayout.ForProjectForm(1100, 760) == new ProjectDialogLayout(560, 720),
            "project forms use 92vw between the padded small-screen bound and the desktop cap");
        Check(ProjectDialogLayout.EditorHeight(2, 12) == 56 && ProjectDialogLayout.EditorHeight(8, 12) == 164,
            "two and eight visible text rows match the WebView at the default font size");
        Check(ProjectDialogLayout.EditorHeight(2, 18) == 74 && ProjectDialogLayout.EditorHeight(8, 18) == 236
            && ProjectDialogLayout.EditorHeight(8, 7) == 150,
            "larger interface text expands the initial rows; smaller type preserves the context minimum");
        Check(ProjectDialogLayout.EditorMinimum(2) == 20 && ProjectDialogLayout.EditorMinimum(8) == 150,
            "dragging can shrink both fields below their initial height with different lower bounds");
        // Includes the reported window at 150% DPI, short windows and large displays.
        foreach (var width in new[] { 320d, 520, 600, 883, 1122, 1440, 1920 })
        foreach (var height in new[] { 240d, 400, 700, 760, 1080 })
        {
            var dialog = ProjectDialogLayout.ForViewport(width, height);
            Check(dialog.Width > 0 && dialog.Width <= 560 && dialog.Width + 40 <= width,
                "dialog stays within the window with room for the backdrop on both sides");
            Check(dialog.MaxHeight > 0 && dialog.MaxHeight + 40 <= height,
                "short windows constrain the scrolling form inside the backdrop");
            Check((width - dialog.Width) / 2 >= 20,
                "centered modal cannot shift the form beyond the right edge");
            var group = ProjectDialogLayout.ForViewport(width, height, 480);
            Check(group.Width <= 480 && group.Width + 40 <= width && group.MaxHeight == dialog.MaxHeight,
                "compact group dialogs share the project dialog's viewport and scrolling bounds");
            foreach (var commandsOnly in new[] { false, true })
            {
                var search = SearchPaletteLayout.ForViewport(width, height, commandsOnly);
                Check(search.Width > 0 && search.Width <= width - 24
                    && search.Top >= 0 && search.Top + search.MaxHeight <= height,
                    "search palettes leave a backdrop and keep results and footer in short windows");
            }
        }
        Check(ProjectDialogLayout.ForViewport(1100, 720).Width == 560,
            "desktop modal matches the WebView 560-DIP width");
        var outer = PreviewLayout.InitialWindowSize(144, 2560, 1500, 24, 12);
        var client = ProjectDialogLayout.ForViewport((outer.Width - 24) / 1.5, (outer.Height - 12) / 1.5);
        Check(client == new ProjectDialogLayout(560, 720),
            "at 150% scale a new window leaves 20 DIPs at each edge of the full client for project dialogs");
        Check(SearchPaletteLayout.ForViewport(1100, 760).Width == 720
            && SearchPaletteLayout.ForViewport(1100, 760).Top == 72
            && SearchPaletteLayout.ForViewport(800, 540).Width == 560
            && SearchPaletteLayout.ForViewport(800, 540).Top == 54
            && SearchPaletteLayout.ForViewport(469, 757).Width == 445
            && SearchPaletteLayout.ForViewport(800, 760, true).Width == 560,
            "global search matches desktop and short-window WebView palette bounds");
        Check(ProjectDialogLayout.ForViewport(1650 / 1.5, 1080 / 1.5)
            == ProjectDialogLayout.ForViewport(2200 / 2d, 1440 / 2d),
            "equivalent effective viewport sizes have the same geometry at different DPI settings");
        foreach (var invalid in new[] { 0d, -1, double.NaN, double.PositiveInfinity })
            Check(ProjectDialogLayout.ForViewport(invalid, invalid) == new ProjectDialogLayout(0, 0),
                "unmeasured viewports cannot produce invalid control sizes");
        Console.WriteLine("Native project dialog viewport bounds passed.");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
