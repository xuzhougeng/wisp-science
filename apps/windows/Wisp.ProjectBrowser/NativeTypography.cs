using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// Shared native font preferences; sizes are relative to the existing 14/12 point design.
public sealed record NativeTypography(double UiSize = 14, double CodeSize = 12,
    string UiFamily = "Segoe UI", string CodeFamily = "Cascadia Mono, Consolas")
{
    public static NativeTypography From(JsonObject preferences) => new(
        Size(preferences, "ui_font_size", 14, 12, 20),
        Size(preferences, "code_font_size", 12, 10, 20),
        Family(preferences, "ui_font_family", "Segoe UI"),
        Family(preferences, "code_font_family", "Cascadia Mono, Consolas"));

    public double Scale(double size, bool code = false) => size * (code ? CodeSize / 12 : UiSize / 14);
    private static double Size(JsonObject prefs, string key, double fallback, double min, double max)
        => prefs[key] is JsonValue value && double.TryParse(value.ToJsonString(),
            System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out var size)
            && double.IsFinite(size) && size > 0 ? Math.Clamp(size, min, max) : fallback;
    private static string Family(JsonObject prefs, string key, string fallback)
        => prefs[key] is JsonValue value && value.TryGetValue<string>(out var family)
            && !string.IsNullOrWhiteSpace(family) ? family.Trim() : fallback;
}
