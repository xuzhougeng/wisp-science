using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeScratchContractTests
{
    public static async Task Run(string openFixturePath)
    {
        var open = JsonNode.Parse(File.ReadAllText(openFixturePath))!.AsObject();
        var scope = open["project_id"];
        if (open["schema"]?.GetValue<string>() != "wisp.native-scratch.v1"
            || open["command"]?.GetValue<string>() != "native_scratch_open"
            || (scope is not null && scope.GetValueKind() != JsonValueKind.Null))
            throw new InvalidOperationException("Native scratch open envelope drift");
        var fake = new FakeScratchTransport { Reply = open["result"]!.DeepClone() };
        var client = new NativeScratchClient(fake);
        var session = await client.OpenAsync();
        if (!session.ProjectId.StartsWith("scratch:") || session.SessionId != "session-scratch"
            || fake.Calls != 1 || fake.ProjectId is not null || fake.Command != "native_scratch_open")
            throw new InvalidOperationException("Scratch open did not use the shared fixture");
        fake.Fail = true;
        try { await client.OpenAsync(); throw new InvalidOperationException("Expected lost scratch open"); }
        catch (IOException) { }
        if (fake.Calls != 2) throw new InvalidOperationException("Scratch open was retried");

        var close = JsonNode.Parse(File.ReadAllText(Path.Combine(Path.GetDirectoryName(openFixturePath)!, "close.json")))!.AsObject();
        if (close["command"]?.GetValue<string>() != "native_scratch_close"
            || close["project_id"]?.GetValue<string>() != session.ProjectId
            || close["result"]?.GetValue<bool>() != true)
            throw new InvalidOperationException("Native scratch close envelope drift");
        fake.Fail = false;
        fake.Reply = close["result"]!.DeepClone();
        var removed = await client.CloseAsync(session.ProjectId);
        if (!removed || fake.ProjectId != session.ProjectId || fake.Command != "native_scratch_close")
            throw new InvalidOperationException("Scratch close did not use the shared fixture");
        var before = fake.Calls;
        fake.Fail = true;
        try { await client.CloseAsync(session.ProjectId); throw new InvalidOperationException("Expected lost scratch close"); }
        catch (IOException) { }
        if (fake.Calls != before + 1) throw new InvalidOperationException("Scratch close was retried");
        Console.WriteLine("Native scratch open and close fixtures, empty open scope and no-retry tests passed.");
    }

    sealed class FakeScratchTransport : INativeSettingsClient
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
