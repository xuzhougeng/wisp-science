using Wisp.ProjectBrowser;

internal static class NativeContextPanelLayoutTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static void Run()
    {
        var layout = new NativeContextPanelLayout();
        var docked = new NativeContextPanelGeometry(100, 400, 800, 300);
        layout.BeginMove(140, 420, docked);
        Check(!layout.Move(147, 420, 1200, 900) && !layout.Floating, "A click or movement below 8px must remain docked.");
        Check(layout.Move(148, 420, 1200, 900) && layout.Floating, "Movement at 8px must undock.");
        Check(layout.Geometry == new NativeContextPanelGeometry(108, 400, 800, 300), "Initial drag must preserve panel dimensions and pointer offset.");
        Check(layout.End() && !layout.Tracking && !layout.End(), "Only a completed change is persisted once.");
        layout.BeginResize(908, 700, layout.Geometry!);
        layout.Move(2000, 2000, 1200, 900);
        Check(layout.Geometry == new NativeContextPanelGeometry(16, 16, 1184, 884), "Oversized resize must remain in the viewport.");
        layout.End(); layout.Dock();
        Check(!layout.Floating && !layout.Resizing, "Dock must end gestures without discarding geometry.");
        layout.Geometry = new(90, 80, 420, 360);
        layout.BeginMove(140, 420, docked); layout.Move(300, 250, 1200, 900); layout.End();
        Check(layout.Geometry == new NativeContextPanelGeometry(260, 230, 420, 360), "Undock must reuse the remembered floating size.");
        layout.Fit(300, 200);
        Check(layout.Geometry == new NativeContextPanelGeometry(16, 16, 284, 184), "Small windows must shrink below normal minima.");
        var saved = System.Text.Json.JsonSerializer.Serialize(layout.Geometry);
        var restored = new NativeContextPanelLayout { Geometry = System.Text.Json.JsonSerializer.Deserialize<NativeContextPanelGeometry>(saved) };
        Check(!restored.Floating && restored.Geometry == layout.Geometry, "Restart restores geometry but defaults to docked mode.");
        restored.BeginResize(0, 0, docked);
        Check(!restored.Resizing && !restored.Move(100, 100, 1000, 800), "Docked panels cannot be resized.");
        var invalid = NativeContextPanelGeometry.Clamp(new(double.NaN, 0, -1, double.PositiveInfinity), 1000, 800);
        Check(invalid == new NativeContextPanelGeometry(0, 0, 420, 360), "Corrupt saved geometry must fall back to valid defaults.");
        var minimum = NativeContextPanelGeometry.Clamp(new(-20, -20, 1, 1), 1000, 800);
        Check(minimum == new NativeContextPanelGeometry(0, 0, 320, 220), "Minimum size and nonnegative positions must be enforced.");
        Check(NativeContextPanelGeometry.Clamp(docked, 0, 0) == new NativeContextPanelGeometry(0, 0, 1, 1), "Zero-sized roots must not throw during teardown.");
    }
}
