namespace Wisp.ProjectBrowser;

public sealed record NativeRasterSize(double CanvasWidth, double CanvasHeight, double ImageWidth, double ImageHeight)
{
    public const int MinimumZoom = 25, MaximumZoom = 400, ZoomStep = 25;
    public static int ClampZoom(int value) => Math.Clamp(value, MinimumZoom, MaximumZoom);
    public static int StepZoom(int value, int direction) => ClampZoom(value + Math.Sign(direction) * ZoomStep);

    public static NativeRasterSize Fit(double pixelWidth, double pixelHeight, double width, double height, int zoom = 100)
    {
        if (pixelWidth <= 0 || pixelHeight <= 0) throw new InvalidDataException("Image dimensions are unavailable");
        // WebView's 100% is fit-to-width, including tall images that require
        // vertical panning. The toolbar is 28px high with an 8px gap.
        var canvasWidth = Math.Max(1, width); var canvasHeight = Math.Max(1, height - 36);
        var scale = canvasWidth / pixelWidth * ClampZoom(zoom) / 100;
        return new(canvasWidth, canvasHeight, pixelWidth * scale, pixelHeight * scale);
    }
}
