using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProjectInventoryTests
{
    private sealed class Host : INativeSettingsClient
    {
        public List<(string Project, TaskCompletionSource<JsonNode?> Reply)> Requests { get; } = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command != "get_project_info" || args.Count != 0 || projectId == null) throw new InvalidOperationException("Unscoped inventory request");
            TaskCompletionSource<JsonNode?> reply = new(); Requests.Add((projectId, reply)); return reply.Task;
        }
    }
    public static async Task RunAsync()
    {
        static void Check(bool ok) { if (!ok) throw new InvalidOperationException("Project capability scope/count drift"); }
        static JsonObject Info(string id, int skills) => new() { ["id"] = id, ["skill_count"] = skills, ["mcp_server_count"] = 0, ["memory_file_count"] = 0 };
        var host = new Host(); var inventory = new NativeProjectInventory();
        inventory.Select("one", "p"); var old = inventory.ReadAsync(host);
        inventory.Select("one", "p"); Check(!await inventory.ReadAsync(host) && host.Requests.Count == 1);
        inventory.Select("two", "p"); var current = inventory.ReadAsync(host);
        host.Requests[1].Reply.SetResult(Info("p", 0)); Check(await current && inventory.Counts == new NativeProjectCounts(0, 0, 0));
        host.Requests[0].Reply.SetResult(Info("p", 8)); Check(!await old && inventory.Counts?.Skills == 0);
        foreach (var malformed in new[] { Info("other", 3), Info("p", -1), new JsonObject { ["id"] = "p" } })
        {
            inventory.Select("two", "p", refresh: true); var load = inventory.ReadAsync(host);
            host.Requests[^1].Reply.SetResult(malformed); Check(!await load && inventory.Counts == null);
        }
        var retry = inventory.ReadAsync(host); host.Requests[^1].Reply.SetResult(Info("p", 7));
        Check(await retry && inventory.Counts?.Skills == 7);
        inventory.Select("two", null); Check(!await inventory.ReadAsync(host));
        var caps = JsonNode.Parse("""{"project":{"id":"p","skill_count":99},"skill_counts":{"bundled":4,"project":2},"mcp_counts":{"bundled":3,"project":1},"memory_files":[{},{}]}""");
        var tiles = NativeCapabilityTile.Read(caps, "p");
        Check(tiles.Select(t => t.Count).SequenceEqual(new[] { 4, 2, 3, 1, 2 }));
        Check(tiles.Select(t => t.Section).SequenceEqual(new[] { "skills", "skills", "connections", "connections", "memory" }));
        try { NativeCapabilityTile.Read(caps, "other"); throw new Exception("Accepted cross-project capability data"); } catch (InvalidDataException) { }
    }
}
