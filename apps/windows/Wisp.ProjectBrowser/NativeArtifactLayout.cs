namespace Wisp.ProjectBrowser;

/// <summary>ArtifactModal viewport limits from modals.css, excluding native dialog chrome.</summary>
public sealed record NativeArtifactLayout(double Width, double MaxHeight, double FigureHeight, double DetailsHeight)
{
    public const double Inset = 46; // 22px padding and 1px border on each side.
    public static NativeArtifactLayout Fit(double viewportWidth, double viewportHeight, string kind,
        double chromeHeight, double detailsDesiredHeight)
    {
        var html = kind == "html";
        var width = Math.Max(0, Math.Min(Math.Min(html ? 1240 : 880, viewportWidth * (html ? .97 : .94)), viewportWidth - 40) - Inset);
        var height = Math.Max(0, viewportHeight - 40 - Inset);
        var available = Math.Max(0, height - chromeHeight);
        var details = Math.Min(Math.Min(viewportHeight * .3, Math.Max(0, detailsDesiredHeight)), available);
        var figure = Math.Min(viewportHeight * (kind is "docx" or "xlsx" or "pptx" ? .66 : .52), available - details);
        return new(width, height, Math.Max(0, figure), Math.Max(0, details));
    }
}
