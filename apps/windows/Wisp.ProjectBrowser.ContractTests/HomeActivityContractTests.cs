using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class HomeActivityContractTests
{
    public static async Task RunAsync(string path)
    {
        var fixture = JsonNode.Parse(File.ReadAllText(path))!;
        var host = new Host(fixture);
        var snapshot = await NativeHomeActivityClient.ReadAsync(host, null, default);
        if (snapshot is not { Projects: [{ Running: 3, NeedsYou: 1 }], Recent.Length: 2 }
            || snapshot.Recent[0].ProjectId != "study" || snapshot.Recent[0].Status != "needs_you"
            || snapshot.Privacy.ProjectIds.Single() != "hidden")
            throw new InvalidOperationException("Home snapshot disagrees with the shared host fixture.");
        if (await NativeHomeActivityClient.ReadAsync(host, snapshot.Revision, default) != null)
            throw new InvalidOperationException("An unchanged poll must preserve the mounted cards.");
        host.FullResult = null;
        try { await NativeHomeActivityClient.ReadAsync(host, null, default); }
        catch (InvalidDataException) { return; }
        throw new InvalidOperationException("An empty first response cannot claim live activity.");
    }
    private sealed class Host(JsonNode fixture) : INativeSettingsClient
    {
        public JsonNode? FullResult = fixture["result"];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (projectId != null || command != fixture["command"]!.ToString())
                throw new InvalidOperationException("Home activity must not open a project context.");
            if (!args.ContainsKey("known_revision")) return Task.FromResult(FullResult);
            if (args["known_revision"]!.ToString() != fixture["result"]!["revision"]!.ToString())
                throw new InvalidOperationException("Changed the host's revision token.");
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
