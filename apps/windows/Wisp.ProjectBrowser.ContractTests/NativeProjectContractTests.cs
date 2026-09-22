using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeProjectContractTests
{
    public static async Task Run(string fixturePath)
    {
        var fixture = JsonNode.Parse(File.ReadAllText(fixturePath))!.AsObject();
        var scope = fixture["project_id"];
        if (fixture["schema"]?.GetValue<string>() != "wisp.native-projects.v1"
            || fixture["command"]?.GetValue<string>() != "native_project_create"
            || (scope is not null && scope.GetValueKind() != JsonValueKind.Null)
            || fixture["args"]?["standard_layout"]?.GetValue<bool>() != false)
            throw new InvalidOperationException("Native project create envelope drift");
        var expected = fixture["result"]!.Deserialize<ProjectSummary>()
            ?? throw new InvalidOperationException("Missing project fixture");
        if (expected.Id != "research-1" || expected.WorkspaceDirectory != "/Users/researcher/Projects/RNA seq"
            || expected.SessionCount != 0)
            throw new InvalidOperationException("Native project result drift");
        var fake = new FakeProjectTransport { Reply = fixture["result"]!.DeepClone() };
        var client = new NativeProjectClient(fake);
        var created = await client.CreateAsync("RNA-seq 研究", "/Users/researcher/Projects/RNA seq", "Differential expression analysis", "Keep raw data untouched.", false);
        if (created.Id != expected.Id || fake.Calls != 1 || fake.ProjectId is not null
            || fake.Command != "native_project_create"
            || fake.Args?["standard_layout"]?.GetValue<bool>() != false
            || fake.Args?["workspace_dir"]?.GetValue<string>() != expected.WorkspaceDirectory)
            throw new InvalidOperationException("Create did not use the shared fixture arguments");
        fake.Fail = true;
        try { await client.CreateAsync("RNA-seq 研究", expected.WorkspaceDirectory, "", "", false); throw new InvalidOperationException("Expected lost create"); }
        catch (IOException) { }
        if (fake.Calls != 2) throw new InvalidOperationException("Project creation was retried");
        var importFixture = JsonNode.Parse(File.ReadAllText(Path.Combine(Path.GetDirectoryName(fixturePath)!, "import.json")))!.AsObject();
        var importScope = importFixture["project_id"];
        if (importFixture["command"]?.GetValue<string>() != "native_project_import"
            || (importScope is not null && importScope.GetValueKind() != JsonValueKind.Null))
            throw new InvalidOperationException("Native project import envelope drift");
        fake.Fail = false;
        fake.Reply = importFixture["result"]!.DeepClone();
        var imported = await client.ImportAsync("/Users/researcher/Exports/RNA seq.zip");
        if (imported.Id != "research-1" || fake.ProjectId is not null || fake.Command != "native_project_import"
            || fake.Args?["archive_path"]?.GetValue<string>() != "/Users/researcher/Exports/RNA seq.zip")
            throw new InvalidOperationException("Import did not use the shared fixture");
        var callsBeforeFailure = fake.Calls;
        fake.Fail = true;
        try { await client.ImportAsync("/Users/researcher/Exports/RNA seq.zip"); throw new InvalidOperationException("Expected lost import"); }
        catch (IOException) { }
        if (fake.Calls != callsBeforeFailure + 1) throw new InvalidOperationException("Project import was retried");
        Console.WriteLine("Native project create and import fixtures, explicit empty scope and no-retry tests passed.");
    }

    sealed class FakeProjectTransport : INativeSettingsClient
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
