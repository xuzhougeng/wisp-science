using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;
using Wisp.ProjectBrowser;

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
        fake.Fail = false;
        var rich = JsonNode.Parse(File.ReadAllText(Path.Combine(Path.GetDirectoryName(fixturePath)!, "workspace-evidence.json")))!;
        fake.Reply = rich["result"]!.DeepClone();
        var evidence = await client.SelectAsync("research-1", "pub-1", "rev-1");
        if (evidence.Bindings?.Single().SourceId != "artifact-version-17" || evidence.Reviews?.Single()["result"]?.GetValue<string>() != "pass"
            || evidence.Waivers?.Single()["reason"]?.GetValue<string>() != "Retained limitation" || evidence.EffectiveCapabilityLevel != "archived")
            throw new InvalidDataException("Expanded shared evidence fixture lost review, waiver or exact version");
        fake.Reply = fixture["result"]!.DeepClone();
        using var model = new WorkspacePublicationModel(client, "research-1") { Title = "Paper", RevisionLabel = "v1" };
        before = fake.Calls;
        if (model.CanCreate || await model.CreateAsync() || fake.Calls != before)
            throw new InvalidOperationException("Publication creation must wait for an authoritative empty workspace");
        await model.LoadAsync();
        if (model.CreationAvailable || await model.CreateAsync())
            throw new InvalidOperationException("Existing publication must replace the initial creation form");
        fake.Reply = JsonNode.Parse("{\"publications\":[],\"publication\":null,\"revision\":null,\"items\":[]}");
        try { await client.SelectAsync("research-1", "pub-1", null); throw new Exception("Missing selected paper accepted"); }
        catch (InvalidDataException) { }
        if (await model.LoadAsync() || model.Workspace?.Publication?.Id != "pub-1")
            throw new InvalidOperationException("Missing explicitly selected revision must not erase the current workspace");
        using var initial = new WorkspacePublicationModel(client, "research-1") { Title = "Paper", RevisionLabel = "v1" };
        await initial.LoadAsync();
        if (!initial.CanCreate) throw new InvalidOperationException("Empty workspace must enable a valid initial draft");
        initial.RevisionLabel = " "; before = fake.Calls;
        if (initial.CanCreate || await initial.CreateAsync() || fake.Calls != before || initial.Title != "Paper")
            throw new InvalidOperationException("Invalid initial draft must remain intact without invoking the host");
        initial.RevisionLabel = "v1";
        fake.Pending = new();
        var pending = initial.CreateAsync(); before = fake.Calls;
        if (await initial.CreateAsync() || fake.Calls != before || initial.CanCreate)
            throw new InvalidOperationException("Pending publication write must exclude duplicate creation");
        fake.Pending.SetResult(fixture["result"]!.DeepClone()); await pending; fake.Pending = null;
        if (initial.Title != "" || initial.CreationAvailable || initial.Workspace?.Publication?.Id != "pub-1")
            throw new InvalidOperationException("Confirmed creation must replace draft with returned publication");
        fake.Pending = new(); var late = initial.LoadAsync(); initial.Dispose();
        fake.Pending.SetResult(new JsonObject { ["publications"] = new JsonArray(), ["items"] = new JsonArray() }); await late;
        if (initial.Workspace?.Publication?.Id != "pub-1")
            throw new InvalidOperationException("Late publication read must not repopulate a closed page");
        await NativePublicationEditorTests.RunAsync();
        Console.WriteLine("Native publication fixture, explicit project id and no-retry tests passed.");
        Console.WriteLine("Publication initial-create lifecycle, duplicate exclusion and closed-view checks passed.");
    }

    sealed class FakePublicationTransport : INativeSettingsClient
    {
        public int Calls;
        public bool Fail;
        public string? Command;
        public string? ProjectId;
        public JsonObject? Args;
        public JsonNode? Reply;
        public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls++;
            Command = command;
            ProjectId = projectId;
            Args = arguments;
            if (Fail) throw new IOException("lost response");
            return Pending?.Task ?? Task.FromResult(Reply?.DeepClone());
        }
    }
}
