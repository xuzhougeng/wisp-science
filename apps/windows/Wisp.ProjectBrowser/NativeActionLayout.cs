namespace Wisp.ProjectBrowser;

/// <summary>Preserves action order while fitting each row to the available width.</summary>
public static class NativeActionLayout
{
    public readonly record struct Size(double Width, double Height);
    public readonly record struct Slot(double X, double Y, double Width, double Height);
    public static Slot[] Arrange(IReadOnlyList<Size> items, double width, double gap = 8)
    {
        width = Math.Max(0, width);
        var slots = new Slot[items.Count];
        double x = 0, y = 0, rowHeight = 0;
        for (var i = 0; i < items.Count; i++)
        {
            var itemWidth = Math.Min(width, Math.Max(0, items[i].Width));
            var height = Math.Max(0, items[i].Height);
            if (x > 0 && x + itemWidth > width) { y += rowHeight + gap; x = 0; rowHeight = 0; }
            slots[i] = new(x, y, itemWidth, height);
            x += itemWidth + gap; rowHeight = Math.Max(rowHeight, height);
        }
        return slots;
    }
}
