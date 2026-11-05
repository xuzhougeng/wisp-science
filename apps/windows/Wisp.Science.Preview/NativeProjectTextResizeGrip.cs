using Microsoft.UI.Input;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.System;

namespace Wisp.Science.Preview;

internal sealed class NativeProjectTextResizeGrip : Grid
{
    public NativeProjectTextResizeGrip(TextBox editor, WispDesign design, string field)
    {
        Width = Height = 16; HorizontalAlignment = HorizontalAlignment.Right; VerticalAlignment = VerticalAlignment.Bottom;
        Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        ProtectedCursor = InputSystemCursor.Create(InputSystemCursorShape.SizeNorthSouth);
        IsTabStop = true; UseSystemFocusVisuals = true;
        var icon = design.Icon("resize", 12, design.ColorText("text-faint"));
        icon.HorizontalAlignment = HorizontalAlignment.Right; icon.VerticalAlignment = VerticalAlignment.Bottom;
        icon.Margin = new Thickness(0, 0, 1, 1); icon.IsHitTestVisible = false; Children.Add(icon);
        var label = design.L($"调整「{field}」高度，上下方向键调整", $"Resize {field}; use Up and Down arrows");
        AutomationProperties.SetName(this, label); ToolTipService.SetToolTip(this, label);
        uint? pointer = null; double startY = 0, startHeight = 0;
        PointerPressed += (_, e) =>
        {
            var point = e.GetCurrentPoint(null);
            if (pointer != null || !point.Properties.IsLeftButtonPressed || !CapturePointer(e.Pointer)) return;
            pointer = e.Pointer.PointerId; startY = point.Position.Y; startHeight = editor.ActualHeight;
            e.Handled = true;
        };
        // Screen-relative delta: resizing recenters the enclosing dialog and
        // moves the grip. Its own moving coordinate system would feed back.
        void Resize(double y) => editor.Height = Math.Max(editor.MinHeight, startHeight + y - startY);
        PointerMoved += (_, e) =>
        {
            if (e.Pointer.PointerId != pointer) return;
            Resize(e.GetCurrentPoint(null).Position.Y); e.Handled = true;
        };
        PointerReleased += (_, e) =>
        {
            if (e.Pointer.PointerId != pointer) return;
            Resize(e.GetCurrentPoint(null).Position.Y); pointer = null; ReleasePointerCapture(e.Pointer); e.Handled = true;
        };
        PointerCaptureLost += (_, e) => { if (e.Pointer.PointerId == pointer) pointer = null; };
        KeyDown += (_, e) =>
        {
            if (pointer != null || e.Key is not (VirtualKey.Up or VirtualKey.Down)) return;
            editor.Height = Math.Max(editor.MinHeight, editor.ActualHeight + (e.Key == VirtualKey.Up ? -24 : 24));
            e.Handled = true;
        };
    }
}
