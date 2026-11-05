using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Streams;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>Downscaled image thumbnail, bound to the staged attachment's scope.</summary>
internal sealed class NativeAttachmentThumbnail : UserControl
{
    private readonly Border host = new();
    public NativeAttachmentThumbnail(WorkspaceConversationModel model, string path, WispDesign design, double width = 42, double height = 42)
    {
        Width = width; Height = height; host.CornerRadius = new(4); host.Background = design.Brush("bg-sunken");
        host.Child = design.Icon("image", 20); Content = host; IsHitTestVisible = false;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        Clip = new RectangleGeometry { Rect = new(0, 0, width, height) };
        SizeChanged += (_, e) => Clip = new RectangleGeometry { Rect = new(0, 0, e.NewSize.Width, e.NewSize.Height) };
        Loaded += async (_, _) =>
        {
            var revision = model.FileReferenceRevision;
            try
            {
                var content = await model.ReadAttachmentAsync(path);
                if (!IsLoaded || revision != model.FileReferenceRevision || content?.Base64 is not { Length: > 0 and <= 24_000_000 } data
                    || !content.Mime.StartsWith("image/", StringComparison.Ordinal)) return;
                var source = await DecodeAsync(content, (int)Math.Ceiling(width * 2));
                if (IsLoaded && revision == model.FileReferenceRevision && model.CanPreviewAttachment(path)) host.Child = new Image { Source = source, Stretch = Stretch.UniformToFill };
            }
            catch { /* Keep the same image icon fallback used by WebView. */ }
        };
    }
    internal static async Task<ImageSource> DecodeAsync(NativePanelFileContent content, int width)
    {
        using var stream = new InMemoryRandomAccessStream();
        using (var writer = new DataWriter(stream)) { writer.WriteBytes(Convert.FromBase64String(content.Base64!)); await writer.StoreAsync(); writer.DetachStream(); }
        stream.Seek(0);
        if (content.Mime == "image/svg+xml")
        { var svg = new SvgImageSource { RasterizePixelWidth = width, RasterizePixelHeight = width }; await svg.SetSourceAsync(stream); return svg; }
        var bitmap = new BitmapImage { DecodePixelWidth = width }; await bitmap.SetSourceAsync(stream); return bitmap;
    }
}
