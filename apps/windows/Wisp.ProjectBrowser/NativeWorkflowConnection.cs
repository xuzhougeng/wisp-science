namespace Wisp.ProjectBrowser;

/// Pointer gestures use logical canvas coordinates, independent of viewport zoom and scrolling.
public sealed class NativeWorkflowConnection
{
    public string? Source { get; private set; }
    public bool IsDragging { get; private set; }
    private double originX, originY;
    public void Begin(string source, double x, double y)
    { Source = source; originX = x; originY = y; IsDragging = false; }
    public void Move(double x, double y)
    { if (Source != null && Math.Pow(x - originX, 2) + Math.Pow(y - originY, 2) > 16) IsDragging = true; }
    public string? Target(NativeWorkflowGraph graph, double x, double y) => Source == null ? null
        : graph.Nodes.FirstOrDefault(n => n.Id != Source &&
            ((x >= n.X && x <= n.X + NativeWorkflowGraph.NodeWidth && y >= n.Y && y <= n.Y + NativeWorkflowGraph.NodeHeight)
            || (Math.Pow(x - n.X, 2) + Math.Pow(y - n.Y - NativeWorkflowGraph.NodeHeight / 2, 2) <= 169)))?.Id;
    public string? Release(NativeWorkflowGraph graph, double x, double y)
    {
        var target = Target(graph, x, y);
        // A click arms accessible click-to-connect; a drag to empty space cancels it.
        if (target != null || IsDragging) Cancel();
        return target;
    }
    public void Cancel() { Source = null; IsDragging = false; }
}
