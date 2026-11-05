using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Streams;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>Clipped, literal card previews. Reads start only in the visible viewport.</summary>
internal sealed class NativeJourneyThumbnail : UserControl, IDisposable
{
    private readonly Border host = new();
    private UIElement? Child { get => host.Child; set => host.Child = value; }
    private readonly JourneyThumbnailLoader loader;
    private readonly JourneyEntry entry;
    private readonly WispDesign design;
    private readonly string kind;
    private CancellationTokenSource? request;
    private NativeRichPreview? imagePreview;
    private bool started, disposed;
    public NativeJourneyThumbnail(JourneyThumbnailLoader loader, JourneyEntry entry, WispDesign design)
    {
        this.loader = loader; this.entry = entry; this.design = design; kind = JourneyOutputPreview.Kind(entry.Title);
        host.Background = design.Brush("bg-app"); host.CornerRadius = new(4); Content = host;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch; IsHitTestVisible = false; IsTabStop = false;
        Fallback();
        SizeChanged += (_, _) => { Clip = new RectangleGeometry { Rect = new(0, 0, Math.Max(0, ActualWidth), Math.Max(0, ActualHeight)) }; if (imagePreview != null) imagePreview.MaxHeight = ActualHeight; };
        EffectiveViewportChanged += (sender, e) =>
        {
            var viewport = e.EffectiveViewport;
            if (!started && ActualWidth > 0 && ActualHeight > 0 && viewport.Right > 0 && viewport.Bottom > 0 && viewport.X < ActualWidth && viewport.Y < ActualHeight) _ = LoadAsync();
        };
        Unloaded += (_, _) => { request?.Cancel(); imagePreview?.Dispose(); imagePreview = null; started = false; };
    }
    private TextBlock Text(string text, double size = 10)
    { var label = design.Text(text, size); label.Foreground = design.Brush("text-muted"); label.TextWrapping = TextWrapping.Wrap; return label; }
    private void Fallback()
    {
        var stack = new StackPanel { Spacing = 8, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
        stack.Children.Add(design.Icon(kind == "image" ? "image" : kind == "csv" ? "table" : "doc", 28, design.ColorText("text-faint")));
        stack.Children.Add(Text(entry.SourceDiscarded ? design.L("源文件不可用", "Unavailable") : kind.ToUpperInvariant(), 12)); Child = stack;
    }
    private async Task LoadAsync()
    {
        if (disposed || started || entry.SourceDiscarded || string.IsNullOrEmpty(entry.SourceId) || !JourneyOutputPreview.CanLoad(kind)) return;
        started = true; request?.Dispose(); var current = request = new();
        try
        {
            var artifact = await loader.LoadAsync(entry.SourceId, current.Token);
            if (disposed || current.IsCancellationRequested || !IsLoaded || artifact.ContentError != null) return;
            if (kind == "image" && artifact.Base64 != null)
            {
                if (artifact.Mime == "image/svg+xml")
                {
                    imagePreview = new NativeRichPreview(design, "image", artifact.Base64, entry.Title, format: artifact.Mime) { MaxHeight = Height, IsTabStop = false, IsHitTestVisible = false };
                    Child = imagePreview;
                }
                else
                {
                    using var memory = new InMemoryRandomAccessStream(); using (var writer = new DataWriter(memory)) { writer.WriteBytes(Convert.FromBase64String(artifact.Base64)); await writer.StoreAsync(); writer.DetachStream(); }
                    memory.Seek(0); var bitmap = new BitmapImage(); await bitmap.SetSourceAsync(memory);
                    if (!disposed && !current.IsCancellationRequested && IsLoaded) Child = new Image { Source = bitmap, Stretch = Stretch.Uniform };
                }
            }
            else if (artifact.Text is { } text)
            {
                if (kind == "csv") Child = Table(text);
                else if (kind == "markdown")
                {
                    var content = new StackPanel { Padding = new(10), Spacing = 2, VerticalAlignment = VerticalAlignment.Top };
                    foreach (var (line, heading) in JourneyOutputPreview.Markdown(text)) { var label = Text(line); if (heading) label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; content.Children.Add(label); } Child = content;
                }
                else { var label = Text(JourneyOutputPreview.Excerpt(text), 9); label.FontFamily = design.Font(true); label.Margin = new(10); label.VerticalAlignment = VerticalAlignment.Top; Child = label; }
            }
        }
        catch (OperationCanceledException) { }
        catch { if (!disposed && !current.IsCancellationRequested) Fallback(); }
    }
    private FrameworkElement Table(string text)
    {
        var rows = JourneyOutputPreview.Table(text); var grid = new Grid { VerticalAlignment = VerticalAlignment.Top };
        var columns = rows.Select(r => r.Length).DefaultIfEmpty(1).Max();
        for (var c = 0; c < columns; c++) grid.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        for (var r = 0; r < rows.Length; r++)
        {
            grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
            for (var c = 0; c < rows[r].Length; c++)
            {
                var label = Text(rows[r][c], 9); label.FontFamily = design.Font(true); label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis; if (r == 0) label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
                var cell = new Border { Child = label, Padding = new(3), BorderThickness = new(0, 0, 1, 1), BorderBrush = design.Brush("border"), Background = r == 0 ? design.Brush("bg-sunken") : null };
                Grid.SetRow(cell, r); Grid.SetColumn(cell, c); grid.Children.Add(cell);
            }
        }
        return grid;
    }
    public void Dispose() { disposed = true; request?.Cancel(); imagePreview?.Dispose(); }
}
