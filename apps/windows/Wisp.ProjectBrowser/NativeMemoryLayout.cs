using System.Globalization;

namespace Wisp.ProjectBrowser;

public static class NativeMemoryLayout
{
    public static bool UseTwoColumns(double viewport) => double.IsFinite(viewport) && viewport > 1100;
    // The footer has its own grid row. Reserve 30px above the editor and the
    // WebView's 10px gap before the footer, without subtracting the footer twice.
    public static double EditorHeight(double bodyHeight) => Math.Max(240, double.IsFinite(bodyHeight) ? bodyHeight - 40 : 240);
    public static string FormatBytes(long bytes) => bytes < 1024 ? $"{Math.Max(0, bytes)} B" : bytes < 1024 * 1024
        ? (bytes / 1024d).ToString("0.0", CultureInfo.InvariantCulture) + " KB"
        : (bytes / (1024d * 1024)).ToString("0.0", CultureInfo.InvariantCulture) + " MB";
}
