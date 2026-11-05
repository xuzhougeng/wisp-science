using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// <summary>Search the shared navigation labels and aliases without navigating
/// or changing the current settings draft.</summary>
public static class NativeSettingsSearch
{
    public static bool Matches(JsonObject entry, string query, string locale)
    {
        var terms = query.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        if (terms.Length == 0) return true;
        // WebView searches both section labels and aliases, but only the displayed group.
        var group = locale == "zh" ? "group" : "group_en";
        var searchable = string.Join(" ", new[] { group, "en", "zh", "aliases" }.Select(key => entry[key]?.GetValue<string>() ?? ""));
        return terms.All(term => searchable.Contains(term, StringComparison.OrdinalIgnoreCase));
    }
}
