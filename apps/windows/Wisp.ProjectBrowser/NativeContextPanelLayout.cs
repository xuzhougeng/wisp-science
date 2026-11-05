namespace Wisp.ProjectBrowser;

public sealed record NativeContextPanelGeometry(double X, double Y, double Width, double Height)
{
    public bool Valid => double.IsFinite(X) && double.IsFinite(Y) && double.IsFinite(Width) && double.IsFinite(Height)
        && Width > 0 && Height > 0;
    public static NativeContextPanelGeometry Clamp(NativeContextPanelGeometry value, double viewportWidth, double viewportHeight)
    {
        var viewW = Math.Max(0, double.IsFinite(viewportWidth) ? viewportWidth : 1280);
        var viewH = Math.Max(0, double.IsFinite(viewportHeight) ? viewportHeight : 800);
        var maxW = Math.Max(1, viewW - 16); var maxH = Math.Max(1, viewH - 16);
        var source = value.Valid ? value : new(0, 0, 420, 360);
        var w = Math.Clamp(source.Width, Math.Min(320, maxW), maxW);
        var h = Math.Clamp(source.Height, Math.Min(220, maxH), maxH);
        return new(Math.Clamp(source.X, 0, Math.Max(0, viewW - w)), Math.Clamp(source.Y, 0, Math.Max(0, viewH - h)), w, h);
    }
}

/// WebView context-usage geometry: mode is per window, only geometry is persisted.
public sealed class NativeContextPanelLayout
{
    public NativeContextPanelGeometry? Geometry { get; set; }
    public bool Floating { get; private set; }
    public bool Tracking { get; private set; }
    public bool Resizing { get; private set; }
    public bool Moved { get; private set; }
    private double originX, originY, grabX, grabY;
    private NativeContextPanelGeometry? resizeStart;
    public void Dock() { Floating = false; End(); }
    public void BeginMove(double x, double y, NativeContextPanelGeometry bounds)
    {
        Tracking = true; Resizing = Moved = false; originX = x; originY = y;
        grabX = x - bounds.X; grabY = y - bounds.Y;
        if (Geometry?.Valid != true) Geometry = bounds;
    }
    public void BeginResize(double x, double y, NativeContextPanelGeometry bounds)
    {
        if (!Floating) return;
        Resizing = true; Tracking = false; Moved = false; originX = x; originY = y; resizeStart = Geometry = bounds;
    }
    public bool Move(double x, double y, double viewportWidth, double viewportHeight)
    {
        if (!double.IsFinite(x) || !double.IsFinite(y)) return false;
        if (Resizing && resizeStart is { } start)
        {
            Moved = true;
            Geometry = NativeContextPanelGeometry.Clamp(start with { Width = start.Width + x - originX, Height = start.Height + y - originY }, viewportWidth, viewportHeight);
            return true;
        }
        if (!Tracking || !Moved && Math.Sqrt(Math.Pow(x - originX, 2) + Math.Pow(y - originY, 2)) < 8) return false;
        Moved = Floating = true;
        var size = Geometry is { Valid: true } saved ? saved : new NativeContextPanelGeometry(0, 0, 420, 360);
        Geometry = NativeContextPanelGeometry.Clamp(size with { X = x - grabX, Y = y - grabY }, viewportWidth, viewportHeight);
        return true;
    }
    public bool End()
    {
        var changed = Moved; Tracking = Resizing = Moved = false; resizeStart = null; return changed;
    }
    public void Fit(double width, double height)
    {
        if (Geometry != null) Geometry = NativeContextPanelGeometry.Clamp(Geometry, width, height);
    }
}
