using System.Globalization;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public enum NativeUsageActivityMode { Daily, Weekly, Cumulative }
public sealed record NativeUsageTotals(long Input, long Output, long Reasoning, long Cached)
{
    public long[] Values => [Input, Output, Reasoning, Cached];
    public static NativeUsageTotals From(JsonNode row) => new(NativeUsagePresentation.Number(row, "input"), NativeUsagePresentation.Number(row, "output"), NativeUsagePresentation.Number(row, "reasoning"), NativeUsagePresentation.Number(row, "cached"));
}
public sealed record NativeUsageWorkspace(string Id, string Name, string Directory, long UpdatedAt, long Sessions, NativeUsageTotals Totals)
{ public string DisplayName => string.IsNullOrWhiteSpace(Name) ? Id : Name; }
public sealed record NativeUsageSession(string Id, string Title, long UpdatedAt, NativeUsageTotals Totals);
public sealed record NativeUsageDay(string Date, long Tokens, bool Future);
public sealed record NativeUsageActivityCell(string Period, long Tokens, int Level, bool Future);
public sealed record NativeUsageModelSlice(string Label, long Tokens, string Color);
public sealed record NativeUsageToolRow(string Kind, string Name, long Calls, string Color);

/// Presentation calculations ported from settings_view.rs; no persistence or project activation.
public static class NativeUsagePresentation
{
    public static readonly string[] Colors = ["#7c3aed", "#0ea5e9", "#10b981", "#f59e0b", "#ef4444", "#ec4899", "#6366f1", "#64748b"];
    public static readonly string[] TotalKeys = ["input", "output", "reasoning", "cached"];
    public static string Text(JsonNode row, string key) => row[key]?.GetValue<string>() ?? "";
    public static long Number(JsonNode row, string key) => row[key] is JsonValue value
        ? value.TryGetValue<long>(out var number) ? number : value.TryGetValue<int>(out var small) ? small : 0 : 0;
    public static IEnumerable<JsonObject> Rows(JsonNode? rows) => (rows as JsonArray)?.OfType<JsonObject>() ?? [];
    public static string Tokens(long value) => value < 1000 ? Math.Max(0, value).ToString(CultureInfo.InvariantCulture)
        : (value / 1000d).ToString("0.0", CultureInfo.InvariantCulture) + "k";
    public static long Sum(IEnumerable<long> values) => values.Aggregate(0L, (sum, value) => value <= 0 ? sum : sum > long.MaxValue - value ? long.MaxValue : sum + value);
    public static NativeUsageWorkspace[] Workspaces(JsonNode overview) => Rows(overview["workspaces"]).Select(row => new NativeUsageWorkspace(
        Text(row, "project_id"), Text(row, "name"), Text(row, "workspace_dir"), Number(row, "updated_at"), Math.Max(0, Number(row, "session_count")), NativeUsageTotals.From(row))).ToArray();
    public static NativeUsageSession[] Sessions(JsonNode page) => Rows(page["items"]).Select(row => new NativeUsageSession(Text(row, "id"), Text(row, "title"), Number(row, "updated_at"), NativeUsageTotals.From(row))).ToArray();
    public static NativeUsageTotals Totals(IEnumerable<NativeUsageWorkspace> rows)
    { var values = rows.ToArray(); return new(Sum(values.Select(row => row.Totals.Input)), Sum(values.Select(row => row.Totals.Output)), Sum(values.Select(row => row.Totals.Reasoning)), Sum(values.Select(row => row.Totals.Cached))); }
    public static NativeUsageDay[] Days(JsonNode overview) => Rows(overview["days"]).Select(row => new NativeUsageDay(Text(row, "date"), Number(row, "tokens"), row["future"]?.GetValue<bool>() == true)).ToArray();
    private static int Level(long value, long max) => value <= 0 || max <= 0 ? 0 : Math.Clamp((int)Math.Ceiling(value / (double)max * 4), 1, 4);
    public static NativeUsageActivityCell[] Activity(IReadOnlyList<NativeUsageDay> days, NativeUsageActivityMode mode)
    {
        if (mode == NativeUsageActivityMode.Daily)
        {
            var max = days.Where(day => !day.Future).Select(day => day.Tokens).DefaultIfEmpty(0).Max();
            return days.Select(day => new NativeUsageActivityCell(day.Date, day.Tokens, day.Future ? 0 : Level(day.Tokens, max), day.Future)).ToArray();
        }
        var weeks = days.Chunk(7).ToArray(); var amounts = weeks.Select(week => Sum(week.Where(day => !day.Future).Select(day => day.Tokens))).ToArray();
        if (mode == NativeUsageActivityMode.Cumulative)
        { var running = 0L; for (var i = 0; i < amounts.Length; i++) amounts[i] = running = Sum([running, amounts[i]]); }
        var maximum = amounts.DefaultIfEmpty(0).Max(); var cells = new List<NativeUsageActivityCell>();
        for (var i = 0; i < weeks.Length; i++)
        {
            var week = weeks[i]; var tokens = amounts[i]; var fill = tokens <= 0 || maximum <= 0 ? 0 : Math.Clamp((int)Math.Ceiling(tokens / (double)maximum * week.Length), 1, week.Length);
            var end = week.LastOrDefault(day => !day.Future)?.Date ?? week[^1].Date;
            var period = mode == NativeUsageActivityMode.Cumulative ? end : week[0].Date + " – " + end;
            for (var row = 0; row < week.Length; row++) cells.Add(new(period, tokens, row + fill >= week.Length ? Math.Max(1, Level(tokens, maximum)) : 0, false));
        }
        return cells.ToArray();
    }
    public static (int Week, int Month)[] Months(IEnumerable<NativeUsageDay> days)
    {
        var labels = new List<(int, int)>(); var previous = 0; var index = 0;
        foreach (var week in days.Chunk(7))
        {
            var date = week[0].Date;
            if (date.Length >= 7 && int.TryParse(date.AsSpan(5, 2), out var month) && month is >= 1 and <= 12 && month != previous)
            { labels.Add((index, month)); previous = month; }
            index++;
        }
        return labels.ToArray();
    }
    public static NativeUsageModelSlice[] Models(JsonNode overview, JsonNode? profiles, string unknown, string other)
    {
        var merged = new Dictionary<string, List<long>>(StringComparer.Ordinal); var catalog = Rows(profiles).ToArray();
        foreach (var row in Rows(overview["models"]).Where(row => Number(row, "tokens") > 0))
        {
            var id = Text(row, "model"); var profile = catalog.FirstOrDefault(profile => Text(profile, "id") == id);
            var label = profile == null ? id == "unknown" ? unknown : id : string.IsNullOrWhiteSpace(Text(profile, "model")) ? Text(profile, "label") : Text(profile, "model");
            if (!merged.TryGetValue(label, out var values)) merged[label] = values = []; values.Add(Number(row, "tokens"));
        }
        var totals = merged.Select(pair => (Label: pair.Key, Tokens: Sum(pair.Value))).OrderByDescending(row => row.Tokens).ThenBy(row => row.Label, StringComparer.Ordinal).ToList();
        if (totals.Count > 8) { var tail = Sum(totals.Skip(7).Select(row => row.Tokens)); totals.RemoveRange(7, totals.Count - 7); totals.Add((other, tail)); }
        return totals.Select((row, index) => new NativeUsageModelSlice(row.Label, row.Tokens, Colors[index])).ToArray();
    }
    public static NativeUsageToolRow[] Tools(JsonNode overview, string other)
    {
        var rows = Rows(overview["tools"]).Where(row => Number(row, "calls") > 0 && Text(row, "kind") is "skill" or "mcp")
            .Select(row => (Kind: Text(row, "kind"), Name: Text(row, "name"), Calls: Number(row, "calls")))
            .OrderByDescending(row => row.Calls).ThenBy(row => row.Kind, StringComparer.Ordinal).ThenBy(row => row.Name, StringComparer.Ordinal).ToList();
        if (rows.Count > 10) { var tail = Sum(rows.Skip(9).Select(row => row.Calls)); rows.RemoveRange(9, rows.Count - 9); rows.Add(("other", other, tail)); }
        return rows.Select((row, index) => new NativeUsageToolRow(row.Kind, row.Name, row.Calls, Colors[index % Colors.Length])).ToArray();
    }
}
