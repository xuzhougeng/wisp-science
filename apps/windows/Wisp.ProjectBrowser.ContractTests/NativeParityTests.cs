using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeParityTests
{
    private static int checks;
    private static void Check(bool condition, string message)
    { if (!condition) throw new InvalidOperationException(message); checks++; Console.WriteLine("PASS native parity: " + message); }
    private static ProjectSummary Project(string id) => new(id, id, "", "C:/fixtures/" + id, false, 0, 0, 0, 0, 0, false, null);
    public static async Task RunAsync()
    {
        await Calendar(); await Actions(); await Conversation(); await PdfPreview();
        Console.WriteLine($"{checks} WinUI native parity checks passed.");
    }
    private static async Task PdfPreview()
    {
        var transport = new Fake();
        transport.Handler = (_, _, _) => Task.FromResult<JsonNode?>(JsonNode.Parse("{\"path\":\"paper.pdf\",\"mime\":\"application/pdf\",\"base64\":\"JVBERg==\",\"truncated\":false}"));
        var panel = new NativePanelClient(transport);
        var fixture = JsonNode.Parse(File.ReadAllText("contracts/native-conversations/v1/panel-pdf-preview.json"))!;
        await panel.ReadFileAsync("project-a", "session-a", "literature/paper.pdf");
        Check(JsonNode.DeepEquals(transport.Calls.Last().Args, fixture), "PDF file preview explicitly opts into the shared raw-page contract");
        await panel.ReadArtifactAsync("project-a", "session-a", "artifact-a");
        var call = transport.Calls.Last();
        Check(call.Project == "project-a" && call.Args["session_id"]!.GetValue<string>() == "session-a"
            && call.Args["artifact_id"]!.GetValue<string>() == "artifact-a" && call.Args["render_pdf"]!.GetValue<bool>()
            && call.Args["render_office"]!.GetValue<bool>(),
            "artifact preview preserves exact scope and opts into raw PDF and Office bytes");
        Check(NativeDocumentPreview.Kind("application/pdf") == "pdf"
            && NativeDocumentPreview.Kind("application/vnd.openxmlformats-officedocument.wordprocessingml.document") == "docx"
            && NativeDocumentPreview.Kind("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet") == "xlsx"
            && NativeDocumentPreview.Kind("application/vnd.openxmlformats-officedocument.presentationml.presentation") == "pptx",
            "supported document MIME types select the corresponding local renderer");
        Check(NativeDocumentPreview.Kind("text/html") == null && NativeDocumentPreview.Kind(null) == null
            && NativeDocumentPreview.Kind("application/vnd.ms-word.document.macroEnabled.12") == null,
            "unadvertised HTML and macro-enabled formats cannot enter the document sandbox");
        Check(NativeScientificPreview.Kind("C:/workspace/ligand.MOL2") == "structure"
            && NativeScientificPreview.Format("protein.cif") == "cif"
            && NativeScientificPreview.Format("ligand.mol2") == "mol2"
            && NativeScientificPreview.Kind("molecule.smi") == "molecule"
            && NativeScientificPreview.Format("alignment.sto") == "stockholm"
            && NativeScientificPreview.Format("alignment.aln") == "clustal"
            && NativeScientificPreview.Format("alignment.afa") == "fasta"
            && NativeScientificPreview.Kind("sequence.faa") == "fasta"
            && NativeScientificPreview.Kind("alignment.afa.html") == null,
            "scientific files dispatch exact structure/alignment formats and reject disguised suffixes");
        Check(NativePreviewResourcePolicy.Allows("https://wisp-preview.local/vendor-runtime/rdkit-worker.mjs")
            && !NativePreviewResourcePolicy.Allows("https://wisp-preview.local.evil/worker.js")
            && !NativePreviewResourcePolicy.Allows("https://wisp-preview.local:8080/worker.js")
            && !NativePreviewResourcePolicy.Allows("https://user@wisp-preview.local/worker.js")
            && !NativePreviewResourcePolicy.Allows("http://127.0.0.1/private")
            && !NativePreviewResourcePolicy.Allows("file:///C:/private.txt"),
            "all preview realms including workers can request only the packaged local origin");
    }
    private static async Task Calendar()
    {
        var transport = new Fake();
        var hold = new TaskCompletionSource<JsonNode?>();
        transport.Handler = (command, _, _) => command == "get_privacy_mode" ? hold.Task : Task.FromResult<JsonNode?>(new JsonArray());
        using var model = new WorkspaceCalendarModel(new NativeCalendarClient(transport), new NativePrivacyClient(transport), [Project("visible"), Project("hidden")]);
        var open = model.OpenAsync();
        await model.ShiftMonthAsync(1); var month = model.Month;
        await model.SelectDayAsync(month.AddDays(5));
        Check(transport.Calls.Count == 1 && !model.PrivacyReady, "navigation while privacy hangs sends no calendar read");
        hold.SetResult(JsonSerializer.SerializeToNode(new PrivacyMode(true, ["hidden"]))); await open;
        Check(model.PrivacyReady && model.VisibleProjects.Select(p => p.Id).SequenceEqual(["visible"]), "privacy removes hidden projects from filters");
        Check(transport.Calls.Skip(1).All(c => c.Project == null && c.Args["project_ids"]!.AsArray().Count == 1 && c.Args["project_ids"]![0]!.GetValue<string>() == "visible"), "every month/day payload excludes hidden projects");
        Check(transport.Calls.Last().Args["from"]!.GetValue<long>() == WorkspaceCalendarModel.Unix(month.AddDays(5)), "privacy response loads the latest chosen day");
        transport.Calls.Clear(); transport.Handler = (_, _, _) => throw new IOException("privacy unavailable");
        await model.OpenAsync(); await model.ShiftMonthAsync(1); await model.SelectDayAsync(DateTime.Today);
        Check(transport.Calls.Count == 1 && !model.PrivacyReady && model.MonthRows.Count == 0, "failed privacy read clears old data, never retries or falls back to all projects");
        var closed = new WorkspaceCalendarModel(new NativeCalendarClient(transport), new NativePrivacyClient(transport), [Project("a")]);
        hold = new(); transport.Handler = (_, _, _) => hold.Task;
        open = closed.OpenAsync(); closed.Dispose(); hold.SetResult(JsonSerializer.SerializeToNode(new PrivacyMode(false, []))); await open;
        Check(!closed.PrivacyReady, "late privacy cannot reopen a closed calendar");
        Check(WorkspaceCalendarModel.Unix(new DateTime(2026, 10, 1)) > WorkspaceCalendarModel.Unix(new DateTime(2026, 9, 30)), "local date intervals advance across a month boundary");
    }
    private static async Task Actions()
    {
        var transport = new Fake { Handler = (_, _, _) => throw new IOException("lost reply") };
        var creation = new WorkspaceProjectCreation(new NativeProjectClient(transport)) { Name = "draft", Directory = "C:/fixtures/new", Description = "keep" };
        creation.AgentContext = "Keep my instructions.";
        creation.SetStandardLayout(true); creation.SetStandardLayout(true);
        Check(creation.AgentContext.Split(WorkspaceProjectCreation.StandardContext).Length == 2, "standard layout convention is inserted only once");
        creation.SetStandardLayout(false);
        Check(creation.AgentContext == "Keep my instructions.", "disabling standard layout preserves custom instructions");
        await creation.CreateAsync();
        Check(transport.Calls.Count == 1 && creation.Name == "draft" && creation.Description == "keep" && !creation.Busy && creation.Created == null, "failed create preserves editable fields with no retry");
        transport.Handler = (_, _, _) => Task.FromResult(JsonSerializer.SerializeToNode(Project("new")));
        Check(await creation.CreateAsync() && creation.Created?.Id == "new" && transport.Calls.Last().Project == null, "create uses empty scope and returns authoritative project");
        await creation.ImportAsync("C:/fixtures/project.zip");
        Check(transport.Calls.Last().Command == "native_project_import" && transport.Calls.Last().Args["archive_path"]!.GetValue<string>() == "C:/fixtures/project.zip", "import forwards selected archive without activating another project");

        var groups = new WorkspaceSessionGroups(transport, "project-a");
        transport.Handler = (command, _, _) => Task.FromResult<JsonNode?>(command == "native_project_folders" ? JsonNode.Parse("[{\"id\":\"f1\",\"name\":\"Experiments\"}]") : JsonValue.Create(true));
        await groups.LoadAsync(); groups.Group = "folder";
        BrowserSession[] sessions = [new("s1", "project-a", "B", 2, "idle", "f1"), new("s2", "project-a", "A", 1, "idle", "unknown")];
        Check(groups.Sections(sessions)[0].Sessions.Single().Id == "s1" && groups.Sections(sessions)[1].Sessions.Single().Id == "s2", "folder grouping keeps unknown folders in ungrouped");
        groups.Group = "none"; groups.Sort = "name";
        Check(groups.Sections(sessions).Single().Sessions[0].Id == "s2", "name sorting is stable and independent from grouping");
        groups.RenamingId = "f1"; groups.Draft = "Renamed";
        await groups.SaveAsync();
        Check(transport.Calls.Any(c => c.Command == "native_project_folder_rename" && c.Project == "project-a" && c.Args["folder_id"]!.GetValue<string>() == "f1"), "rename explicitly scopes both project and folder");
        groups.RenamingId = "f1"; groups.Draft = "keep rename";
        transport.Handler = (_, _, _) => throw new IOException("lost rename"); await groups.SaveAsync();
        Check(groups.Draft == "keep rename" && groups.RenamingId == "f1", "lost rename keeps the draft and target");
        groups.Selecting = true; groups.Selected.UnionWith(["s1", "s2"]);
        transport.Handler = (_, args, _) => args["session_id"]!.GetValue<string>() == "s2" ? throw new IOException("lost move") : Task.FromResult<JsonNode?>(JsonValue.Create(true));
        await groups.MoveAsync("f1");
        Check(!groups.Selected.Contains("s1") && groups.Selected.Contains("s2") && groups.Selecting, "partial move removes confirmed successes and retains unresolved selection");

        var publication = new WorkspacePublicationModel(new NativePublicationClient(transport), "project-a") { Title = "Paper", RevisionLabel = "v1" };
        transport.Handler = (_, _, _) => Task.FromResult<JsonNode?>(JsonNode.Parse("{\"publications\":[],\"publication\":null,\"revision\":null,\"items\":[]}"));
        await publication.LoadAsync();
        transport.Handler = (_, _, _) => throw new IOException("lost publication");
        await publication.CreateAsync();
        Check(publication.Title == "Paper" && publication.RevisionLabel == "v1", "publication failure keeps manuscript draft");
        var library = new WorkspaceLibraryModel(new NativeLibraryClient(transport));
        var item = new LibraryItemSummary("l1", "code", "Analysis", "r", "plot(x)", "p", "P", "s", "S", null, 1);
        transport.Handler = (_, _, _) => Task.FromResult(JsonSerializer.SerializeToNode(new[] { item })); await library.SearchAsync();
        transport.Handler = (_, _, _) => throw new IOException("lost delete"); await library.DeleteAsync("l1");
        Check(library.Items.Count == 1 && WorkspaceLibraryModel.ComposerText(item).Contains("```r\nplot(x)"), "lost library delete keeps row and insertion retains language/source title");
        var hold = new TaskCompletionSource<JsonNode?>(); transport.Handler = (_, _, _) => hold.Task;
        var read = library.SearchAsync(); library.Dispose(); hold.SetResult(new JsonArray()); await read;
        Check(library.Items.Count == 1, "closed library ignores delayed search reply");
    }
    private static async Task Conversation()
    {
        var transport = new Fake(); ulong sequence = 0; bool running = false;
        JsonNode Snapshot(string project, string session) => JsonSerializer.SerializeToNode(new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", ++sequence, project, session, [], null, running, false, false, "m1", null, null, []), ConversationSnapshot.JsonOptions)!;
        transport.Handler = (command, args, project) => Task.FromResult<JsonNode?>(command switch
        {
            "native_conversation_snapshot" => Snapshot(project!, args["session_id"]!.GetValue<string>()),
            "native_conversation_attach" => JsonSerializer.SerializeToNode(new ComposerAttachment("uploads/data.csv", "data.csv"), ConversationSnapshot.JsonOptions),
            "list_models" => new JsonArray(),
            "native_conversation_enqueue" => new JsonObject { ["queued"] = true },
            _ => JsonValue.Create(true)
        });
        var conversation = new WorkspaceConversationModel(new NativeConversationClient(transport), transport);
        await conversation.OpenAsync("p", "s");
        Check(await conversation.AttachAsync("C:/data.csv"), "attachment reports authoritative success before clearing an upload command");
        Check(conversation.CanSend && conversation.Attachments.Single().Path == "uploads/data.csv", "attachment-only composer can send project-relative copy");
        await conversation.OpenAsync("p", "s2"); Check(conversation.Attachments.Length == 0, "attachments do not leak into another session");
        await conversation.OpenAsync("p", "s"); Check(conversation.Attachments.Length == 1, "unsent attachments return with their session draft");
        await conversation.SendAsync();
        var send = transport.Calls.Single(c => c.Command == "native_conversation_send");
        Check(send.Args["attachments"]![0]!.GetValue<string>() == "uploads/data.csv" && send.Project == "p" && conversation.Attachments.Length == 0, "send forwards attachments once and clears only after success");
        conversation.Draft = "next"; await conversation.QueueAsync();
        Check(!transport.Calls.Any(c => c.Command == "native_conversation_enqueue"), "idle turn never queues follow-up");
        running = true; await conversation.RefreshAsync(); await conversation.QueueAsync();
        Check(transport.Calls.Count(c => c.Command == "native_conversation_enqueue") == 1 && conversation.QueuedFollowUp == "next" && conversation.Draft.Length == 0, "running turn queues draft without sending");
        conversation.Draft = "second"; await conversation.QueueAsync();
        Check(transport.Calls.Count(c => c.Command == "native_conversation_enqueue") == 1, "second queued follow-up is blocked");

        await conversation.OpenAsync("p", "s3"); conversation.Draft = "uncertain";
        var handler = transport.Handler;
        transport.Handler = (cmd, args, project) => cmd == "native_conversation_enqueue" ? throw new IOException("lost queue") : handler(cmd, args, project);
        await conversation.QueueAsync(); await conversation.QueueAsync();
        Check(conversation.Draft == "uncertain" && !conversation.Busy && transport.Calls.Count(c => c.Command == "native_conversation_enqueue") == 2, "lost queue keeps draft and blocks replay");
        conversation.AcknowledgeUncertainSend();
        Check(conversation.CanQueue, "explicit verification releases an uncertain queue without replaying it");
        var hold = new TaskCompletionSource<JsonNode?>();
        transport.Handler = (cmd, args, project) => cmd == "get_bootstrap_status" ? hold.Task : handler(cmd, args, project);
        var feedback = conversation.PrepareIssueReportAsync(); conversation.Reset();
        hold.SetResult(new JsonObject { ["app_version"] = "1.0", ["os"] = "windows", ["arch"] = "x64", ["workspace"] = "secret-path" }); await feedback;
        Check(!conversation.Draft.Contains("GitHub issue"), "late feedback cannot refill after leaving the session");
        await conversation.OpenAsync("p", "s4"); await conversation.PrepareIssueReportAsync();
        Check(conversation.Draft.Contains("GitHub issue") && !conversation.Draft.Contains("secret-path") && transport.Calls.Count(c => c.Command == "native_conversation_send") == 1, "feedback fills composer without workspace path or send");
        running = false; await conversation.RefreshAsync(); conversation.Draft = "/upload";
        transport.Handler = (cmd, args, project) => cmd == "native_conversation_attach" ? throw new IOException("lost attachment") : handler(cmd, args, project);
        Check(!await conversation.AttachAsync("C:/lost.csv") && conversation.Draft == "/upload", "failed attachment preserves upload command and reports failure");
        var attachmentHold = new TaskCompletionSource<JsonNode?>();
        transport.Handler = (cmd, args, project) => cmd == "native_conversation_attach" ? attachmentHold.Task : handler(cmd, args, project);
        var attaching = conversation.AttachAsync("C:/late.csv");
        await conversation.OpenAsync("p", "s5"); conversation.Draft = "new draft";
        attachmentHold.SetResult(JsonSerializer.SerializeToNode(new ComposerAttachment("uploads/late.csv", "late.csv"), ConversationSnapshot.JsonOptions));
        Check(!await attaching && conversation.Draft == "new draft" && conversation.Attachments.Length == 0,
            "late attachment cannot report success or clear another session draft");
        var count = transport.Calls.Count; conversation.Pause(); await conversation.RefreshAsync();
        Check(transport.Calls.Count == count, "paused conversations stop background snapshot reads");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        public Func<string, JsonObject, string?, Task<JsonNode?>> Handler = (_, _, _) => Task.FromResult<JsonNode?>(null);
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        { Calls.Add((command, arguments.DeepClone().AsObject(), projectId)); return Handler(command, arguments, projectId); }
    }
}
