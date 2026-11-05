namespace Wisp.ProjectBrowser;

public static class NativeHomePresentation
{
    // Keep the distinctive end of a workspace path. Measure the actual font;
    // do not split a CJK character, combining sequence or emoji at the cutoff.
    public static string FitPath(string path, double width, Func<string, double> measure)
    {
        if (measure(path) <= width) return path;
        const string ellipsis = "…";
        if (measure(ellipsis) > width) return "";
        var offsets = System.Globalization.StringInfo.ParseCombiningCharacters(path);
        int low = 0, high = offsets.Length;
        while (low < high)
        {
            var mid = low + (high - low) / 2;
            if (measure(ellipsis + path[offsets[mid]..]) <= width) high = mid; else low = mid + 1;
        }
        return low < offsets.Length ? ellipsis + path[offsets[low]..] : ellipsis;
    }

    // Matches wisp_dto::SessionStatusKind used by the WebView home cards.
    public static string Status(string? status) => status is "running" or "needs_you" ? status : "complete";
    public static string WindowTitle(string? projectName) => string.IsNullOrWhiteSpace(projectName)
        ? "wisp science" : $"wisp science — {projectName.Trim()}";
}
