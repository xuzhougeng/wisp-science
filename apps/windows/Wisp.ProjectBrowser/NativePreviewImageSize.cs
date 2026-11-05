namespace Wisp.ProjectBrowser;

/// <summary>Fit the original image within the available width and height without enlarging it.</summary>
public static class NativePreviewImageSize
{
    public static double Height(double availableWidth, double width, double height, double maximumHeight)
    {
        if (!double.IsFinite(availableWidth) || !double.IsFinite(width) || !double.IsFinite(height)
            || !double.IsFinite(maximumHeight) || width <= 0 || height <= 0 || maximumHeight <= 0)
            return 0;
        return Math.Min(maximumHeight, height * Math.Min(1, Math.Max(0, availableWidth) / width));
    }
}
