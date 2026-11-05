using Wisp.ProjectBrowser;

internal static class NativeHomeLayoutTests
{
    public static void Run()
    {
        foreach (var width in new[] { 320d, 520, 600, 800, 820, 821, 883, 1024, 1100, 1280, 1440, 1920, 3440 })
        foreach (var scale in new[] { 12d / 14, 1, 20d / 14 })
        {
            var home = HomeLayout.ForWidth(width, scale);
            Check(home.ContentWidth + 2 * home.SidePadding <= width && home.ContentWidth <= 1200,
                "home stays inside the client viewport and retains its desktop content limit");
            Check(home.ListWidth > 0 && home.ActionWidth <= home.ContentWidth,
                "both list and action budgets stay usable");
            Check(Math.Abs(home.CenteredInset(width) * 2 + home.ContentWidth - width) < .01,
                "explicit page width leaves equal margins after restoring and maximizing");
            if (!home.StackColumns)
                Check(Math.Abs(2 * home.ListWidth + 40 - home.ContentWidth) < .01,
                    "project and recent-session columns have equal widths");
            if (!home.ActionsBelowBrand)
                Check(home.BrandWidth + 24 + home.ActionWidth <= home.ContentWidth + .01
                    && home.ActionWidth >= HomeLayout.QuickActionsWidth(width),
                    "actions beside the wordmark never squeeze or overlap the brand");
            var quick = NativeActionLayout.Arrange(Enumerable.Repeat(new NativeActionLayout.Size(38, 38), 5).ToArray(), home.ActionWidth);
            Check(quick.All(s => s.X + s.Width <= home.ActionWidth + .01), "all shortcuts remain inside their available row");
        }

        Check(HomeLayout.ForWidth(820).StackColumns && !HomeLayout.ForWidth(821).StackColumns,
            "home uses the same 820-DIP single-column boundary as WebView");
        Check(HomeLayout.ForWidth(520).StackBrand && !HomeLayout.ForWidth(521).StackBrand,
            "very narrow windows move the tagline below the full wordmark");
        var desktop = HomeLayout.ForWidth(900);
        Check(!desktop.ActionsBelowBrand && desktop.StackActions && !HomeLayout.ForWidth(1103).ActionsBelowBrand && HomeLayout.ForWidth(1103).StackActions,
            "on intermediate desktops icons and project commands occupy two rows beside the wordmark");
        Check(!HomeLayout.ForWidth(1440).StackActions, "wide desktops fit both action groups on one row");
        var measured = HomeLayout.ForWidth(1707, measuredMainActionsWidth: 470);
        Check(measured.BrandWidth == 476 && measured.ActionWidth == 700 && !measured.StackActions,
            "the brand receives unused desktop space after measured localized actions fit");
        Check(HomeLayout.ForWidth(1202, measuredMainActionsWidth: 453).StackActions
            && !HomeLayout.ForWidth(1203, measuredMainActionsWidth: 453).StackActions
            && HomeLayout.ForWidth(1203, measuredMainActionsWidth: 453.5).StackActions,
            "five desktop shortcuts take exactly 222px; wrap against the actual fractional button group width");
        var natural = new[] { 170d, 140, 130 };
        var chrome = new[] { 26d, 26, 34 };
        var compact = HomeLayout.ForWidth(842, measuredMainActionsWidth: natural.Sum() + 16);
        var buttons = HomeLayout.MainButtonWidths(natural, chrome, compact.ActionWidth);
        Check(!compact.ActionsBelowBrand && compact.StackActions && compact.BrandWidth == 400
            && Math.Abs(buttons.Sum() + 16 - compact.ActionWidth) < .01
            && buttons.Zip(natural).All(pair => pair.First < pair.Second)
            && buttons.Zip(chrome).All(pair => pair.First > pair.Second),
            "at the observed 842-DIP viewport the whole primary group fits without moving or dropping a button");
        Check(HomeLayout.MainButtonWidths(natural, chrome, 600).SequenceEqual(natural)
            && HomeLayout.ForWidth(780).ActionsBelowBrand && !HomeLayout.ForWidth(780).StackActions,
            "single-column windows use the full width below the brand; roomy buttons keep their natural widths");
        Check(!HomeLayout.ForWidth(821).ActionsBelowBrand && HomeLayout.ForWidth(820).ActionsBelowBrand,
            "the header and lists change arrangement at the same breakpoint");

        static double Glyphs(string s) => new System.Globalization.StringInfo(s).LengthInTextElements;
        const string path = @"C:\Users\review\workspace\研究🧪";
        Check(NativeHomePresentation.FitPath(path, 999, Glyphs) == path
            && NativeHomePresentation.FitPath(path, 4, Glyphs) == "…研究🧪"
            && NativeHomePresentation.FitPath("abc-e\u0301", 2, Glyphs) == "…e\u0301"
            && NativeHomePresentation.FitPath(path, 0, Glyphs) == "",
            "a narrowed project path retains its identifying suffix and whole graphemes; widening restores the original");
        Check(HomeLayout.ForWidth(1103, measuredMainActionsWidth: 470).StackActions,
            "a narrower desktop wraps groups without stealing the minimum brand width");
        Check(!HomeLayout.ForWidth(1103, measuredMainActionsWidth: 800).ActionsBelowBrand,
            "long translated action labels wrap within the right-hand group");
        Check(HomeLayout.ForWidth(1707, measuredMainActionsWidth: double.NaN) == HomeLayout.ForWidth(1707),
            "unmeasured action widths retain the stable fallback");
        Check(HomeLayout.TaglineSize(800) == 15 && Math.Abs(HomeLayout.TaglineSize(1200) - 16.2) < .001
            && HomeLayout.TaglineSize(1707) == 18 && HomeLayout.TaglineSize(double.NaN) == 15,
            "the tagline follows the WebView viewport clamp, including before measurement");
        static double TaglineLines(double width)
        {
            var lines = 1; double used = 0;
            foreach (var word in "with Wisp Science by your side.".Split(' '))
            {
                if (used > 0 && used + 1 + word.Length > width) { lines++; used = 0; }
                used += (used > 0 ? 1 : 0) + word.Length;
            }
            return lines;
        }
        var balanced = HomeLayout.BalancedTaglineWidth(26, TaglineLines);
        Check(balanced >= 17 && balanced < 20 && TaglineLines(balanced) == 2
            && double.IsPositiveInfinity(HomeLayout.BalancedTaglineWidth(42, TaglineLines)),
            "two lines keep 'by your side.' together; a tagline that fits stays on one line");
        Check(HomeLayout.ShortcutSize(820) == 34 && HomeLayout.ShortcutSize(821) == 38,
            "home shortcuts use the WebView narrow-screen target size");
        Check(HomeLayout.ForWidth(1707).CenteredInset(1707) == 253.5,
            "reported maximized home is centered rather than shifted to the right");
        Check(HomeLayout.ForWidth(883, 20d / 14).BrandWidth == 400 && HomeLayout.ForWidth(883, 20d / 14).StackActions,
            "a larger font can wrap without changing the 400-DIP brand basis");
        Check(HomeLayout.ForWidth(1350 / 1.5) == desktop && HomeLayout.ForWidth(1800 / 2d) == desktop,
            "the same effective client size has identical geometry at 150 and 200 percent DPI");
        Check(HomeLayout.ForWidth(double.NaN).ContentWidth == 0 && HomeLayout.ForWidth(-10).ContentWidth == 0,
            "unmeasured viewports cannot produce invalid grid dimensions");
        Console.WriteLine("Native home responsive layout budgets passed.");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
