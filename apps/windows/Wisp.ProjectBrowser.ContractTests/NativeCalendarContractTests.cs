using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeCalendarContractTests
{
    public static async Task Run(string fixturePath)
    {
        var fixture = JsonNode.Parse(File.ReadAllText(fixturePath))!.AsObject();
        var scope = fixture["project_id"];
        if (fixture["schema"]?.GetValue<string>() != "wisp.native-calendar.v1"
            || fixture["command"]?.GetValue<string>() != "native_research_calendar"
            || (scope is not null && scope.GetValueKind() != JsonValueKind.Null)
            || fixture["args"]?["project_ids"]?[0]?.GetValue<string>() != "research-1")
            throw new InvalidOperationException("Native calendar envelope drift");
        var fake = new FakeCalendarTransport { Reply = fixture["result"]!.DeepClone() };
        var client = new NativeCalendarClient(fake);
        var rows = await client.ReadAsync(new[] { "research-1" }, 0, 86400);
        if (rows.Count != 1 || rows[0].ProjectId != "research-1" || rows[0].History.Entries[0].Title != "a finding"
            || rows[0].Error is not null || fake.Calls != 1 || fake.ProjectId is not null
            || fake.Command != "native_research_calendar"
            || fake.Args?["project_ids"]?[0]?.GetValue<string>() != "research-1"
            || fake.Args?["from"]?.GetValue<long>() != 0)
            throw new InvalidOperationException("Calendar read did not use the shared fixture");
        fake.Fail = true;
        try { await client.ReadAsync(new[] { "research-1" }, 0, 86400); throw new InvalidOperationException("Expected lost calendar read"); }
        catch (IOException) { }
        if (fake.Calls != 2) throw new InvalidOperationException("Calendar read was retried");
        Console.WriteLine("Native calendar fixture, explicit empty scope and no-retry tests passed.");
    }

    sealed class FakeCalendarTransport : INativeSettingsClient
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
