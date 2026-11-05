namespace Wisp.ProjectBrowser;

/// <summary>Preserves action order while fitting each row to the available width.</summary>
public static class NativeActionLayout
{
    public readonly record struct Size(double Width, double Height);
    public readonly record struct Slot(double X, double Y, double Width, double Height);
    // XAML rounds a panel's desired width to the display pixel grid. Reserving
    // one guard DIP prevents that rounding from making a measured row narrower
    // during Arrange, where its final button would otherwise wrap unexpectedly.
    public static double MeasureConstraint(double available) => double.IsFinite(available) ? Math.Max(0, Math.Floor(available) - 1) : available;
    public static double DesiredWidth(IReadOnlyList<Slot> slots, double constraint, bool rightAligned)
        => slots.Count == 0 || constraint == 0 ? 0 : rightAligned && double.IsFinite(constraint) ? constraint + 1 : Math.Ceiling(slots.Max(s => s.X + s.Width)) + 1;
    public static Slot[] Arrange(IReadOnlyList<Size> items, double width, double gap = 8, bool rightAligned = false, double? rowGap = null, bool verticallyCentered = false, bool spaceBetween = false, bool firstItemLeading = false)
    {
        width = Math.Max(0, width);
        var slots = new Slot[items.Count];
        double x = 0, y = 0, rowHeight = 0;
        for (var i = 0; i < items.Count; i++)
        {
            var itemWidth = Math.Min(width, Math.Max(0, items[i].Width));
            var height = Math.Max(0, items[i].Height);
            if (x > 0 && x + itemWidth > width) { y += rowHeight + (rowGap ?? gap); x = 0; rowHeight = 0; }
            slots[i] = new(x, y, itemWidth, height);
            x += itemWidth + gap; rowHeight = Math.Max(rowHeight, height);
        }
        if ((rightAligned || spaceBetween) && double.IsFinite(width) || verticallyCentered)
        {
            for (var start = 0; start < slots.Length;)
            {
                var end = start + 1;
                while (end < slots.Length && slots[end].Y == slots[start].Y) end++;
                var offset = rightAligned && double.IsFinite(width) ? Math.Max(0, width - slots[end - 1].X - slots[end - 1].Width) : 0;
                var height = slots.Skip(start).Take(end - start).Max(slot => slot.Height);
                var extraGap = spaceBetween && double.IsFinite(width) && end - start > 1
                    ? Math.Max(0, width - slots[end - 1].X - slots[end - 1].Width) / (end - start - 1) : 0;
                for (var i = start; i < end; i++) slots[i] = slots[i] with
                { X = slots[i].X + offset + extraGap * (i - start), Y = slots[i].Y + (verticallyCentered ? (height - slots[i].Height) / 2 : 0) };
                start = end;
            }
        }
        if (firstItemLeading && slots.Length > 0) slots[0] = slots[0] with { X = 0 };
        return slots;
    }
}
