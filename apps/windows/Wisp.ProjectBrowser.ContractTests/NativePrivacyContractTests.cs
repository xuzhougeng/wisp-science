using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class NativePrivacyContractTests
{
    public static async Task Run(string fixturePath)
    {
        var open = JsonNode.Parse(File.ReadAllText(fixturePath))!.AsObject();
        var scope = open["project_id"];
        if (open["command"]?.GetValue<string>() != "get_privacy_mode"
            || (scope is not null && scope.GetValueKind() != JsonValueKind.Null)
            || open["args"]!.AsObject().Count != 0)
            throw new InvalidOperationException("Privacy mode envelope drift");
        var fake = new FakePrivacyTransport { Reply = open["result"]!.DeepClone() };
        var client = new NativePrivacyClient(fake);
        var mode = await client.GetAsync();
        if (!mode.Active || mode.ProjectIds is not ["hidden", "research-1"] || fake.Calls != 1 || fake.ProjectId is not null || fake.Command != "get_privacy_mode")
            throw new InvalidOperationException("Privacy read did not use the shared fixture");
        fake.Fail = true;
        try { await client.GetAsync(); throw new InvalidOperationException("Expected lost privacy read"); }
        catch (IOException) { }
        if (fake.Calls != 2) throw new InvalidOperationException("Privacy read was retried");
        Console.WriteLine("Native privacy mode fixture, empty scope and no-retry tests passed.");
    }

    sealed class FakePrivacyTransport : INativeSettingsClient
    {
        public int Calls;
        public bool Fail;
        public string? Command;
        public string? ProjectId;
        public JsonNode? Reply;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls++;
            Command = command;
            ProjectId = projectId;
            if (Fail) throw new IOException("lost response");
            return Task.FromResult(Reply?.DeepClone());
        }
    }
}
