using Wisp.ProjectBrowser;

internal static class NativeSettingsLayoutTests
{
    public static void Run()
    {
        Check(NativeSettingsLayout.GeneralChoiceWidth(44.6) == 88
            && NativeSettingsLayout.GeneralChoiceWidth(171.3) == 215,
            "closed General pickers reserve room for the longest translated option and the chevron");
        Check(NativeSettingsLayout.ResizeArgumentBox(180, 76, -40, 72, 250) == (140d,148d)
            && NativeSettingsLayout.ResizeArgumentBox(180, 76, 100, -100, 200) == (200d,32d)
            && NativeSettingsLayout.ResizeArgumentBox(180, 240, 0, 90, 20) == (20d,330d),
            "ACP arguments resize on both axes and never push the form wider than its column");
        Check(NativeSettingsLayout.SearchHintMaxWidth(470, 380, 13) == 90
            && NativeSettingsLayout.SearchHintMaxWidth(600, 380, 13) == 220,
            "compact search feedback wraps in the space remaining beside the measured header");
        Check(NativeSettingsLayout.SearchHintMaxWidth(320, 380, 20) == 100
            && double.IsPositiveInfinity(NativeSettingsLayout.SearchHintMaxWidth(681, 380, 13)),
            "large type remains readable when scrolling is unavoidable; desktop navigation uses its column width");
        Check(NativeSettingsLayout.HookCommandHeight(54) == 62 && NativeSettingsLayout.HookCommandHeight(78) == 86,
            "hook commands retain three measured rows when a larger UI font increases the line box");
        Check(NativeSettingsLayout.HookCommandHeight(54.2) == 63 && NativeSettingsLayout.HookCommandHeight(0) == 32,
            "hook textareas round up fractional font metrics while respecting the native control minimum");
        foreach (var height in new[] { -1d, double.NaN, double.PositiveInfinity })
            Check(NativeSettingsLayout.HookCommandHeight(height) == 62,
                "unavailable font metrics keep a finite initial editor height");
        Check(NativeSettingsLayout.PetHero(1707, 944, true) == (false, 560d, false)
            && NativeSettingsLayout.PetHero(1707, 944, false) == (false, 560d, true),
            "at desktop widths only the empty preview distributes the space left over from measured copy");
        Check(NativeSettingsLayout.PetHero(721, 450, true) == (false, 310d, false)
            && NativeSettingsLayout.PetHero(720, 450, true) == (true, 450d, true)
            && NativeSettingsLayout.PetHero(720, 450, false) == (true, 450d, true),
            "pet copy reclaims the preview and gap width exactly at the WebView 720px stacking breakpoint");
        Check(NativeSettingsLayout.PetHero(320, 240, false) == (true, 240d, true)
            && NativeSettingsLayout.PetHero(800, 100, false) == (false, 0d, true),
            "pet copy fits the available row on narrow windows without a negative measure budget");
        foreach (var width in new[] { 0d, -1, double.NaN, double.PositiveInfinity })
            Check(NativeSettingsLayout.PetHero(800, width, true).CopyWidth == 0,
                "unmeasured pet heroes never assign an invalid maximum text width");
        Check(Math.Abs(NativeSettingsLayout.PetGlowRadius(1000, 200) - 307.836839901) < .001,
            "pet glow ends at 36 percent of the farthest corner, preserving a circular DIP radius");
        Check(NativeSettingsLayout.PetGlowRadius(500, 100) == NativeSettingsLayout.PetGlowRadius(1000, 200) / 2,
            "pet glow scales with the card without stretching its axes independently");
        foreach (var size in new[] { (0d, 200d), (100d, -1d), (double.NaN, 200d), (100d, double.PositiveInfinity) })
            Check(NativeSettingsLayout.PetGlowRadius(size.Item1, size.Item2) == 0,
                "unmeasured pet cards do not produce invalid brush radii");
        foreach (var width in new[] { 575d, 680, 700, 701, 1707 })
            Check(NativeSettingsLayout.SubscriptionHeadingStacks(width) == (width <= 700),
                "subscription account status moves below its description at the 700px WebView breakpoint");
        Check(!NativeSettingsLayout.SubscriptionHeadingStacks(0) && !NativeSettingsLayout.SubscriptionHeadingStacks(double.NaN),
            "subscription headers wait for a measured viewport before stacking");
        Check(NativeSettingsLayout.ReservesOverflowScrollbar("models") && NativeSettingsLayout.ReservesOverflowScrollbar("network")
            && NativeSettingsLayout.ReservesOverflowScrollbar("channels")
            && !NativeSettingsLayout.ReservesOverflowScrollbar("general") && !NativeSettingsLayout.ReservesOverflowScrollbar("session"),
            "scrolling model subpages reserve a gutter while permanently reserved preference panes keep their existing policy");
        Check(NativeSettingsLayout.PreferenceMenuAvailableHeight(760, 420, 36) == 417,
            "a picker near the lower half uses the larger available height budget");
        Check(NativeSettingsLayout.PreferenceMenuAvailableHeight(567, 500, 36) == 497,
            "a bottom-edge picker keeps a finite height budget without shrinking to the remaining bottom sliver");
        Check(NativeSettingsLayout.PreferenceMenuAvailableHeight(567, 0, 36) == 528,
            "a top-edge picker reserves its field and the three-pixel gap below");
        foreach (var top in new[] { -100d, 0, 50, 500, 900 })
            Check(NativeSettingsLayout.PreferenceMenuAvailableHeight(567, top, 36) is >= 0 and <= 567,
                "scrolling or offscreen owners never produce a menu budget outside the viewport");
        Check(NativeSettingsLayout.PreferenceMenuAvailableHeight(0, 0, 36) == 0
            && NativeSettingsLayout.PreferenceMenuAvailableHeight(double.NaN, 0, 36) == 0,
            "unmeasured dropdowns do not receive invalid sizes");
        // Pointer/keyboard focus can leave the owner while a native dropdown is
        // open. Escape returns focus to the owner rather than clearing its ring.
        foreach (var state in new[] { (Focus: true, Open: false), (Focus: false, Open: true), (Focus: true, Open: false) })
            Check(NativeSettingsLayout.PreferenceFocusVisible(true, state.Focus, state.Open),
                "opening and dismissing a picker preserves the owning field's focus treatment");
        Check(!NativeSettingsLayout.PreferenceFocusVisible(true, false, false), "a blurred field has no focus ring");
        foreach (var focused in new[] { false, true })
            foreach (var open in new[] { false, true })
                Check(!NativeSettingsLayout.PreferenceFocusVisible(false, focused, open),
                    "disabling a field removes its focus treatment even during popup focus transfer");
        Check(!NativeSettingsLayout.ShowsProjectScope("connections") && !NativeSettingsLayout.ShowsProjectScope("general")
            && !NativeSettingsLayout.ShowsProjectScope("storage")
            && NativeSettingsLayout.ShowsProjectScope("project") && NativeSettingsLayout.ShowsProjectScope("permissions"),
            "connections retain their opened context without inserting a scope selector into shared settings navigation");
        Check(NativeSettingsLayout.SkillDocumentHeight(640, 446, 24, 22) == 172,
            "skill document uses the remaining viewport beneath its metadata and toolbar");
        Check(NativeSettingsLayout.SkillDocumentHeight(430, 520, 24, 22) == 170
            && NativeSettingsLayout.SkillDocumentHeight(430, 520, 14, 22) == 150,
            "short skill pages retain 120 readable pixels and leave metadata reachable through the outer scrollbar");
        Check(NativeSettingsLayout.SkillListHeight(640, 218, 22) == 400
            && NativeSettingsLayout.SkillListHeight(430, 350, 22) == 120,
            "skills list retains a scrollable row area beneath fixed controls, including short windows");
        foreach (var width in new[] { 320d, 520, 801, 1100, 1707, 2560 })
        {
            var layout = NativeSettingsLayout.ForWidth(width);
            Check(width <= 680 ? layout.NavigationWidth == 0 : layout.NavigationWidth >= 188 && layout.NavigationWidth <= 240,
                "compact navigation moves above content; desktop navigation follows WebView width limits");
            Check(layout.ContentWidth > 0 && layout.ContentWidth <= 1040 && layout.NavigationWidth + layout.ContentWidth <= width,
                "settings content has a finite width inside its own column");
        }
        Check(NativeSettingsLayout.ForWidth(1100) == new NativeSettingsLayout(198, 902), "reference window keeps all fields visible beside navigation");
        Check(NativeSettingsLayout.ForWidth(1707) == new NativeSettingsLayout(240, 1040), "wide settings pages cap and center their content");
        Check(NativeSettingsLayout.HorizontalNavigation(680) && !NativeSettingsLayout.HorizontalNavigation(681)
            && !NativeSettingsLayout.HorizontalNavigation(double.NaN), "navigation uses the WebView 680px breakpoint without applying it before measurement");
        Check(NativeSettingsLayout.ForWidth(640) == new NativeSettingsLayout(0, 640),
            "compact settings use the complete viewport width beneath the navigation");
        Check(!NativeSettingsLayout.GeneralColumns(902) && NativeSettingsLayout.GeneralColumns(902.82),
            "General changes from stacked groups to columns at the same viewport breakpoint as WebView");
        NativeActionLayout.Slot[] Proxy(double width, bool custom) => NativeSettingsLayout.NetworkProxySlots(width,
            new(112, 36), custom ? new(130, 36) : null, new(50, 30), new(50, 30));
        var standard = Proxy(250, false);
        Check(standard[1].X == 142 && standard[2].X == 200 && standard[1].Y == 3,
            "Clear and Save align right and center vertically beside the actual translated picker");
        var custom = Proxy(400, true);
        Check(custom[1].Width == 164 && custom[2].X == 292 && custom[3].X == 350,
            "the custom address absorbs all free space before Clear and Save");
        var split = Proxy(350, true);
        Check(split[1].Width == 172 && split[2].Y == 3 && split[3].X == 0 && split[3].Y == 44,
            "Save alone wraps when there is still room for Clear after the address");
        var small = Proxy(180, true);
        Check(small[1].Width == 180 && small[1].Y == 44 && small[2].X == 72 && small[3].X == 130,
            "the address and the right-aligned actions take separate lines on narrow windows");
        Check(!NativeSettingsLayout.BrowserFieldsStack(414) && !NativeSettingsLayout.BrowserFieldsStack(510)
            && NativeSettingsLayout.BrowserFieldsStack(240), "browser inputs stay on one row in both normal collection columns and stack only when cramped");
        Check(NativeSettingsLayout.ConnectionCompact(645) && NativeSettingsLayout.ConnectionCompact(700)
            && !NativeSettingsLayout.ConnectionCompact(701) && !NativeSettingsLayout.ConnectionCompact(double.NaN),
            "connector titles and documentation inset use the WebView 700px viewport breakpoint");
        Check(NativeSettingsLayout.SessionNumberWidth(654) == 90 && NativeSettingsLayout.SessionNumberWidth(850) == 90
            && NativeSettingsLayout.SessionNumberWidth(851) == 112 && NativeSettingsLayout.SessionNumberWidth(1707) == 112,
            "session number inputs reserve 90px at the compact viewport breakpoint and 112px on desktop");
        Check(!NativeSettingsLayout.QuickActionsStack(801) && !NativeSettingsLayout.QuickActionsStack(721)
            && NativeSettingsLayout.QuickActionsStack(720), "Quick Action rows follow the WebView window breakpoint, not the narrower settings column");
        foreach (var width in new[] { 320d, 653, 720, 721, 801, 1707 })
        {
            var cells = NativeSettingsLayout.QuickActionCells(width);
            Check(cells.ActionRow is not (1 or 2), "action controls never occupy the full-width description or workflow-reference rows");
            Check(width <= 720
                ? cells.TitleSpan == 2 && cells.ActionRow > 2 && cells.ActionColumn == 0 && cells.ActionSpan == 2
                : cells.TitleSpan == 1 && cells.ActionRow == 0 && cells.ActionColumn >= cells.TitleSpan && cells.ActionSpan == 1,
                "long quick-action copy and controls have nonoverlapping cells on each side of the breakpoint");
        }
        Check(!NativeSettingsLayout.DefinitionFieldsStack(801) && !NativeSettingsLayout.DefinitionFieldsStack(681)
            && NativeSettingsLayout.DefinitionFieldsStack(680), "definition fields use the shared form-grid breakpoint");
        Check(NativeSettingsLayout.ModelFormColumns(681) && !NativeSettingsLayout.ModelFormColumns(680),
            "model editors keep two columns above the whole-window 680px breakpoint");
        Check(NativeSettingsLayout.ForWidth(double.NaN) == new NativeSettingsLayout(0, 0), "unmeasured controls have no invalid dimensions");
        Check(!NativeSettingsLayout.AppearanceColumns(1100) && NativeSettingsLayout.AppearanceColumns(1101),
            "appearance preview moves below the controls at the WebView breakpoint");
        var wideSample = NativeSettingsLayout.ThemePreviewLines(160, 96);
        var narrowSample = NativeSettingsLayout.ThemePreviewLines(80, 48);
        Check(Math.Abs(wideSample.Gap - 11.52) < .01 && Math.Abs(wideSample.Stroke - 4.608) < .01
            && narrowSample.InsetY * 2 == wideSample.InsetY && narrowSample.InsetX * 2 == wideSample.InsetX
            && narrowSample.Stroke == 3, "theme thumbnail art scales with its own preview while fine strokes remain visible");
        Check(NativeSettingsLayout.AppearanceBodyWidth(1707) == 1030
            && NativeSettingsLayout.AppearanceBodyWidth(1100) == 892
            && NativeSettingsLayout.AppearanceBodyWidth(640) == 630,
            "appearance reserves its inner scrollbar on desktop, beside navigation and below horizontal navigation");
        Check(NativeSettingsLayout.AppearanceBodyWidth(0) == 0 && NativeSettingsLayout.AppearanceBodyWidth(double.NaN) == 0,
            "unmeasured appearance layout never produces a negative width");
        Check(NativeSettingsLayout.AppearanceCardPadding(850) == (16, 18)
            && NativeSettingsLayout.AppearanceCardPadding(851) == (24, 22)
            && NativeSettingsLayout.AppearanceCardPadding(1707) == (24, 22),
            "theme, fonts and preview cards share the CSS compact and desktop insets");
        Check(!NativeSettingsLayout.RemoteColumns(1100) && NativeSettingsLayout.RemoteColumns(1101)
            && !NativeSettingsLayout.RemoteColumns(double.NaN),
            "remote access overview stacks at the WebView 1100px window breakpoint, including unmeasured layouts");
        Check(NativeSettingsLayout.ChannelFieldsStack(520) && !NativeSettingsLayout.ChannelFieldsStack(521)
            && !NativeSettingsLayout.ChannelFieldsStack(645), "channel field rows use the 520px window breakpoint instead of stacking inside the narrower sync card");
        foreach (var width in new[] { 500d, 520, 521, 575, 680, 681, 1707 })
        {
            Check(NativeSettingsLayout.ChannelFieldsStack(width, formGrid: true) == (width <= 680),
                "device address/port and lab name fields stack at the shared form breakpoint");
            Check(NativeSettingsLayout.ChannelFieldsStack(width) == (width <= 520),
                "device mode cards and channel action rows retain their distinct narrower breakpoint");
        }
        foreach (var width in new[] { 0d, -1, double.NaN, double.PositiveInfinity })
            Check(!NativeSettingsLayout.ChannelFieldsStack(width) && !NativeSettingsLayout.ChannelFieldsStack(width, true),
                "unmeasured channel rows do not apply a compact layout from invalid dimensions");
        Check(!NativeSettingsLayout.AppearanceFieldsStack(414) && NativeSettingsLayout.AppearanceFieldsStack(240),
            "font controls keep their horizontal labels until their own row cannot fit");
        Check(Math.Abs(NativeSettingsLayout.AppearanceControlWidth(803, .55) - 441.65) < .01
            && NativeSettingsLayout.AppearanceControlWidth(800, .48) == 384
            && NativeSettingsLayout.AppearanceControlWidth(240, .55) == 0,
            "palette and font controls use the CSS row width, leaving the full width for stacked fields");
        Check(!NativeSettingsLayout.AppearanceDiffColumns(1101) && NativeSettingsLayout.AppearanceDiffColumns(1100)
            && NativeSettingsLayout.AppearanceDiffColumns(680) && NativeSettingsLayout.AppearanceDiffColumns(588)
            && !NativeSettingsLayout.AppearanceDiffColumns(0),
            "the full-width preview pairs diff samples even below the horizontal navigation breakpoint");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
