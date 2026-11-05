using Windows.ApplicationModel.DataTransfer;
using Microsoft.UI.Xaml.Controls;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private bool clipboardBusy;
    private readonly TextCommandBarFlyout composerTextMenu = new();
    private bool composerTextMenuOpen;
    public bool HandleComposerTextEscape()
    {
        if (!composerTextMenuOpen) return false;
        composerTextMenu.Hide(); return true;
    }
    private void ConfigureComposerClipboardMenu()
    {
        composer.ContextFlyout = composerTextMenu;
        var paste = new AppBarButton { Icon = new ImageIcon { Source = design.Icon("clipboard").Source } };
        paste.Click += (_, _) => { composerTextMenu.Hide(); TryPasteComposerContent(); };
        composerTextMenu.SecondaryCommands.Add(paste);
        composerTextMenu.Opening += (_, _) =>
        {
            paste.Visibility = Microsoft.UI.Xaml.Visibility.Collapsed;
            try
            {
                var data = Clipboard.GetContent();
                var files = data.Contains(StandardDataFormats.StorageItems);
                if (!files && !data.Contains(StandardDataFormats.Bitmap)) return;
                paste.Visibility = Microsoft.UI.Xaml.Visibility.Visible;
                paste.Label = files ? design.L("粘贴文件引用", "Paste file references") : design.L("粘贴图片", "Paste image");
                paste.IsEnabled = !clipboardBusy && !preparingSession && (model.CanAttach || model.CanPrepareProjectDraft);
                if (!composerTextMenu.SecondaryCommands.Contains(paste)) composerTextMenu.SecondaryCommands.Insert(0, paste);
            }
            catch { /* Native text commands remain available if the clipboard is busy. */ }
        };
        composerTextMenu.Opened += (_, _) => composerTextMenuOpen = true;
        composerTextMenu.Closed += (_, _) => composerTextMenuOpen = false;
    }
    private bool TryPasteComposerContent()
    {
        if (disposed || preparingSession || !(model.CanAttach || model.CanPrepareProjectDraft)) return clipboardBusy;
        try
        {
            var data = Clipboard.GetContent();
            if (!data.Contains(StandardDataFormats.StorageItems) && !data.Contains(StandardDataFormats.Bitmap)) return false;
            if (!clipboardBusy) _ = PasteComposerContentAsync(data, model.FileReferenceRevision);
            return true;
        }
        catch (Exception ex)
        { model.ReportAttachmentSelectionError(design.L("无法读取剪贴板：", "Could not read clipboard: ") + ex.Message); return true; }
    }
    private async Task PasteComposerContentAsync(DataPackageView data, int revision)
    {
        clipboardBusy = true; commandsDismissed = true; model.ReferencePicker.Dismiss(); Refresh();
        var project = model.CurrentProjectId; var session = model.CurrentSessionId;
        bool Current() => !disposed && model.FileReferenceRevision == revision;
        try
        {
            // An Explorer copy may also advertise a bitmap. File identity takes
            // precedence, matching WebView: references, not duplicate uploads.
            if (data.Contains(StandardDataFormats.StorageItems))
            {
                var items = await data.GetStorageItemsAsync();
                if (!Current()) return;
                var paths = items.Select(item => item.Path).Where(path => !string.IsNullOrWhiteSpace(path)).ToArray();
                if (paths.Length > 0) { model.ReferenceClipboardPaths(paths, revision); return; }
            }
            if (!Current() || !data.Contains(StandardDataFormats.Bitmap)) return;
            using var image = await NativeClipboardImage.CaptureAsync(data, lifetime.Token);
            if (!Current() || !await EnsureSessionForActionAsync() || disposed || model.CurrentProjectId != project
                || session != null && model.CurrentSessionId != session || !model.CanAttach) return;
            revision = model.FileReferenceRevision;
            await model.AttachAsync(image.Path, lifetime.Token);
        }
        catch (OperationCanceledException) { }
        catch (Exception ex)
        { if (Current()) model.ReportAttachmentSelectionError(design.L("无法粘贴附件：", "Could not paste attachment: ") + ex.Message); }
        finally { clipboardBusy = false; if (!disposed) { Refresh(); if (Current()) FocusComposer(); } }
    }
}
