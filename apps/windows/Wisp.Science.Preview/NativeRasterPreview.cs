using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Input;
using Microsoft.UI.Xaml.Input;
using Windows.Foundation;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeRasterPreview : StackPanel
{
    private readonly BitmapImage bitmap;
    private readonly Image image;
    private readonly RasterCanvas canvas;
    private readonly ScrollViewer viewport;
    private readonly Button minus, plus, percentage;
    private readonly TextBlock percentageText;
    private double availableWidth, availableHeight;
    private int zoom = 100;
    private (Point Point, double X, double Y)? drag;
    private NativeRasterPreview(BitmapImage bitmap, string name, WispDesign design, double width, double height)
    {
        this.bitmap = bitmap; Spacing = 8;
        image = new Image { Source = bitmap, Stretch = Stretch.Uniform,
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Top };
        AutomationProperties.SetName(image, name);
        canvas = new RasterCanvas { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) }; canvas.Children.Add(image);
        viewport = new ScrollViewer { Content = canvas, ZoomMode = ZoomMode.Disabled,
            HorizontalContentAlignment = HorizontalAlignment.Left, VerticalContentAlignment = VerticalAlignment.Top,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        var controls = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4, HorizontalAlignment = HorizontalAlignment.Center };
        Button ZoomButton(string label, UIElement content)
        {
            var button = new Button { Content = content, MinWidth = 30, MinHeight = 0, Height = 28,
                Padding = new Thickness(8, 0, 8, 0), CornerRadius = new(5),
                Background = design.Brush("bg-elev"), Foreground = design.Brush("text-muted"),
                BorderBrush = design.Brush("border-strong"), BorderThickness = new(1) };
            button.Resources["ButtonBackgroundPointerOver"] = design.Brush("surface-hover");
            button.Resources["ButtonForegroundPointerOver"] = design.Brush("text");
            button.Resources["ButtonBackgroundDisabled"] = design.Brush("bg-elev");
            button.Resources["ButtonForegroundDisabled"] = design.Brush("text-muted");
            button.Resources["ButtonBorderBrushDisabled"] = design.Brush("border-strong");
            AutomationProperties.SetName(button, label); ToolTipService.SetToolTip(button, label);
            return button;
        }
        percentageText = design.Text("100%", 12); percentageText.TextWrapping = TextWrapping.NoWrap;
        minus = ZoomButton(design.L("缩小", "Zoom out"), design.Icon("minus", 12));
        plus = ZoomButton(design.L("放大", "Zoom in"), design.Icon("plus", 12));
        percentage = ZoomButton(design.L("重置缩放", "Reset zoom"), percentageText);
        minus.Click += (_, _) => SetZoom(NativeRasterSize.StepZoom(zoom, -1));
        plus.Click += (_, _) => SetZoom(NativeRasterSize.StepZoom(zoom, 1));
        percentage.Click += (_, _) => SetZoom(100);
        // Handle inside the ScrollViewer, before its built-in wheel scrolling.
        canvas.PointerWheelChanged += (_, e) =>
        {
            var point = e.GetCurrentPoint(canvas);
            if (point.Properties.IsHorizontalMouseWheel || point.Properties.MouseWheelDelta == 0) return;
            SetZoom(NativeRasterSize.StepZoom(zoom, point.Properties.MouseWheelDelta)); e.Handled = true;
        };
        canvas.PointerPressed += (_, e) =>
        {
            var point = e.GetCurrentPoint(viewport);
            if (!point.Properties.IsLeftButtonPressed || (viewport.ScrollableWidth <= 0 && viewport.ScrollableHeight <= 0)) return;
            if (!canvas.CapturePointer(e.Pointer)) return;
            drag = (point.Position, viewport.HorizontalOffset, viewport.VerticalOffset);
            canvas.Dragging(true); e.Handled = true;
        };
        canvas.PointerMoved += (_, e) =>
        {
            if (drag is not { } start) return;
            var point = e.GetCurrentPoint(viewport).Position;
            viewport.ChangeView(start.X - (point.X - start.Point.X), start.Y - (point.Y - start.Point.Y), null, true);
            e.Handled = true;
        };
        void EndDrag(object sender, PointerRoutedEventArgs e)
        {
            if (drag == null) return;
            drag = null; canvas.Dragging(false); canvas.ReleasePointerCapture(e.Pointer); e.Handled = true;
        }
        canvas.PointerReleased += EndDrag; canvas.PointerCanceled += EndDrag; canvas.PointerCaptureLost += EndDrag;
        controls.Children.Add(minus); controls.Children.Add(percentage); controls.Children.Add(plus);
        Children.Add(controls); Children.Add(viewport); Resize(width, height);
    }
    public void Resize(double width, double height)
    {
        availableWidth = width; availableHeight = height;
        var size = NativeRasterSize.Fit(bitmap.PixelWidth, bitmap.PixelHeight, width, height, zoom);
        image.Width = size.ImageWidth; image.Height = size.ImageHeight;
        viewport.Width = size.CanvasWidth; viewport.Height = size.CanvasHeight;
        canvas.Width = Math.Max(size.CanvasWidth, size.ImageWidth); canvas.Height = Math.Max(size.CanvasHeight, size.ImageHeight);
        // Preserve the decoded image, zoom and scroll position when only the
        // viewport changes, including a different provenance tab.
    }
    private void SetZoom(int value)
    {
        zoom = NativeRasterSize.ClampZoom(value); percentageText.Text = $"{zoom}%";
        AutomationProperties.SetItemStatus(percentage, $"{zoom}%");
        minus.IsEnabled = zoom > NativeRasterSize.MinimumZoom; plus.IsEnabled = zoom < NativeRasterSize.MaximumZoom;
        minus.Opacity = minus.IsEnabled ? 1 : .4; plus.Opacity = plus.IsEnabled ? 1 : .4;
        Resize(availableWidth, availableHeight);
    }
    private sealed class RasterCanvas : Grid
    {
        public RasterCanvas() => Dragging(false);
        public void Dragging(bool value) => ProtectedCursor = InputSystemCursor.Create(value ? InputSystemCursorShape.SizeAll : InputSystemCursorShape.Hand);
    }
    public static async Task<FrameworkElement> CreateAsync(NativePanelFileContent content, string name, WispDesign design, double width, double height)
        => new NativeRasterPreview((BitmapImage)await NativeAttachmentThumbnail.DecodeAsync(content, 1600), name, design, width, height);
}
