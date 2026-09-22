using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class NativePublicationContractTests
{
    public static async Task Run(string fixturePath)
    {
        var fixture = JsonNode.Parse(File.ReadAllText(fixturePath))!.AsObject();
        if (fixture["schema"]?.GetValue<string>() != "wisp.native-publication.v1"
            || fixture["command"]?.GetValue<string>() != "native_publication_workspace"
            || fixture["project_id"]?.GetValue<string>() != "research-1")
            throw new InvalidOperationException("Native publication envelope drift");
        var fake = new FakePublicationTransport { Reply = fixture["result"]!.DeepClone() };
        var client = new NativePublicationClient(fake);
        var page = await client.ReadAsync("research-1");
        if (page.Publications.Count != 1 || page.Publication?.Title != "RNA-seq paper"
            || page.Publication.ProjectId != "research-1" || page.Revision?.Label != "v1"
            || page.Items[0].Kind != "claim" || fake.Calls != 1 || fake.ProjectId != "research-1"
            || fake.Command != "native_publication_workspace")
            throw new InvalidOperationException("Publication read did not use the shared fixture");
        fake.Reply = fixture["result"]!.DeepClone();
        var created = await client.CreateAsync("research-1", "RNA-seq paper", "Differential expression", "v1");
        if (created.Publication?.Id != "pub-1" || fake.ProjectId != "research-1"
            || fake.Command != "native_publication_create" || fake.Args?["revision_label"]?.GetValue<string>() != "v1")
            throw new InvalidOperationException("Publication create did not use the shared fixture");
        var before = fake.Calls;
        fake.Fail = true;
        try { await client.CreateAsync("research-1", "RNA-seq paper", "Differential expression", "v1"); throw new InvalidOperationException("Expected lost publication create"); }
        catch (IOException) { }
        if (fake.Calls != before + 1) throw new InvalidOperationException("Publication create was retried");
        Console.WriteLine("Native publication fixture, explicit project id and no-retry tests passed.");
    }

    sealed class FakePublicationTransport : INativeSettingsClient
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
