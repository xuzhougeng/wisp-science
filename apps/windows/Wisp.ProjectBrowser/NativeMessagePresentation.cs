using System.Globalization;

namespace Wisp.ProjectBrowser;

public sealed record NativeMessageReference(string Kind, string Label);
public sealed record NativeUserMessage(string Body, string ShortBody, bool IsLong, string[] Attachments, NativeMessageReference[] References);
public sealed record NativeMessageTime(string Compact, string Full);

public static class NativeMessagePresentation
{
    public static double ImageWidth(double windowWidth) => Math.Clamp(double.IsFinite(windowWidth) ? windowWidth * .22 : 148, 148, 220);
    private static readonly (string Prefix, string Kind)[] Markers = [
        ("Uploaded files: ", "file"), ("Attached artifacts: ", "artifact"), ("Attached sessions: ", "session"),
        ("Project context: ", "project"), ("Selected skills: ", "skill"), ("Selected workflows: ", "workflow"),
        ("Target environments: ", "context"), ("Target runtimes: ", "runtime")];
    public static NativeUserMessage User(string text, IEnumerable<string>? attachments = null)
    {
        var body = new List<string>(); var files = new List<string>(); var references = new List<NativeMessageReference>();
        foreach (var block in text.Replace("\r\n", "\n").Split("\n\n", StringSplitOptions.None))
        {
            if (block.StartsWith("AI source-edit instruction: ", StringComparison.Ordinal) || block.StartsWith("Feedback context: ", StringComparison.Ordinal)) continue;
            var marker = Markers.FirstOrDefault(item => block.StartsWith(item.Prefix, StringComparison.Ordinal));
            if (marker.Prefix == null) { body.Add(block); continue; }
            foreach (var value in block[marker.Prefix.Length..].Split(", ", StringSplitOptions.RemoveEmptyEntries).Select(s => s.Trim()).Where(s => s.Length > 0))
                if (marker.Kind == "file") files.Add(value); else references.Add(new(marker.Kind, value));
        }
        files.AddRange(attachments ?? []);
        var valueBody = string.Join("\n\n", body).Trim();
        var lines = valueBody.Split('\n'); var isLong = lines.Length > 12 || valueBody.EnumerateRunes().Count() > 600;
        var shortBody = isLong ? string.Concat(string.Join("\n", lines.Take(12)).EnumerateRunes().Take(600).Select(r => r.ToString())).TrimEnd() + "…" : valueBody;
        return new(valueBody, shortBody, isLong, files.Distinct(StringComparer.Ordinal).ToArray(), references.ToArray());
    }
    public static NativeMessageTime? Time(long? timestamp, string? locale, DateTimeOffset now, TimeZoneInfo? zone = null)
    {
        if (timestamp is not > 0) return null;
        try
        {
            var value = timestamp > 1_000_000_000_000 ? DateTimeOffset.FromUnixTimeMilliseconds(timestamp.Value) : DateTimeOffset.FromUnixTimeSeconds(timestamp.Value);
            var local = TimeZoneInfo.ConvertTime(value, zone ?? TimeZoneInfo.Local); var today = TimeZoneInfo.ConvertTime(now, zone ?? TimeZoneInfo.Local);
            var format = local.Date == today.Date ? "HH:mm" : local.Year == today.Year ? "MM-dd HH:mm" : "yyyy-MM-dd HH:mm";
            return new(local.ToString(format, CultureInfo.InvariantCulture), local.ToString("G", CultureInfo.GetCultureInfo(NativeLocalization.Normalize(locale) == "zh" ? "zh-CN" : "en-US")));
        }
        catch (ArgumentOutOfRangeException) { return null; }
    }
}
