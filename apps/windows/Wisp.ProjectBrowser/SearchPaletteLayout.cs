namespace Wisp.ProjectBrowser;

/// <summary>Bounds for the WebView-style top-aligned search and command palettes.</summary>
public readonly record struct SearchPaletteLayout(double Width, double MaxHeight, double Top)
{
    public static bool ListOverflows(double scrollableHeight) => scrollableHeight > 1;
    // Index actionable rows only: section headings never take the keyboard cursor.
    public static int StepChoice(int current, int count, bool forward) => count <= 0 ? -1 :
        current < 0 || current >= count ? (forward ? 0 : count - 1) : (current + (forward ? 1 : -1) + count) % count;

    public static Contracts.NativeSearchItem? Attachment(Contracts.NativeSearchItem? selected, IEnumerable<Contracts.NativeSearchItem> matches)
        => selected?.Kind is "artifact" or "session" ? selected : matches.FirstOrDefault(item => item.Kind is "artifact" or "session");

    public static SearchPaletteLayout ForViewport(double width, double height, bool commandsOnly = false)
    {
        width = double.IsFinite(width) ? Math.Max(0, width) : 0;
        height = double.IsFinite(height) ? Math.Max(0, height) : 0;
        var narrow = width <= 820;
        var inset = narrow ? 12 : 16;
        var preferred = narrow ? 560 : commandsOnly ? 520 : 720;
        var top = Math.Min(height / 2, Math.Clamp(height * .1, 12, 72));
        var available = Math.Max(0, height - top - Math.Min(narrow ? 16 : 20, height / 2));
        return new(Math.Min(preferred, Math.Max(0, narrow ? width - 2 * inset : Math.Min(width * .92, width - 2 * inset))),
            Math.Min(available, Math.Max(0, height <= 700 ? height - 56 : Math.Min(commandsOnly ? 480 : 660, height - 96))), top);
    }
}
