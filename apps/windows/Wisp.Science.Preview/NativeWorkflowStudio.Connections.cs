using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkflowStudio
{
    private void BeginConnection(NativeWorkflowNode node, UIElement port, PointerRoutedEventArgs e)
    {
        if (!e.GetCurrentPoint(canvas).Properties.IsLeftButtonPressed || model.Busy) return;
        CancelConnection(false);
        var point = e.GetCurrentPoint(canvas).Position;
        connection.Begin(node.Id, point.X, point.Y); suppressPortClick = true;
        // ButtonBase already captured the press before this handled-events listener.
        // Transfer capture so moving outside the port continues to reach the canvas.
        port.ReleasePointerCaptures();
        if (canvas.CapturePointer(e.Pointer)) connectionPointer = e.Pointer;
        // Keep layout fixed while pressed; inserting the status row here moves the
        // canvas under the pointer and incorrectly turns a click into a drag.
        e.Handled = true;
    }
    private void MoveConnection(object sender, PointerRoutedEventArgs e)
    {
        if (connectionPointer?.PointerId != e.Pointer.PointerId) return;
        var point = e.GetCurrentPoint(canvas).Position; connection.Move(point.X, point.Y);
        var source = graph.Nodes.FirstOrDefault(n => n.Id == connecting); if (source == null) { CancelConnection(); return; }
        var start = new Point(source.X + NativeWorkflowGraph.NodeWidth, source.Y + NativeWorkflowGraph.NodeHeight / 2);
        var bend = Math.Max(32, Math.Abs(point.X - start.X) / 2);
        var figure = new PathFigure { StartPoint = start, IsClosed = false };
        figure.Segments.Add(new BezierSegment { Point1 = new(start.X + bend, start.Y), Point2 = new(point.X - bend, point.Y), Point3 = point });
        var geometry = new PathGeometry(); geometry.Figures.Add(figure); connectionPreview.Data = geometry;
        var target = ConnectionPointInViewport(e) ? connection.Target(graph, point.X, point.Y) : null;
        foreach (var (key, card) in nodeCards)
        {
            var id = S(model.TaskForKey(key), "id");
            card.BorderBrush = design.Brush(id == target || id == connecting ? "clay" : "border-strong");
            card.Opacity = 1;
        }
        e.Handled = true;
    }
    private bool ConnectionPointInViewport(PointerRoutedEventArgs e)
    {
        var point = e.GetCurrentPoint(viewport).Position;
        return point.X >= 0 && point.Y >= 0 && point.X <= viewport.ActualWidth && point.Y <= viewport.ActualHeight;
    }
    private void ReleaseConnection(object sender, PointerRoutedEventArgs e)
    {
        if (connectionPointer?.PointerId != e.Pointer.PointerId) return;
        var source = connecting; var point = e.GetCurrentPoint(canvas).Position; connection.Move(point.X, point.Y);
        var target = ConnectionPointInViewport(e) ? connection.Release(graph, point.X, point.Y) : connection.Release(graph, double.NaN, double.NaN);
        var pointer = connectionPointer; connectionPointer = null; canvas.ReleasePointerCapture(pointer); connectionPreview.Data = null;
        DispatcherQueue.TryEnqueue(() => suppressPortClick = false);
        e.Handled = true;
        if (target != null && source != null) { model.Connect(source, target); Render(); }
        else { RenderSelection(); UpdateState(); }
    }
    private void CompleteConnection(string target)
    {
        if (connecting is not { } source || source == target) return;
        CancelConnection(); model.Connect(source, target); Render();
    }
    private void CancelConnection(bool refresh = true)
    {
        connection.Cancel(); connectionPreview.Data = null;
        var pointer = connectionPointer; connectionPointer = null;
        if (pointer != null) canvas.ReleasePointerCapture(pointer);
        suppressPortClick = false;
        if (refresh && !rendering && !closed) { RenderSelection(); UpdateState(); }
    }
}
