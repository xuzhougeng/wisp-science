using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// Shared native font preferences; sizes are relative to the existing 14/12 point design.
public sealed record NativeTypography(double UiSize = 14, double CodeSize = 12,
    string UiFamily = "Inter", string CodeFamily = "JetBrains Mono")
{
    public static NativeTypography From(JsonObject preferences) => new(
        Size(preferences, "ui_font_size", 14, 0, 30),
        Size(preferences, "code_font_size", 12, 0, 30),
        Family(preferences, "ui_font_family", "Inter"),
        Family(preferences, "code_font_family", "JetBrains Mono"));

    // Match base.css on Windows. Resolve our bundled fonts even when they are not
    // installed, then retain explicit CJK fallbacks for the Latin-only UI subset.
    public string FontSource(bool code = false)
    {
        var fallback = code
            ? new[] { "JetBrains Mono", "Cascadia Mono", "Consolas", "Noto Sans Mono CJK SC", "Microsoft YaHei UI" }
            : new[] { "Inter", "Microsoft YaHei UI", "Microsoft YaHei", "Noto Sans SC", "Segoe UI", "Arial" };
        return string.Join(", ", (code ? CodeFamily : UiFamily).Split(',')
            .Select(name => name.Trim().Trim('\"', '\''))
            .Where(name => name.Length > 0)
            .Concat(fallback).Distinct(StringComparer.OrdinalIgnoreCase).Select(name =>
                name.Equals("Inter", StringComparison.OrdinalIgnoreCase)
                    ? "ms-appx:///Assets/Fonts/inter-latin.ttf#Inter"
                    : name.Equals("JetBrains Mono", StringComparison.OrdinalIgnoreCase)
                        ? "ms-appx:///Assets/Fonts/jetbrains-mono-400.ttf#JetBrains Mono" : name));
    }

    public string MarkdownFontSource(string? locale) => NativeLocalization.Normalize(locale) == "zh"
        ? FontSource() : "ms-appx:///Assets/Fonts/wisp-serif.ttf#Wisp Serif, Noto Serif SC, Songti SC, STSong, Georgia, Times New Roman";
    public double MarkdownSize => Math.Max(.01, UiSize + 1);
    public double ResponseSize(string? locale, bool commentary = false) => Math.Max(.01,
        UiSize + (commentary ? 0 : NativeLocalization.Normalize(locale) == "zh" ? 1.5 : 1));
    public double ResponseLeading(string? locale, bool commentary = false) => commentary ? 1.5
        : NativeLocalization.Normalize(locale) == "zh" ? 1.78 : 1.62;
    public double ResponseTableSize => Math.Max(.01, UiSize - 1);

    // XAML requires a positive rendering size; retain the WebView's 0px wire value.
    public double Scale(double size, bool code = false) => Math.Max(.01, size * (code ? CodeSize / 12 : UiSize / 14));
    private static double Size(JsonObject prefs, string key, double fallback, double min, double max)
        => prefs[key] is JsonValue value && double.TryParse(value.ToJsonString(),
            System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out var size)
            && double.IsFinite(size) && size >= 0 ? Math.Clamp(size, min, max) : fallback;
    private static string Family(JsonObject prefs, string key, string fallback)
        => prefs[key] is JsonValue value && value.TryGetValue<string>(out var family)
            && !string.IsNullOrWhiteSpace(family) ? family.Trim() : fallback;
}
