using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeProjectCounts(int Skills, int Mcp, int Memory)
{
    internal static int Count(JsonNode? node, string key) =>
        node?[key] is JsonValue v && v.TryGetValue<int>(out var n) && n >= 0 ? n
        : throw new InvalidDataException($"Missing or invalid {key}.");

    public static NativeProjectCounts Read(JsonNode? info, string project)
    {
        if (info?["id"]?.GetValue<string>() != project) throw new InvalidDataException("Unexpected project.");
        return new(Count(info, "skill_count"), Count(info, "mcp_server_count"), Count(info, "memory_file_count"));
    }
}

// The sidebar counts the full project inventory. Tiles below count enabled/configured sources.
public sealed class NativeProjectInventory
{
    private (string Database, string? Project) scope;
    private bool requested;
    private long revision;
    public NativeProjectCounts? Counts { get; private set; }
    public void Select(string database, string? project, bool refresh = false)
    {
        if (!refresh && scope == (database, project)) return;
        scope = (database, project); revision++; requested = false; Counts = null;
    }
    public async Task<bool> ReadAsync(INativeSettingsClient client)
    {
        if (requested || scope.Project is not { } project) return false;
        requested = true; var ticket = revision;
        try
        {
            var result = NativeProjectCounts.Read(await client.InvokeAsync("get_project_info", new(), project), project);
            if (ticket != revision) return false;
            Counts = result; return true;
        }
        catch
        {
            // An explicit reconnect may retry, but a late failure cannot invalidate the new scope.
            if (ticket == revision) requested = false;
            return false;
        }
    }
}

public sealed record NativeCapabilityTile(string Label, string Section, int Count)
{
    public static NativeCapabilityTile[] Read(JsonNode? caps, string project)
    {
        if (caps?["project"]?["id"]?.GetValue<string>() != project || caps["memory_files"] is not JsonArray files)
            throw new InvalidDataException("Missing project capability inventory.");
        return [
            new("caps.bundled_skills", "skills", NativeProjectCounts.Count(caps["skill_counts"], "bundled")),
            new("caps.project_skills", "skills", NativeProjectCounts.Count(caps["skill_counts"], "project")),
            new("caps.bundled_mcp_servers", "connections", NativeProjectCounts.Count(caps["mcp_counts"], "bundled")),
            new("caps.project_mcp_servers", "connections", NativeProjectCounts.Count(caps["mcp_counts"], "project")),
            new("caps.memory_files", "memory", files.Count)
        ];
    }
}
