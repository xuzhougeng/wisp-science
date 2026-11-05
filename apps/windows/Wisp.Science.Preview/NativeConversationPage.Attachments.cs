using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private ContentDialog? attachmentDialog;
    private int attachmentDialogRevision;
    public bool HandleAttachmentEscape()
    {
        if (attachmentDialog == null) return false;
        attachmentDialog.Hide(); return true;
    }
    private FrameworkElement AttachmentCard(ComposerAttachment file)
        => AttachmentCard(file.Name, file.Path, null);
    private FrameworkElement AttachmentCard(string name, string? path, NativeAttachmentUpload? upload)
    {
        var image = path != null && NativeAttachmentPresentation.Image(path);
        var row = new Grid { Width = image ? 238 : 224 };
        var content = new Grid { ColumnSpacing = 9, MinHeight = 42 };
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.ColumnDefinitions.Add(new());
        content.Children.Add(image
            ? new NativeAttachmentThumbnail(model, path!, design)
            : new Border { Width = 32, Child = design.Icon("doc", 20), VerticalAlignment = VerticalAlignment.Center });
        var copy = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
        var label = design.Text(name, 12); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
        if (upload != null) label.Foreground = design.Brush(upload.Uploading ? "text-muted" : "err");
        var meta = design.Text(design.T(upload?.MetaKey ?? (image ? "attachment.image" : "attachment.file")), 10.5);
        meta.Foreground = design.Brush(upload is { Uploading: false } ? "err" : "text-faint");
        copy.Children.Add(label); copy.Children.Add(meta); Grid.SetColumn(copy, 1); content.Children.Add(copy);
        var card = new Grid { Padding = new Thickness(7, 7, 27, 7) }; card.Children.Add(content);
        if (path != null)
        {
            var open = new Button { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), BorderThickness = new Thickness(0),
                HorizontalAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Stretch, Margin = new Thickness(-7, -7, -27, -7),
                IsEnabled = model.CanPreviewAttachment(path), CornerRadius = new CornerRadius(8) };
            AutomationProperties.SetName(open, name); AutomationProperties.SetAutomationId(open, "attachment-preview-" + path);
            open.Click += async (_, _) => await OpenAttachmentAsync(path, name); card.Children.Add(open);
        }
        row.Children.Add(new Border { Child = card, Margin = new Thickness(0, 7, 7, 0),
            Background = design.Brush("bg-elev"), BorderBrush = design.Brush(upload is { Uploading: false } ? "err" : "border"),
            BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8) });
        var remove = design.ToolButton(design.T("composer.remove_attachment") + " " + name, "close");
        remove.Content = design.Icon("close", 12); remove.Width = remove.Height = 20; remove.MinWidth = remove.MinHeight = 0;
        remove.Padding = new Thickness(0); remove.CornerRadius = new CornerRadius(10);
        remove.Background = design.Brush("bg-elev"); remove.BorderBrush = design.Brush("border-strong"); remove.BorderThickness = new Thickness(1);
        remove.HorizontalAlignment = HorizontalAlignment.Right; remove.VerticalAlignment = VerticalAlignment.Top;
        remove.IsEnabled = upload != null || model.CanAttach;
        remove.Click += (_, _) => { if (upload != null) model.DismissAttachmentUpload(upload.Id); else model.RemoveAttachment(path!); };
        row.Children.Add(remove);
        ToolTipService.SetToolTip(row, upload?.Error ?? name);
        return row;
    }
    private async Task OpenAttachmentAsync(string path, string name)
    {
        if (disposed || attachmentDialog != null || !model.CanPreviewAttachment(path)) return;
        var revision = attachmentDialogRevision = model.FileReferenceRevision;
        var body = new ContentControl { HorizontalContentAlignment = HorizontalAlignment.Stretch, MinWidth = 220, MinHeight = 100,
            Content = new ProgressRing { IsActive = true, Width = 28, Height = 28 } };
        var width = Math.Max(220, Math.Min(832, XamlRoot.Size.Width * .94 - 48));
        var height = Math.Max(120, XamlRoot.Size.Height * .52);
        var layout = new StackPanel { Spacing = 10, Width = width };
        var header = new Grid { ColumnSpacing = 8 };
        header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = design.Text(name, 14); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis; title.VerticalAlignment = VerticalAlignment.Center;
        header.Children.Add(title);
        var close = design.ToolButton(design.T("right.close"), "close"); close.Click += (_, _) => HandleAttachmentEscape();
        Grid.SetColumn(close, 1); header.Children.Add(close); layout.Children.Add(header);
        layout.Children.Add(new Border { Background = design.Brush("bg-sunken"), CornerRadius = new(8), Padding = new(8),
            Child = new ScrollViewer { Content = body, MaxHeight = height, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled } });
        var dialog = attachmentDialog = new ContentDialog { XamlRoot = XamlRoot, DefaultButton = ContentDialogButton.None,
            RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light, Content = layout };
        dialog.Resources["ContentDialogMaxWidth"] = 880d;
        dialog.Resources["ContentDialogBackground"] = design.Brush("bg-elev");
        AutomationProperties.SetName(dialog, name);
        NativeRichPreview? rich = null;
        var started = false;
        dialog.Loaded += async (_, _) =>
        {
            if (started) return; started = true;
            try
            {
                var content = await model.ReadAttachmentAsync(path, lifetime.Token);
                if (disposed || attachmentDialog != dialog || revision != model.FileReferenceRevision || content == null) return;
                if (content.Base64 != null && content.Mime.StartsWith("image/", StringComparison.Ordinal) && content.Mime != "image/svg+xml")
                {
                    var source = await NativeAttachmentThumbnail.DecodeAsync(content, 1600);
                    if (disposed || attachmentDialog != dialog || revision != model.FileReferenceRevision) return;
                    var pictureWidth = width - 16; var pictureHeight = Math.Max(80, height - 52);
                    var bitmap = (Microsoft.UI.Xaml.Media.Imaging.BitmapImage)source;
                    var scale = Math.Min(pictureWidth / bitmap.PixelWidth, pictureHeight / bitmap.PixelHeight);
                    var image = new Image { Source = source, Stretch = Stretch.Uniform, Width = bitmap.PixelWidth * scale, Height = bitmap.PixelHeight * scale,
                        HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
                    AutomationProperties.SetName(image, name);
                    var canvas = new Grid { Width = pictureWidth, Height = pictureHeight }; canvas.Children.Add(image);
                    var viewport = new ScrollViewer { Content = canvas, Height = pictureHeight, ZoomMode = ZoomMode.Enabled, MinZoomFactor = 1, MaxZoomFactor = 8,
                        HorizontalContentAlignment = HorizontalAlignment.Center, VerticalContentAlignment = VerticalAlignment.Center,
                        HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
                    var controls = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
                    var percentage = design.Text("100%", 12); percentage.VerticalAlignment = VerticalAlignment.Center;
                    var minus = design.ToolButton(design.L("缩小", "Zoom out"), "minus"); minus.Click += (_, _) => viewport.ChangeView(null, null, Math.Max(1, viewport.ZoomFactor / 1.25f));
                    var plus = design.ToolButton(design.L("放大", "Zoom in"), "plus"); plus.Click += (_, _) => viewport.ChangeView(null, null, Math.Min(8, viewport.ZoomFactor * 1.25f));
                    var fit = design.ToolButton(design.L("适应窗口", "Fit to window"), "expand"); fit.Click += (_, _) => viewport.ChangeView(0, 0, 1);
                    viewport.ViewChanged += (_, _) => percentage.Text = $"{viewport.ZoomFactor * 100:0}%";
                    controls.Children.Add(minus); controls.Children.Add(percentage); controls.Children.Add(plus); controls.Children.Add(fit);
                    var picture = new StackPanel { Spacing = 8 }; picture.Children.Add(controls); picture.Children.Add(viewport); body.Content = picture;
                }
                else if (content.Base64 != null && content.Mime == "image/svg+xml")
                    body.Content = rich = new NativeRichPreview(design, "image", content.Base64, name, format: content.Mime, escape: () => HandleAttachmentEscape());
                else if (content.Base64 != null && content.Mime == "application/pdf")
                    body.Content = rich = new NativeRichPreview(design, "pdf", content.Base64, name, format: content.Mime, escape: () => HandleAttachmentEscape());
                else if (content.Text is { } text)
                    body.Content = new TextBox { AcceptsReturn = true, Text = text, IsReadOnly = true, TextWrapping = TextWrapping.Wrap,
                        FontFamily = design.Font(true), Height = Math.Min(320, height - 16), VerticalContentAlignment = VerticalAlignment.Top };
                else body.Content = design.Text(design.L("无法预览此附件。", "Preview is unavailable for this attachment."), 13);
            }
            catch (Exception ex)
            { if (!disposed && attachmentDialog == dialog) body.Content = design.Text(ex.Message, 13); }
        };
        try { await dialog.ShowAsync(); }
        finally { if (attachmentDialog == dialog) attachmentDialog = null; rich?.Dispose(); }
    }
}
