using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeJourneyContractTests
{
    public static async Task Run(string fixturePath)
    {
        var fixture = JsonNode.Parse(File.ReadAllText(fixturePath))!.AsObject();
        if (fixture["schema"]?.GetValue<string>() != "wisp.native-journey.v1"
            || fixture["command"]?.GetValue<string>() != "native_research_journey"
            || fixture["project_id"]?.GetValue<string>() != "research-1"
            || fixture["args"]?["from"]?.GetValue<long>() != 0)
            throw new InvalidOperationException("Native journey envelope drift");
        var fake = new FakeJourneyTransport { Reply = fixture["result"]!.DeepClone() };
        var client = new NativeJourneyClient(fake);
        var page = await client.ReadAsync("research-1", 0, 86400);
        if (page.Entries.Count != 1 || page.Entries[0].Title != "a finding" || page.Truncated
            || fake.Calls != 1 || fake.ProjectId != "research-1" || fake.Command != "native_research_journey"
            || fake.Args?["until"]?.GetValue<long>() != 86400)
            throw new InvalidOperationException("Journey read did not use the shared fixture");
        fake.Fail = true;
        try { await client.ReadAsync("research-1", 0, 86400); throw new InvalidOperationException("Expected lost journey read"); }
        catch (IOException) { }
        if (fake.Calls != 2) throw new InvalidOperationException("Journey read was retried");
        Console.WriteLine("Native journey fixture, explicit project id and no-retry tests passed.");
    }

    sealed class FakeJourneyTransport : INativeSettingsClient
    {
        public int Calls;
        public bool Fail;
        public string? Command;
        public string? ProjectId;
        public JsonObject? Args;
        public JsonNode? Reply;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls++;
            Command = command;
            ProjectId = projectId;
            Args = arguments;
            if (Fail) throw new IOException("lost response");
            return Task.FromResult(Reply?.DeepClone());
        }
    }
}
