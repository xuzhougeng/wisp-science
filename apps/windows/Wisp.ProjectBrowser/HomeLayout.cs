namespace Wisp.ProjectBrowser;

/// <summary>Home uses the WebView's spacing in client DIPs, independent of window height.</summary>
public readonly record struct HomeLayout(double SidePadding, double TopPadding, double ContentWidth,
    double BrandWidth, double ActionWidth, bool StackColumns, bool StackBrand, bool ActionsBelowBrand,
    bool StackActions)
{
    public const double MaxWidth = 1200;
    public const double WordmarkWidth = 180;
    public const double WordmarkHeight = 119;
    public const double HeaderGap = 24;
    public const double SectionGap = 32;
    public const double ColumnGap = 40;
    public static double QuickActionsWidth(double width) => 5 * ShortcutSize(width) + 4 * 8;

    public static double TaglineSize(double width) => Math.Clamp(double.IsFinite(width) ? width * .0135 : 15, 15, 18);
    // Like CSS text-wrap: balance, shorten the measure without adding a line.
    // The caller measures with the actual localized text and selected font.
    public static double BalancedTaglineWidth(double available, Func<double, double> heightAtWidth)
    {
        if (available <= 0 || !double.IsFinite(available)) return double.PositiveInfinity;
        var height = heightAtWidth(available);
        if (height <= heightAtWidth(double.PositiveInfinity)) return double.PositiveInfinity;
        double low = 0, high = available;
        while (high - low > 1)
        {
            var mid = (low + high) / 2;
            if (heightAtWidth(mid) <= height) high = mid; else low = mid;
        }
        return Math.Min(available, Math.Ceiling(high));
    }
    public static double ShortcutSize(double width) => width <= 820 ? 34 : 38;

    public static HomeLayout ForWidth(double width, double fontScale = 1, double? measuredMainActionsWidth = null)
    {
        width = double.IsFinite(width) ? Math.Max(0, width) : 0;
        fontScale = double.IsFinite(fontScale) ? Math.Clamp(fontScale, 12d / 14, 20d / 14) : 1;
        var padding = Math.Min(width / 2, Math.Clamp(width * .04, 24, 48));
        var content = Math.Min(MaxWidth, Math.Max(0, width - 2 * padding));
        var brand = 400d; // CSS flex-basis and minimum; the tagline wraps within it.
        var mainActions = measuredMainActionsWidth is > 0 && double.IsFinite(measuredMainActionsWidth.Value)
            ? measuredMainActionsWidth.Value : 426 * fontScale; // Research assistant, Import and New project.
        var quickActions = QuickActionsWidth(width);
        var below = width <= 820;
        var inlineActions = quickActions + 8 + mainActions;
        // CSS lets the brand grow into spare space after the action groups fit.
        // A fixed 400-DIP brand needlessly wraps the tagline on wide desktops.
        var available = below ? content : content - brand - HeaderGap;
        var stackActions = available < inlineActions;
        var actionWidth = below || stackActions ? available : inlineActions;
        if (!below && !stackActions) brand = content - HeaderGap - actionWidth;
        return new(padding, width <= 820 ? 28 : 40, content, Math.Min(brand, content), actionWidth,
            width <= 820, width <= 520, below, stackActions);
    }

    // The main action group is a single flex row. CSS distributes negative
    // space in proportion to each item's content box, leaving padding intact.
    public static double[] MainButtonWidths(IReadOnlyList<double> natural, IReadOnlyList<double> chrome, double budget)
    {
        var result = natural.ToArray();
        var total = result.Sum();
        var available = Math.Max(0, budget - 8 * Math.Max(0, result.Length - 1));
        if (total <= available || total <= 0) return result;
        var fixedWidth = chrome.Sum();
        if (available <= fixedWidth)
            return chrome.Select(w => fixedWidth == 0 ? 0 : w * available / fixedWidth).ToArray();
        var fraction = (available - fixedWidth) / (total - fixedWidth);
        for (var i = 0; i < result.Length; i++) result[i] = chrome[i] + (natural[i] - chrome[i]) * fraction;
        return result;
    }

    public double ListWidth => StackColumns ? ContentWidth : (ContentWidth - ColumnGap) / 2;
    public double CenteredInset(double viewportWidth) => Math.Max(0, (viewportWidth - ContentWidth) / 2);
}
