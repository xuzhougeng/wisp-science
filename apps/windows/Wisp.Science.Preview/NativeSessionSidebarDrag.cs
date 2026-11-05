using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>Sidebar moves share the existing composer-reference drag identity.</summary>
internal sealed class NativeSessionSidebarDrag(string project, WorkspaceSessionGroups groups,
    BrowserSession[] sessions, WispDesign design, Func<bool> current, Func<Task> updated)
{
    private NativeSessionDrag? dragging;
    private readonly List<Action> clearTargets = [];
    private void Clear() { foreach (var clear in clearTargets) clear(); }
    public void Source(FrameworkElement source, string kind, string id)
    {
        source.CanDrag = true;
        bool Ready() => current() && source.IsLoaded && !groups.Closed && !groups.Busy && !groups.Selecting;
        Windows.Foundation.Point? pressed = null; uint pointerId = 0; var active = false;
        source.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler((_, e) =>
        {
            var point = e.GetCurrentPoint(source);
            if (point.Properties.IsLeftButtonPressed) { pressed = point.Position; pointerId = point.PointerId; }
        }), true);
        source.AddHandler(UIElement.PointerMovedEvent, new PointerEventHandler(async (_, e) =>
        {
            var point = e.GetCurrentPoint(source);
            if (active || pressed is not { } origin || point.PointerId != pointerId || !point.Properties.IsLeftButtonPressed
                || Math.Abs(point.Position.X - origin.X) + Math.Abs(point.Position.Y - origin.Y) < 8) return;
            pressed = null; if (!Ready()) return;
            active = true; e.Handled = true; source.ReleasePointerCaptures();
            try { await source.StartDragAsync(point); }
            catch (Exception ex) { if (current()) { groups.Fail(ex.Message); await updated(); } }
            finally { active = false; dragging = null; Clear(); }
        }), true);
        source.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler((_, _) => pressed = null), true);
        source.PointerCanceled += (_, _) => pressed = null; source.PointerCaptureLost += (_, _) => pressed = null;
        source.DragStarting += (_, e) =>
        {
            if (!Ready()) { e.Cancel = true; return; }
            dragging = new(project, kind, id);
            e.Data.RequestedOperation = DataPackageOperation.Copy | DataPackageOperation.Move;
            e.Data.SetData(NativeSessionDrag.Format, JsonSerializer.Serialize(dragging));
        };
        source.DropCompleted += (_, _) => { dragging = null; Clear(); };
    }
    public void Target(Border target, string? folder)
    {
        target.AllowDrop = true;
        var transparent = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        void Reset() { target.Background = transparent; target.BorderBrush = transparent; target.BorderThickness = new(1); }
        clearTargets.Add(Reset); Reset();
        NativeFolderDrop? Plan() => current() && target.IsLoaded ? NativeFolderDrop.Plan(dragging, project, groups, sessions, folder) : null;
        void Over(object _, DragEventArgs e)
        {
            if (!e.DataView.Contains(NativeSessionDrag.Format)) return;
            e.Handled = true; var plan = Plan(); Clear();
            e.AcceptedOperation = plan == null ? DataPackageOperation.None : DataPackageOperation.Move;
            if (plan == null) return;
            target.BorderBrush = design.Brush("clay");
            if (plan.Order != null) target.BorderThickness = plan.After ? new(0, 0, 0, 2) : new(0, 2, 0, 0);
            else target.Background = design.Brush("clay-soft");
        }
        target.AddHandler(UIElement.DragEnterEvent, new DragEventHandler(Over), true);
        target.AddHandler(UIElement.DragOverEvent, new DragEventHandler(Over), true);
        target.AddHandler(UIElement.DragLeaveEvent, new DragEventHandler((_, e) =>
        {
            var point = e.GetPosition(target);
            if (point.X < 0 || point.Y < 0 || point.X >= target.ActualWidth || point.Y >= target.ActualHeight) Reset();
        }), true);
        target.Unloaded += (_, _) => Reset();
        target.AddHandler(UIElement.DropEvent, new DragEventHandler(async (_, e) =>
        {
            if (!e.DataView.Contains(NativeSessionDrag.Format)) return;
            e.Handled = true; var plan = Plan(); dragging = null; Clear();
            e.AcceptedOperation = plan == null ? DataPackageOperation.None : DataPackageOperation.Move;
            if (plan == null) return;
            var deferral = e.GetDeferral();
            try
            {
                if (plan.Order is { } order) await groups.ReorderFoldersAsync(order);
                else if (plan.SessionId is { } session) await groups.MoveSessionAsync(session, plan.FolderId);
                if (current()) await updated();
            }
            finally { deferral.Complete(); }
        }), true);
    }
}
