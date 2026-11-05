using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
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
        await CreationFormAsync(expected);
        ReopenCreationForm();
        await LocalizedLayoutAsync(expected);
        Console.WriteLine("Native project create and import fixtures, explicit empty scope and no-retry tests passed.");
    }

    private static void ReopenCreationForm()
    {
        var host = new FakeProjectTransport();
        var client = new NativeProjectClient(host);
        using var first = new WorkspaceProjectCreation(client, "Use figures/");
        first.Name = "Field notes"; first.Directory = @"C:\Field notes";
        first.Description = "Measurements"; first.AgentContext = "Retain raw data.";
        first.SetStandardLayout(true);
        var seed = first.CaptureDraft(); first.Dispose();
        using var reopened = new WorkspaceProjectCreation(client, "Use figures/", seed);
        reopened.SetStandardLayout(reopened.StandardLayout);
        Check(reopened.CaptureDraft() == seed && reopened.CanCreate && host.Calls == 0,
            "closing and reopening keeps all five fields and a single convention block without contacting the host");
        reopened.Name = "Another name";
        using var fresh = new WorkspaceProjectCreation(client);
        Check(!fresh.CanCreate && !fresh.StandardLayout && fresh.AgentContext == "" && seed.Name == "Field notes",
            "a new home starts empty and editing the reopened form does not mutate the closed snapshot");
    }

    private static async Task CreationFormAsync(ProjectSummary expected)
    {
        var fake = new FakeProjectTransport { Reply = JsonSerializer.SerializeToNode(expected) };
        using var draft = new WorkspaceProjectCreation(new NativeProjectClient(fake));
        Check(!draft.CanCreate && !await draft.CreateAsync() && fake.Calls == 0,
            "empty form disables creation and never calls the host");
        draft.Name = "  RNA  ";
        Check(!draft.CanCreate, "name alone is insufficient before choosing a directory");
        draft.Directory = " \t ";
        Check(!draft.CanCreate, "whitespace directory cannot enable Create");
        draft.Directory = "  C:\\Research data\\RNA  ";
        draft.Description = "Shown only in the project switcher.";
        draft.AgentContext = "Preserve raw inputs.";
        draft.SetStandardLayout(true); draft.SetStandardLayout(true);
        Check(draft.AgentContext.Split(WorkspaceProjectCreation.StandardContext).Length == 2,
            "standard layout inserts one context block");
        draft.SetStandardLayout(false);
        Check(draft.AgentContext == "Preserve raw inputs." && !draft.StandardLayout,
            "turning the layout off preserves the user's context");
        Check(draft.CanCreate, "name and selected directory enable Create");
        fake.Pending = new();
        var creating = draft.CreateAsync();
        Check(!draft.CanCreate && draft.Busy && !await draft.CreateAsync() && fake.Calls == 1,
            "pending creation disables submission and suppresses duplicate calls");
        Check(fake.Args?["name"]?.GetValue<string>() == "RNA"
            && fake.Args?["workspace_dir"]?.GetValue<string>() == "C:\\Research data\\RNA"
            && fake.Args?["description"]?.GetValue<string>() == draft.Description
            && fake.Args?["agent_context"]?.GetValue<string>() == "Preserve raw inputs.",
            "creation trims name and path but keeps description separate from agent instructions");
        fake.Pending.SetException(new IOException("host unavailable"));
        Check(!await creating && draft.Error != null && draft.CanCreate
            && draft.Name == "  RNA  " && draft.Directory == "  C:\\Research data\\RNA  "
            && draft.AgentContext == "Preserve raw inputs." && fake.Calls == 1,
            "a failure keeps form contents and never retries automatically");
        fake.Pending = new();
        var late = draft.CreateAsync(); draft.Dispose();
        fake.Pending.SetResult(JsonSerializer.SerializeToNode(expected));
        Check(!await late && !draft.CanCreate && draft.Created == null,
            "a closed dialog discards late responses without re-enabling creation or navigation");
    }

    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }

    private static async Task LocalizedLayoutAsync(ProjectSummary expected)
    {
        foreach (var block in new[] { "Research layout\n- figures -> figures/\nKeep the project root tidy.", "科研目录\n- 图表 -> figures/\n保持根目录整洁。" })
        {
            var fake = new FakeProjectTransport { Reply = JsonSerializer.SerializeToNode(expected) };
            using var draft = new WorkspaceProjectCreation(new NativeProjectClient(fake), block)
            { Name = "Draft", Directory = "C:/fixtures/draft", Description = "User description", AgentContext = "Keep my custom instructions." };
            draft.SetStandardLayout(true); draft.SetStandardLayout(true);
            Check(draft.AgentContext == "Keep my custom instructions.\n\n" + block,
                "the chosen locale's complete multiline layout is inserted once without translating user instructions");
            foreach (var newline in new[] { "\r", "\r\n" })
            {
                draft.AgentContext = draft.AgentContext.ReplaceLineEndings(newline);
                draft.SetStandardLayout(true);
                Check(draft.AgentContext == "Keep my custom instructions.\n\n" + block,
                    "a WinUI CR or CRLF editor round-trip does not duplicate the shared LF block");
                draft.AgentContext = draft.AgentContext.ReplaceLineEndings(newline);
                draft.SetStandardLayout(false);
                Check(draft.AgentContext == "Keep my custom instructions.",
                    "turning off a native editor block removes it across newline conventions");
                draft.SetStandardLayout(true);
            }
            draft.SetStandardLayout(false);
            Check(draft.AgentContext == "Keep my custom instructions." && !draft.StandardLayout,
                "switching off removes only the injected locale's convention text");
            draft.SetStandardLayout(true);
            Check(await draft.CreateAsync() && fake.Args?["agent_context"]?.GetValue<string>() == draft.AgentContext
                && fake.Args?["description"]?.GetValue<string>() == "User description"
                && fake.Args?["standard_layout"]?.GetValue<bool>() == true,
                "the rendered localized context is exactly the context sent to the host");
        }
    }

    sealed class FakeProjectTransport : INativeSettingsClient
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
