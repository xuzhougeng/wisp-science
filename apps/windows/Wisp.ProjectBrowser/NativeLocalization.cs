using System.Text.Json.Nodes;
using System.Text.RegularExpressions;

namespace Wisp.ProjectBrowser;

/// Keyed UI strings shared with WebView. User content never enters translation lookup.
public sealed class NativeLocalization(IReadOnlyDictionary<string, Dictionary<string, string>> catalog, string? locale = null)
{
    public string Locale { get; private set; } = Normalize(locale);
    public static string Normalize(string? value)
    {
        var code = value?.Trim() ?? "";
        return code.Equals("zh", StringComparison.OrdinalIgnoreCase)
            || code.StartsWith("zh-", StringComparison.OrdinalIgnoreCase)
            || code.StartsWith("zh_", StringComparison.OrdinalIgnoreCase) ? "zh" : "en";
    }
    public bool ApplyCommittedSettings(JsonNode? settings)
    {
        if (settings is not JsonObject obj || obj["locale"] is not JsonValue value
            || !value.TryGetValue<string>(out var code)) return false;
        var next = Normalize(code);
        if (next == Locale) return false;
        Locale = next; return true;
    }
    public string Choose(string chinese, string english) => Locale == "zh" ? chinese : english;
    public bool Preview(string? locale)
    {
        var next = Normalize(locale);
        if (next == Locale) return false;
        Locale = next; return true;
    }
    public string Text(string key) => catalog.TryGetValue(Locale, out var selected) && selected.TryGetValue(key, out var text)
        ? text : catalog.TryGetValue("en", out var english) && english.TryGetValue(key, out text) ? text : key;
    public string Format(string key, params (string Name, string Value)[] values)
    {
        var replacements = values.ToDictionary(item => item.Name, item => item.Value, StringComparer.Ordinal);
        // One pass: inserted user values are neither translated nor interpreted as placeholders.
        return Regex.Replace(Text(key), @"\{([^{}]+)\}", match => replacements.GetValueOrDefault(match.Groups[1].Value, match.Value));
    }
}
