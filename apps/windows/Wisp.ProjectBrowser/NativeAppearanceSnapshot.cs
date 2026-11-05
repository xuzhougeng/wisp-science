using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public static class NativeAppearanceSnapshot
{
    internal static readonly IReadOnlySet<string> Fields = new HashSet<string>(StringComparer.Ordinal)
    {
        "theme", "light_palette", "dark_palette", "ui_font_size", "code_font_size", "ui_font_family", "code_font_family",
        "custom_css", "send_with_modifier", "selection_popup_enabled"
    };

    // The host returns defaults even for a database with no appearance row. Only
    // in that case does WebView keep the preferences it restored before connecting.
    public static JsonObject Resolve(JsonObject host, Func<JsonObject>? local)
    {
        var result = (JsonObject)host.DeepClone();
        if (host["saved"] is JsonValue marker && marker.TryGetValue<bool>(out var saved) && !saved && local != null)
            foreach (var (key, value) in local())
                if (Fields.Contains(key) && value != null) result[key] = value.DeepClone();
        result.Remove("saved"); // read metadata is not part of set_appearance_prefs
        return result;
    }

    public static JsonObject ForCache(JsonObject prefs)
    {
        var copy = new JsonObject();
        foreach (var (key, value) in prefs) if (Fields.Contains(key) && value != null) copy[key] = value.DeepClone();
        return copy;
    }
}
