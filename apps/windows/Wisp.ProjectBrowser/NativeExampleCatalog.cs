using System.Reflection;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeExampleInfo(string Id, string Title);

/// <summary>Read-only recordings bundled from the same seed manifests as WebView.
/// Browsing never opens a database or extracts files into a user's project.</summary>
public static class NativeExampleCatalog
{
    private static readonly Assembly Assembly = typeof(NativeExampleCatalog).Assembly;
    private static readonly Lazy<NativeExampleInfo[]> entries = new(() => Assembly.GetManifestResourceNames()
        .Where(name => name.StartsWith("Example.manifest_", StringComparison.Ordinal) && name.EndsWith(".json", StringComparison.Ordinal))
        .Order(StringComparer.Ordinal).Select(name =>
        {
            var id = name[8..^5]; var root = Read(id);
            var request = root["input_data"]?["request"]?.GetValue<string>() ?? "";
            var first = request.Split('.')[0].Trim();
            return new NativeExampleInfo(id, string.Concat(first.EnumerateRunes().Take(70).Select(r => r.ToString())));
        }).ToArray());
    public static IReadOnlyList<NativeExampleInfo> Entries => entries.Value;
    private static JsonNode Read(string id)
    {
        using var stream = Assembly.GetManifestResourceStream("Example." + id + ".json")
            ?? throw new InvalidDataException("Example recording is not bundled");
        return JsonNode.Parse(stream)?["root_frame"] ?? throw new InvalidDataException("Missing example recording");
    }
    public static ConversationSnapshot Load(string id)
    {
        if (!Entries.Any(entry => entry.Id == id)) throw new ArgumentException("Unknown example recording", nameof(id));
        return Decode(id, Read(id));
    }
    public static ConversationSnapshot Decode(string id, JsonNode root)
    {
        var output = root["output_data"]; var rows = new List<ConversationItem>();
        if (output?["context_seed"] is JsonArray seeds)
            foreach (var seed in seeds)
            {
                var text = string.Concat(Enumerable.Repeat(seed?["text"]?.GetValue<string>() ?? "", Math.Clamp(seed?["pad"]?.GetValue<int>() ?? 1, 1, 64)));
                for (var i = 0; i < Math.Clamp(seed?["repeat"]?.GetValue<int>() ?? 1, 1, 200); i++)
                    rows.Add(new(seed?["role"]?.GetValue<string>() ?? "assistant", Clean(text), null, null, null, null));
            }
        if (output?["items"] is JsonArray items)
            foreach (var node in items)
            {
                var item = node?.Deserialize<ConversationItem>(ConversationSnapshot.JsonOptions)
                    ?? throw new InvalidDataException("Invalid example transcript row");
                rows.Add(item with { Text = Clean(item.Text), Input = item.Input == null ? null : Clean(item.Input), Status = item.Role == "question" ? "answered" : item.Status });
            }
        if (rows.Count == 0)
        {
            rows.Add(new("user", Clean(root["input_data"]?["request"]?.GetValue<string>() ?? ""), null, null, null, null));
            if (output?["thinking"]?.GetValue<string>() is { Length: > 0 } thinking) rows.Add(new("reasoning", Clean(thinking), null, null, null, null));
            rows.Add(new("assistant", Clean(output?["response"]?.GetValue<string>() ?? ""), null, null, null, null));
        }
        return new(ConversationSnapshot.SchemaId, "bundled-example", 1, "example", id, rows.ToArray(), null, false, false, true, "", null, null, []);
    }
    private static string Clean(string text)
        => Regex.Replace(Regex.Replace(text, @"!\[([^\]]*)\]\(\{\{artifact:[^}]+\}\}\)", "[$1 (figure)]"), @"\{\{artifact:[^}]+\}\}", "(artifact)");
}
