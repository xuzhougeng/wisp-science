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
        if (page.Entries[0].Kind != "finding" || page.Entries[0].Summary != "Evidence" || !page.Entries[0].Manual || page.Entries[0].SourceId != "entry-1")
            throw new InvalidOperationException("Journey detail fields were discarded");
        fake.Fail = false;
        fake.Reply = JsonNode.Parse("""
            {"version_id":"v1","filename":"result.pdf","version_number":2,"source":{"run_id":"run-1","run_title":"Analysis","run_status":"succeeded","context_id":"local","generated_at":100,"inputs":[{"title":"Data","role":"input","version_id":"v0","confidence":"exact"}]},"text":null,"mime":"application/pdf","base64":"JVBERg==","truncated":false,"content_error":null}
            """);
        var artifact = await client.ArtifactAsync("research-1", "v1");
        if (fake.Command != "native_research_journey_artifact" || fake.ProjectId != "research-1" || fake.Args?["version_id"]?.GetValue<string>() != "v1"
            || artifact.VersionNumber != 2 || artifact.Source.Inputs.Single().VersionId != "v0" || artifact.Source.RunId != "run-1")
            throw new InvalidOperationException("Journey artifact source contract drift");
        try { await client.ArtifactAsync("research-1", "wrong-version"); throw new Exception("expected identity rejection"); }
        catch (InvalidDataException) { }
        fake.Reply = JsonNode.Parse("""{"id":"r","context_id":"local","title":"Run","kind":"shell","status":"succeeded","created_at":1,"progress_json":"{}"}""");
        var run = await client.RunAsync("research-1", "r");
        if (run.Id != "r" || fake.Command != "native_research_journey_run" || fake.ProjectId != "research-1" || fake.Args?["run_id"]?.GetValue<string>() != "r")
            throw new InvalidOperationException("Journey run must use exact project and run identity");
        try { await client.RunAsync("research-1", "other"); throw new Exception("expected run identity rejection"); }
        catch (InvalidDataException) { }
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
