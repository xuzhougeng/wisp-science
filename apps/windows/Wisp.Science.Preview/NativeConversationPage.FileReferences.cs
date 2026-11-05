using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Windows.ApplicationModel.DataTransfer;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private bool fileDropBusy;
    private readonly Func<NativeSessionReferenceScope?>? sessionReferenceScope;
    private void EnableFileReferenceDrop(FrameworkElement target, Action<bool> highlight)
    {
        target.AllowDrop = true;
        bool Recognized(DataPackageView data) => data.Contains(NativeFileDrag.Format) || data.Contains(NativeSessionDrag.Format)
            || data.Contains(StandardDataFormats.StorageItems);
        bool Ready(DataPackageView data) => !disposed && !fileDropBusy && !clipboardBusy && !preparingSession
            && (data.Contains(NativeSessionDrag.Format) ? sessionReferenceScope != null && model.CanReceiveSessionReference : model.CanReceiveFileReference);
        void Over(object sender, DragEventArgs e)
        {
            if (!Recognized(e.DataView)) { highlight(false); return; }
            var ready = Ready(e.DataView); e.Handled = true;
            e.AcceptedOperation = ready ? DataPackageOperation.Copy : DataPackageOperation.None;
            highlight(ready);
            if (ready) e.DragUIOverride.Caption = e.DataView.Contains(NativeSessionDrag.Format)
                ? design.L("引用会话", "Reference conversations") : design.T("attachment.file_path");
        }
        target.AddHandler(UIElement.DragEnterEvent, new DragEventHandler(Over), true);
        target.AddHandler(UIElement.DragOverEvent, new DragEventHandler(Over), true);
        target.AddHandler(UIElement.DragLeaveEvent, new DragEventHandler((_, e) => {
            // Crossing child controls is still inside the composer drop surface.
            var position = e.GetPosition(target);
            if (position.X < 0 || position.Y < 0 || position.X >= target.ActualWidth || position.Y >= target.ActualHeight)
                highlight(false);
        }), true);
        target.Unloaded += (_, _) => highlight(false);
        target.AddHandler(UIElement.DropEvent, new DragEventHandler(async (_, e) => {
            highlight(false);
            if (!Recognized(e.DataView)) return;
            e.Handled = true;
            if (!Ready(e.DataView)) { e.AcceptedOperation = DataPackageOperation.None; return; }
            e.AcceptedOperation = DataPackageOperation.Copy;
            var deferral = e.GetDeferral(); var revision = model.FileReferenceRevision;
            fileDropBusy = true;
            try
            {
                bool accepted;
                if (e.DataView.Contains(NativeFileDrag.Format))
                {
                    // A workspace drag retains its scoped path validation; it
                    // must not fall back to an unscoped advertised format.
                    var value = await e.DataView.GetDataAsync(NativeFileDrag.Format) as string;
                    if (disposed || revision != model.FileReferenceRevision || value is not { Length: > 0 and < 32768 }) return;
                    var drag = JsonSerializer.Deserialize<NativeFileDrag>(value);
                    accepted = drag != null && await model.ReferenceFileAsync(drag, lifetime.Token);
                }
                else if (e.DataView.Contains(NativeSessionDrag.Format))
                {
                    var value = await e.DataView.GetDataAsync(NativeSessionDrag.Format) as string;
                    if (disposed || revision != model.FileReferenceRevision || value is not { Length: > 0 and < 32768 }) return;
                    var drag = JsonSerializer.Deserialize<NativeSessionDrag>(value);
                    var scope = sessionReferenceScope?.Invoke();
                    accepted = drag != null && scope != null && model.ReferenceSidebarSessions(drag, scope, revision) > 0;
                }
                else
                {
                    // Explorer files and folders are references, like native
                    // WebView drops. Do not copy/upload or create a session.
                    accepted = await model.ReferenceDroppedPathsAsync(async () =>
                        (await e.DataView.GetStorageItemsAsync()).Select(item => item.Path).ToArray(), lifetime.Token) > 0;
                }
                if (accepted && !disposed && revision == model.FileReferenceRevision) FocusComposer();
            }
            catch (OperationCanceledException) { }
            catch (Exception ex) { if (!disposed && revision == model.FileReferenceRevision) model.ReportAttachmentSelectionError(ex.Message); }
            finally { fileDropBusy = false; deferral.Complete(); }
        }), true);
    }
    private FrameworkElement ComposerReferenceCard(NativeReferenceOption reference)
    {
        var (icon, meta) = reference.Reference.Kind switch {
            "file-path" => ("doc", "attachment.file_path"), "session" => ("chat", "attachment.session"),
            "skill" => ("skill", "attachment.skill"), "project" => ("folder", "attachment.project"),
            "context" => ("server", "attachment.context"), "runtime" => ("terminal", "attachment.runtime"),
            _ => ("doc", "attachment.artifact") };
        var row = new Grid { Width = 224 };
        var content = new Grid { ColumnSpacing = 9, MinHeight = 42, Padding = new Thickness(7, 7, 27, 7) };
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.ColumnDefinitions.Add(new());
        content.Children.Add(new Border { Width = 32, Child = design.Icon(icon, 20), VerticalAlignment = VerticalAlignment.Center });
        var text = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        var label = design.Text(reference.Label, 12); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        label.TextTrimming = TextTrimming.CharacterEllipsis; label.TextWrapping = TextWrapping.NoWrap;
        text.Children.Add(label); var type = design.Text(design.T(meta), 10.5); type.Foreground = design.Brush("text-faint"); text.Children.Add(type);
        Grid.SetColumn(text, 1); content.Children.Add(text);
        row.Children.Add(new Border { Child = content, Margin = new Thickness(0, 7, 7, 0), Background = design.Brush("bg-elev"),
            BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8) });
        var remove = design.ToolButton(design.T("composer.remove_attachment"), "close"); remove.IsEnabled = model.CanRemoveReference;
        remove.Content = design.Icon("close", 12); remove.Width = remove.Height = 20; remove.MinWidth = remove.MinHeight = 0;
        remove.Padding = new Thickness(0); remove.CornerRadius = new CornerRadius(10);
        remove.Background = design.Brush("bg-elev"); remove.BorderBrush = design.Brush("border-strong"); remove.BorderThickness = new Thickness(1);
        remove.HorizontalAlignment = HorizontalAlignment.Right; remove.VerticalAlignment = VerticalAlignment.Top;
        AutomationProperties.SetName(remove, design.T("composer.remove_attachment") + ": " + reference.Label);
        remove.Click += (_, _) => model.RemoveReference(reference.Reference.Key); row.Children.Add(remove);
        ToolTipService.SetToolTip(row, reference.Label + (string.IsNullOrWhiteSpace(reference.Detail) ? "" : " · " + reference.Detail)); return row;
    }
}
