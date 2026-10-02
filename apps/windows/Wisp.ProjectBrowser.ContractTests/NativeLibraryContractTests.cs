using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeLibraryContractTests
{
    public static async Task Run(string searchFixturePath)
    {
        var search = JsonNode.Parse(File.ReadAllText(searchFixturePath))!.AsObject();
        var scope = search["project_id"];
        if (search["schema"]?.GetValue<string>() != "wisp.native-library.v1"
            || search["command"]?.GetValue<string>() != "native_library_search"
            || (scope is not null && scope.GetValueKind() != JsonValueKind.Null)
            || search["args"]?["kind"]?.GetValue<string>() != "code")
            throw new InvalidOperationException("Native library search envelope drift");
        var fake = new FakeLibraryTransport { Reply = search["result"]!.DeepClone() };
        var client = new NativeLibraryClient(fake);
        var rows = await client.SearchAsync("RNA", "code");
        if (rows.Count != 1 || rows[0].Id != "item-1" || rows[0].CodePreview != "import pandas"
            || rows[0].SourceProjectId != "research-1" || fake.Calls != 1 || fake.ProjectId is not null
            || fake.Command != "native_library_search" || fake.Args?["query"]?.GetValue<string>() != "RNA"
            || fake.Args?["kind"]?.GetValue<string>() != "code")
            throw new InvalidOperationException("Library search did not use the shared fixture");
        fake.Fail = true;
        try { await client.SearchAsync("RNA", "code"); throw new InvalidOperationException("Expected lost library search"); }
        catch (IOException) { }
        if (fake.Calls != 2) throw new InvalidOperationException("Library search was retried");

        var deleteFixture = JsonNode.Parse(File.ReadAllText(Path.Combine(Path.GetDirectoryName(searchFixturePath)!, "delete.json")))!.AsObject();
        var deleteScope = deleteFixture["project_id"];
        if (deleteFixture["command"]?.GetValue<string>() != "native_library_delete"
            || (deleteScope is not null && deleteScope.GetValueKind() != JsonValueKind.Null)
            || deleteFixture["result"]?.GetValue<bool>() != true)
            throw new InvalidOperationException("Native library delete envelope drift");
        fake.Fail = false;
        fake.Reply = deleteFixture["result"]!.DeepClone();
        var removed = await client.DeleteAsync("item-1");
        if (!removed || fake.ProjectId is not null || fake.Command != "native_library_delete"
            || fake.Args?["id"]?.GetValue<string>() != "item-1")
            throw new InvalidOperationException("Library delete did not use the shared fixture");
        var beforeFailure = fake.Calls;
        fake.Fail = true;
        try { await client.DeleteAsync("item-1"); throw new InvalidOperationException("Expected lost library delete"); }
        catch (IOException) { }
        if (fake.Calls != beforeFailure + 1) throw new InvalidOperationException("Library delete was retried");
        Console.WriteLine("Native library search and delete fixtures, explicit empty scope and no-retry tests passed.");
    }

    sealed class FakeLibraryTransport : INativeSettingsClient
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
