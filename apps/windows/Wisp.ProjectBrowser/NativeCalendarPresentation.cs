using System.Text;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public readonly record struct NativeCalendarLayout(bool Desktop, bool HorizontalProjects, double Inset, double NavigationWidth, double DetailsWidth)
{
    public static NativeCalendarLayout ForViewport(double width, double height)
    {
        width = double.IsFinite(width) ? Math.Max(0, width) : 0;
        height = double.IsFinite(height) ? Math.Max(0, height) : 0;
        return new(width >= 1200 && height >= 720, width <= 650, Math.Clamp(width * .025, 16, 80),
            Math.Clamp(width * .12, 180, 240), width >= 1200 ? Math.Clamp(width * .3, 360, 720) : 0);
    }
}

public static class NativeCalendarPresentation
{
    public const int PageSize = 8;
    public static int ColorIndex(string id)
    {
        uint hash = 0;
        foreach (var value in Encoding.UTF8.GetBytes(id)) hash = unchecked(hash * 31 + value);
        return (int)(hash % 6);
    }
    public static DateTime SelectedDay(DateTime month, DateTime today)
        => month.Year == today.Year && month.Month == today.Month ? today.Date : new(month.Year, month.Month, 1);
    // Full Monday-first weeks, including both leading and trailing blank cells.
    public static IReadOnlyList<DateTime?> Cells(DateTime month)
    {
        var start = new DateTime(month.Year, month.Month, 1);
        var offset = ((int)start.DayOfWeek + 6) % 7;
        var count = DateTime.DaysInMonth(month.Year, month.Month);
        return Enumerable.Range(0, (offset + count + 6) / 7 * 7)
            .Select(index => index < offset || index >= offset + count ? (DateTime?)null : start.AddDays(index - offset)).ToArray();
    }
    public static DateTime LocalDay(long timestamp) => DateTimeOffset.FromUnixTimeSeconds(timestamp).LocalDateTime.Date;
    public static (IReadOnlyList<CalendarEntry> Entries, IReadOnlyDictionary<string, int> Outputs) FoldOutputs(IEnumerable<CalendarEntry> input)
    {
        var rows = input.ToArray();
        var runs = rows.Where(e => e.Kind == "run" && e.SourceId.Length > 0).Select(e => e.SourceId).ToHashSet(StringComparer.Ordinal);
        var outputs = new Dictionary<string, int>(StringComparer.Ordinal);
        var visible = new List<CalendarEntry>();
        foreach (var entry in rows)
        {
            if (entry.Kind == "artifact" && entry.RunId is { } run && runs.Contains(run))
                outputs[run] = outputs.GetValueOrDefault(run) + 1;
            else visible.Add(entry);
        }
        return (visible.OrderByDescending(e => e.OccurredAt).ThenByDescending(e => e.Id, StringComparer.Ordinal).ToArray(), outputs);
    }
}
