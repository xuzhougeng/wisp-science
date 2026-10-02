using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// <summary>Search the shared navigation labels and aliases without navigating
/// or changing the current settings draft.</summary>
public static class NativeSettingsSearch
{
    public static bool Matches(string id, JsonObject entry, string query)
    {
        var terms = query.Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        var searchable = string.Join(" ", new[] { id }.Concat(new[] { "zh", "en", "group", "group_en", "aliases" }
            .Select(key => entry[key]?.GetValue<string>() ?? "")));
        return terms.All(term => searchable.Contains(term, StringComparison.OrdinalIgnoreCase));
    }
}
