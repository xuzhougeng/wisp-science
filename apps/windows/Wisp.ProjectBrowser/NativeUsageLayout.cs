namespace Wisp.ProjectBrowser;

public static class NativeUsageLayout
{
    public static int SummaryColumns(double width) => double.IsFinite(width) && width > 0 ? Math.Clamp((int)Math.Floor((width + 10) / 120), 1, 4) : 1;
    public static bool OverviewColumns(double viewport, double content) => viewport > 1050 && content >= 824;
    public static bool ModelBesideLegend(double viewport) => viewport > 720 && viewport <= 1050;
    public static bool Compact(double viewport) => viewport <= 720;
    public static double TableMinimum(double viewport) => Compact(viewport) ? 402 : 440;
}
