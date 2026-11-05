using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

// An invisible top edge, like .composer-resizer. Track the pointer in window
// coordinates: the edge itself moves as the bottom-anchored composer grows.
internal sealed class NativeComposerResizeGrip : Grid
{
    private uint? pointer;
    public bool Dragging => pointer != null;

    public NativeComposerResizeGrip(Func<double?> current, Action<double?> preview, Action<double> commit)
    {
        Height = 8; Margin = new Thickness(-14, 0, -12, 4);
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        ProtectedCursor = InputSystemCursor.Create(InputSystemCursorShape.SizeNorthSouth);
        IsTabStop = true; UseSystemFocusVisuals = true;
        double startY = 0; double? startHeight = null, pending = null;
        PointerPressed += (_, e) =>
        {
            var point = e.GetCurrentPoint(null);
            if (Dragging || !point.Properties.IsLeftButtonPressed || !CapturePointer(e.Pointer)) return;
            pointer = e.Pointer.PointerId; startY = point.Position.Y; startHeight = current(); pending = null;
            e.Handled = true;
        };
        PointerMoved += (_, e) =>
        {
            if (e.Pointer.PointerId != pointer) return;
            var dy = startY - e.GetCurrentPoint(null).Position.Y;
            if (dy == 0 && pending == null) return;
            pending = PreviewLayout.ResizeComposer(startHeight, dy); preview(pending); e.Handled = true;
        };
        PointerReleased += (_, e) =>
        {
            if (e.Pointer.PointerId != pointer) return;
            var dy = startY - e.GetCurrentPoint(null).Position.Y;
            var changed = pending.HasValue || dy != 0;
            pointer = null; ReleasePointerCapture(e.Pointer);
            if (changed) commit(PreviewLayout.ResizeComposer(startHeight, dy));
            e.Handled = true;
        };
        void Cancel()
        {
            if (!Dragging) return;
            pointer = null; ReleasePointerCaptures(); preview(current());
        }
        PointerCaptureLost += (_, e) => { if (e.Pointer.PointerId == pointer) Cancel(); };
        PointerCanceled += (_, e) => { if (e.Pointer.PointerId == pointer) Cancel(); };
        Unloaded += (_, _) => Cancel();
        KeyDown += (_, e) =>
        {
            if (Dragging || e.Key is not (VirtualKey.Up or VirtualKey.Down)) return;
            commit(PreviewLayout.ResizeComposer(current(), e.Key == VirtualKey.Up ? 24 : -24)); e.Handled = true;
        };
    }
}
