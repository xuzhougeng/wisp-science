namespace Wisp.ProjectBrowser;

/// <summary>Finite layout budgets shared by every native settings category.</summary>
public readonly record struct NativeSettingsLayout(double NavigationWidth, double ContentWidth)
{
    public static (double Width, double Height) ResizeArgumentBox(double width, double height, double dx, double dy, double availableWidth)
        => (Math.Min(availableWidth, Math.Max(32, width + dx)), Math.Max(32, height + dy));
    // Connections inherit the opened project's context, like WebView. The picker
    // is reserved for native pages whose content explicitly compares scopes.
    public static bool ShowsProjectScope(string section) => section is "project" or "permissions" or "workflow-conversion";
    public static bool HorizontalNavigation(double viewportWidth) => double.IsFinite(viewportWidth) && viewportWidth > 0 && viewportWidth <= 680;
    // The compact navigation scrolls as a row, so its last child would otherwise
    // measure at infinite width. Reserve a few characters if even the header
    // barely fits, rather than rendering the message one character per line.
    public static double SearchHintMaxWidth(double viewportWidth, double occupiedWidth, double fontSize)
        => HorizontalNavigation(viewportWidth)
            ? Math.Max(5 * fontSize, viewportWidth - occupiedWidth)
            : double.PositiveInfinity;
    // The WebView switches at a 1100px viewport, where its navigation is 198px.
    public static bool GeneralColumns(double availableContentWidth) => availableContentWidth > 902;
    // Auto-width select: measure all localized option labels, then reserve the
    // WebView field's 11/30px padding and two 1px borders. The row caps MaxWidth.
    public const double GeneralChoiceArrow = 30;
    public static double GeneralChoiceWidth(double longestOptionWidth) => Math.Ceiling(longestOptionWidth) + 11 + GeneralChoiceArrow + 2;
    // Match .network-proxy-controls: form lines at each item's natural width,
    // grow the address on its line, then apply Clear's leading auto margin.
    public const double NetworkAddressBasis = 130;
    public static NativeActionLayout.Slot[] NetworkProxySlots(double width, NativeActionLayout.Size picker,
        NativeActionLayout.Size? address, NativeActionLayout.Size clear, NativeActionLayout.Size save)
    {
        NativeActionLayout.Size[] sizes = address is { } field ? [picker, field with { Width = NetworkAddressBasis }, clear, save] : [picker, clear, save];
        var slots = NativeActionLayout.Arrange(sizes, width);
        for (var start = 0; start < slots.Length;)
        {
            var end = start + 1;
            while (end < slots.Length && slots[end].Y == slots[start].Y) end++;
            var last = slots[end - 1];
            var free = double.IsFinite(width) ? Math.Max(0, width - last.X - last.Width) : 0;
            var height = slots[start..end].Max(s => s.Height);
            var grows = address.HasValue && start <= 1 && 1 < end;
            var clearIndex = slots.Length - 2;
            for (var i = start; i < end; i++)
            {
                var s = slots[i];
                var shift = grows ? i > 1 : start <= clearIndex && i >= clearIndex;
                slots[i] = s with { X = s.X + (shift ? free : 0), Y = s.Y + (height - s.Height) / 2,
                    Width = s.Width + (grows && i == 1 ? free : 0) };
            }
            start = end;
        }
        return slots;
    }
    public static bool BrowserFieldsStack(double fieldsWidth) => fieldsWidth < 300;
    // CSS radial-gradient(circle at 16% 20%, ..., transparent 36%) uses
    // the farthest corner as its radius before applying the 36% color stop.
    public static double PetGlowRadius(double width, double height)
        => double.IsFinite(width) && double.IsFinite(height) && width > 0 && height > 0
            ? Math.Sqrt(Math.Pow(width * .84, 2) + Math.Pow(height * .8, 2)) * .36 : 0;
    // Only an empty WebView preview has auto margins. Its column absorbs spare
    // space around the 116px placeholder; a configured sprite stays beside its copy.
    public static (bool Stacked, double CopyWidth, bool FlexiblePreview) PetHero(double viewportWidth, double contentWidth, bool hasAsset)
    {
        var stacked = double.IsFinite(viewportWidth) && viewportWidth > 0 && viewportWidth <= 720;
        var available = double.IsFinite(contentWidth) ? Math.Max(0, contentWidth) : 0;
        return (stacked, Math.Clamp(available - (stacked ? 0 : 140), 0, 560), stacked || !hasAsset);
    }
    public static bool ConnectionCompact(double viewportWidth) => double.IsFinite(viewportWidth) && viewportWidth > 0 && viewportWidth <= 700;
    public static double SessionNumberWidth(double viewportWidth) => viewportWidth <= 850 ? 90 : 112;
    public static bool ModelFormColumns(double viewportWidth) => viewportWidth > 680;
    public static bool SubscriptionHeadingStacks(double viewportWidth) => double.IsFinite(viewportWidth) && viewportWidth > 0 && viewportWidth <= 700;
    public static bool ReservesOverflowScrollbar(string section) => section is "network" or "permissions" or "credentials" or "environments" or "models" or "channels";
    public static bool QuickActionsStack(double viewportWidth) => viewportWidth <= 720;
    public readonly record struct QuickActionCellLayout(int TitleSpan, int ActionRow, int ActionColumn, int ActionSpan);
    // Descriptions and workflow references always span both columns. Only the
    // controls move below them when the window enters the stacked layout.
    public static QuickActionCellLayout QuickActionCells(double viewportWidth)
        => QuickActionsStack(viewportWidth) ? new(2, 3, 0, 2) : new(1, 0, 1, 1);
    public static bool DefinitionFieldsStack(double viewportWidth) => viewportWidth <= 680;
    // Hooks use a three-row textarea. Its measured line box grows with the UI
    // font; the old fixed height clipped a row at larger accessibility sizes.
    public static double HookCommandHeight(double threeLineHeight)
        => double.IsFinite(threeLineHeight) && threeLineHeight >= 0
            ? Math.Max(32, Math.Ceiling(threeLineHeight + 8)) : 62;
    public static bool AppearanceColumns(double viewportWidth) => viewportWidth > 1100;
    // WebView's h2 keeps its browser margin of .83em when the UI font changes.
    public static double AppearanceHeadingMargin(double headingFontSize) => .83 * headingFontSize;
    // settings.css .theme-preview-content: percentage padding (also vertical)
    // is relative to column width. The three text strokes use the inner box.
    public static (double InsetX, double InsetY, double Gap, double Stroke) ThemePreviewLines(double columnWidth, double height)
    {
        var x = columnWidth * .1; var y = columnWidth * .12;
        return (x, y, (columnWidth - 2 * x) * .09, Math.Max(3, (height - 2 * y) * .08));
    }
    // Opening a native picker can move keyboard focus into its popup. Keep the
    // owning field highlighted until it closes; disabled fields never highlight.
    public static bool PreferenceFocusVisible(bool enabled, bool fieldFocused, bool pickerOpen)
        => enabled && (fieldFocused || pickerOpen);
    // Cap long menus with a finite window-sized budget. Desktop popup placement
    // can extend past a short app window, just like the WebView native select.
    public static double PreferenceMenuAvailableHeight(double viewportHeight, double fieldTop, double fieldHeight)
    {
        if (!double.IsFinite(viewportHeight) || !double.IsFinite(fieldTop) || !double.IsFinite(fieldHeight)) return 0;
        var height = Math.Max(0, viewportHeight);
        var above = Math.Clamp(fieldTop - 3, 0, height);
        var below = Math.Clamp(height - fieldTop - Math.Max(0, fieldHeight) - 3, 0, height);
        return Math.Max(above, below);
    }
    // The CSS settings pane owns a 10px scrollbar inside its centered 1040px box.
    public static double AppearanceBodyWidth(double viewportWidth)
        => Math.Max(0, ForWidth(viewportWidth).ContentWidth - 10);
    public static (double Horizontal, double Vertical) AppearanceCardPadding(double viewportWidth)
        => viewportWidth > 850 ? (24, 22) : (16, 18);
    public static bool RemoteColumns(double viewportWidth) => viewportWidth > 1100;
    // Form grids stack with the settings form at 680px; bind/action rows and
    // device-mode cards use the narrower channel-specific 520px breakpoint.
    public static bool ChannelFieldsStack(double viewportWidth, bool formGrid = false)
        => double.IsFinite(viewportWidth) && viewportWidth > 0 && viewportWidth <= (formGrid ? 680 : 520);
    // The skill document owns its scrollbar. Keep enough readable content when
    // metadata exceeds a short page; the outer viewport can scroll to the file.
    public static double SkillDocumentHeight(double viewportHeight, double documentTop, double padding, double bottomPadding)
        => Math.Max(padding * 2 + 122, Math.Floor(viewportHeight - documentTop - bottomPadding));
    public static double SkillListHeight(double viewportHeight, double listTop, double bottomPadding)
        => Math.Max(120, Math.Floor(viewportHeight - listTop - bottomPadding));
    // A font row needs room for both its hint and a slider plus the 44px output.
    public static bool AppearanceFieldsStack(double rowWidth) => rowWidth < 300;
    // CSS percentage fields use the full row width, before its 14px flex gap.
    public static double AppearanceControlWidth(double rowWidth, double fraction)
        => double.IsFinite(rowWidth) && !AppearanceFieldsStack(rowWidth) ? rowWidth * fraction : 0;
    // Once the live preview has its own row, WebView keeps the before/after
    // snippets side by side, including below the horizontal-navigation breakpoint.
    public static bool AppearanceDiffColumns(double viewportWidth) => viewportWidth is > 0 and <= 1100;
    public static NativeSettingsLayout ForWidth(double width)
    {
        width = double.IsFinite(width) ? Math.Max(0, width) : 0;
        var navigation = HorizontalNavigation(width) ? 0 : Math.Min(width, Math.Clamp(width * .18, 188, 240));
        return new(navigation, Math.Min(1040, Math.Max(0, width - navigation)));
    }
}
