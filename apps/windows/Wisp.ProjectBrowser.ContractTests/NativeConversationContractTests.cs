using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

static class NativeConversationContractTests
{
    public static async Task Run(string projectFixture)
    {
        var directory = Path.GetFullPath(Path.Combine(Path.GetDirectoryName(projectFixture)!, "../../native-conversations/v1"));
        var timeline = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "trajectory-layout.json")))!;
        foreach (var item in timeline["cases"]!.AsArray())
        {
            var turns = item!["turns"]!.Deserialize<NativeTrajectoryTurn[]>(ConversationSnapshot.JsonOptions)!;
            var rows = NativeTrajectoryRow.Collect(turns, item["query"]!.GetValue<string>());
            var actual = NativeTrajectorySegment.Collect(rows, Enum.Parse<NativeTrajectoryAxis>(item["axis"]!.GetValue<string>(), true));
            var expected = item["segments"]!.Deserialize<NativeTrajectorySegment[]>(ConversationSnapshot.JsonOptions)!;
            Require(actual.Length == expected.Length, "Timeline segment count drift");
            foreach (var (a, e) in actual.Zip(expected))
                Require(a.Key == e.Key && a.Lane == e.Lane && Math.Abs(a.LeftPct - e.LeftPct) < 0.000001 && Math.Abs(a.WidthPct - e.WidthPct) < 0.000001, "Timeline position drift");
        }
        var timedTurns = timeline["cases"]![0]!["turns"]!.Deserialize<NativeTrajectoryTurn[]>(ConversationSnapshot.JsonOptions)!;
        foreach (var expected in timeline["timing"]!.AsArray())
        {
            var timing = NativeTrajectoryTiming.Collect(timedTurns.Single(turn => turn.Index == expected!["turn"]!.GetValue<long>()).Cells);
            Require(timing == new NativeTrajectoryTiming(expected!["input"]!.GetValue<double>(), expected["model"]!.GetValue<double>(), expected["tools"]!.GetValue<double>()), "Per-turn timing drift");
        }
        var recorded = timedTurns[0].Cells;
        Require(recorded[2].Status(true) == "running" && recorded[2].Status(false) == "pending" && recorded[1].Status(true) == "completed", "Trajectory status drift");
        Require((recorded[2] with { Ok = false }).Status(true) == "error", "Trajectory failure masked");
        Require(recorded[0].Preview == "Question" && recorded[0].Source == "Question" && JsonNode.Parse(recorded[0].RawJson)?["kind"]?.GetValue<string>() == "user", "Trajectory inspector text drift");
        var provenanceItems = JsonSerializer.Deserialize<ConversationItem[]>(File.ReadAllText(Path.Combine(directory, "panel-provenance.json")), ConversationSnapshot.JsonOptions)!;
        var provenance = NativeProvenanceRow.Collect(provenanceItems);
        Require(provenance.Select(row => row.Index).SequenceEqual(new[] { 1, 3, 4, 5 }), "Tool source order drift");
        Require(provenance.Select(row => row.InitiallyExpanded).SequenceEqual(new[] { false, true, true, false }), "Provenance disclosure defaults drift");
        Require(provenance[0].Output == "42\n" && provenance[1].Input == "样本.csv" && provenance[3].Input == "", "Provenance recorded text drift");
        Require(provenance[1].Matches("not FOUND") && !provenance[1].Matches("python") && provenance[2].Matches("LS"), "Provenance search drift");
        Require(NativeProvenanceRow.Collect(new[] { provenanceItems[4] }).Single().Index == 0 && NativeProvenanceRow.Collect(new[] { provenanceItems[0] }).Length == 0, "Transcript page isolation drift");
        var highlightFake = new Fake { Reply = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "panel-highlights.json"))) };
        var highlights = new NativeHighlightClient(highlightFake);
        Require((await highlights.ListAsync("project-a", "session-a")).Single().Code == "样本 质量\n合格", "Highlight text drift");
        try { await highlights.ListAsync("other", "session-a"); throw new Exception("Expected scope rejection"); } catch (InvalidDataException) { }
        highlightFake.Fail = true;
        try { await highlights.RemoveAsync("project-a", "session-a", "highlight-a"); } catch (IOException) { }
        Require(highlightFake.Calls == 3 && highlightFake.Args?["library_item_id"]?.GetValue<string>() == "highlight-a", "Highlight removal replayed or lost identity");
        Require(NativeSavedExcerpt.Find("样本 质量\n合格", "样本质量合格") is { } range && "样本 质量\n合格"[range] == "样本 质量\n合格", "Saved excerpt whitespace match drift");
        Require(NativeSavedExcerpt.Find("abc", " ") is null && NativeSavedExcerpt.Find("abc", "ABC") is null, "Saved excerpt empty/case matching drift");
        highlightFake.Fail = false;
        highlightFake.Reply = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "panel-highlights.json")))![0]!.DeepClone();
        await highlights.StarAsync("project-a", "session-a", "样本 质量\n合格");
        try { await highlights.StarAsync("project-a", "session-a", "other text"); throw new Exception("Expected selection mismatch"); } catch (InvalidDataException) { }
        highlightFake.Fail = true;
        var beforeSave = highlightFake.Calls;
        try { await highlights.StarAsync("project-a", "session-a", "new text"); } catch (IOException) { }
        Require(highlightFake.Calls == beforeSave + 1 && highlightFake.Args?["text"]?.GetValue<string>() == "new text", "Selection save replayed or changed text");
        Require(NativeSavedExcerpt.FindAll("🧬 A B / 🧬AB", "🧬AB").Select(range => "🧬 A B / 🧬AB"[range]).SequenceEqual(new[] { "🧬 A B", "🧬AB" }), "Repeated excerpt underline ranges drift");
        var notebookFixture = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "panel-notebook.json")))!;
        var notebookCells = NativeNotebookCell.Collect(notebookFixture["items"]!.Deserialize<ConversationItem[]>(ConversationSnapshot.JsonOptions)!);
        Require(notebookCells.SequenceEqual(notebookFixture["cells"]!.Deserialize<NativeNotebookCell[]>(ConversationSnapshot.JsonOptions)!), "Notebook projection differs from WebView fixture");
        Require(notebookCells[5].OutputInitiallyExpanded && !notebookCells[3].OutputInitiallyExpanded && notebookCells[6].Status == "running", "Notebook output/status drift");
        var notebookFake = new Fake { Reply = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "panel-notebook-stars.json"))) };
        var notebookClient = new NativeNotebookClient(notebookFake);
        Require((await notebookClient.ListStarsAsync("project-a", "session-a")).Single().Matches(notebookCells[3]), "Notebook saved code mismatch");
        notebookFake.Reply = notebookFake.Reply![0]!.DeepClone();
        await notebookClient.StarAsync("project-a", "session-a", "python", "print(1)");
        try { await notebookClient.StarAsync("project-a", "session-a", "r", "print(1)"); throw new Exception("Expected code identity rejection"); } catch (InvalidDataException) { }
        notebookFake.Fail = true;
        try { await notebookClient.UnstarAsync("project-a", "session-a", "code-a"); } catch (IOException) { }
        Require(notebookFake.Calls == 4 && notebookFake.Args?["library_item_id"]?.GetValue<string>() == "code-a", "Notebook mutation replayed or lost identity");
        Require(NativeSideChatKeyboard.ResolveReturn(false, false) == NativeSideChatReturnAction.Send, "Side-chat Return must send");
        Require(NativeSideChatKeyboard.ResolveReturn(true, false) == NativeSideChatReturnAction.Newline, "Side-chat Shift-Return must insert a newline");
        Require(NativeSideChatKeyboard.ResolveReturn(false, true) == NativeSideChatReturnAction.Composition && NativeSideChatKeyboard.ResolveReturn(true, true) == NativeSideChatReturnAction.Composition, "IME confirmation must not send");
        var sideFake = new Fake { Reply = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "panel-side-chat.json"))) };
        var sideClient = new NativeSideChatClient(sideFake);
        var sideReply = await sideClient.AskAsync("project-a", "session-a", "进展如何", "agent-a");
        Require(sideReply.Evidence.Single().EventSeq == 40 && sideReply.SnapshotVersion == 42 && sideFake.Args?["acp_agent_id"]?.GetValue<string>() == "agent-a", "Side-chat evidence or agent identity drift");
        try { await sideClient.AskAsync("project-a", "other", "question"); throw new Exception("Expected side-chat scope rejection"); } catch (InvalidDataException) { }
        sideFake.Fail = true;
        try { await sideClient.AskAsync("project-a", "session-a", "question"); } catch (IOException) { }
        Require(sideFake.Calls == 3, "Side-chat question was replayed");
        Require(NativeSideChatQuote.Question("解释", new[] { new NativeSideChatQuote("one\ntwo", "a`b\nc") }) == "Selected excerpt from reference `a\\`b c`:\n> one\n> two\n\n解释", "Side-chat reference formatting drift");
        var panel = JsonSerializer.Deserialize<NativePanelFile[]>(File.ReadAllText(Path.Combine(directory, "panel-files.json")), ConversationSnapshot.JsonOptions)!;
        Require(panel[0].IsDir && panel[1].Name == "README.md", "Panel file fixture drift");
        var preview = JsonSerializer.Deserialize<NativePanelFileContent>(File.ReadAllText(Path.Combine(directory, "panel-preview.json")), ConversationSnapshot.JsonOptions)!;
        Require(preview.Truncated && preview.TotalBytes == 8000000, "Truncated preview drift");
        var terminal = JsonSerializer.Deserialize<NativeTerminalOutput>(File.ReadAllText(Path.Combine(directory, "terminal-output.json")), ConversationSnapshot.JsonOptions)!;
        Require(System.Text.Encoding.UTF8.GetString(terminal.Bytes("terminal-a", null)) == "hello", "Terminal byte fixture drift");
        try { terminal.Bytes("other", null); throw new Exception("Expected terminal scope rejection"); } catch (InvalidDataException) { }
        try { (terminal with { Reset = false }).Bytes("terminal-a", 5); throw new Exception("Expected terminal replay rejection"); } catch (InvalidDataException) { }
        var share = JsonSerializer.Deserialize<NativeShareRow[]>(File.ReadAllText(Path.Combine(directory, "share.json")), ConversationSnapshot.JsonOptions)!;
        Require(share.Length == 3 && share[1].Role == "reasoning", "Share fixture drift");
        var complexShare = JsonSerializer.Deserialize<NativeShareRow[]>(File.ReadAllText(Path.Combine(directory, "share-complex.json")), ConversationSnapshot.JsonOptions)!;
        Require(complexShare.Length == 1 && complexShare[0].Text.Contains("| 样本 A | **Passed** |") && complexShare[0].Text.Contains("   - Check **read depth**"), "Complex share Markdown must survive the native contract");
        var saveFake = new Fake { Reply = JsonValue.Create(true) };
        var fileClient = new NativePanelClient(saveFake);
        await fileClient.SaveFileAsync("project-a", "session-a", "analysis.py", "old", "new");
        Require(saveFake.Project == "project-a" && saveFake.Args?["session_id"]?.GetValue<string>() == "session-a" && saveFake.Args?["original_text"]?.GetValue<string>() == "old" && saveFake.Args?["text"]?.GetValue<string>() == "new", "Native file save scope or conflict baseline lost");
        saveFake.Fail = true;
        try { await fileClient.SaveFileAsync("project-a", "session-a", "analysis.py", "old", "new"); throw new Exception("Expected lost save response"); } catch (IOException) { }
        Require(saveFake.Calls == 2, "File saves must not replay after an uncertain response");
        var actionFake = new Fake { Reply = JsonValue.Create(true) };
        var actionClient = new NativePanelClient(actionFake);
        await actionClient.FileActionAsync("project-a", "session-a", NativePanelFileAction.Rename, "data/a", "data/b");
        Require(actionFake.Project == "project-a" && actionFake.Args?["session_id"]?.GetValue<string>() == "session-a" && actionFake.Args?["file_action"]?.GetValue<string>() == "rename" && actionFake.Args?["new_path"]?.GetValue<string>() == "data/b", "File rename scope or destination lost");
        actionFake.Fail = true;
        try { await actionClient.FileActionAsync("project-a", "session-a", NativePanelFileAction.Delete, "data/b"); throw new Exception("Expected lost delete reply"); } catch (IOException) { }
        Require(actionFake.Calls == 2, "Uncertain file action must not replay");
        var contexts = JsonSerializer.Deserialize<NativePanelContexts>(File.ReadAllText(Path.Combine(directory, "panel-contexts.json")), ConversationSnapshot.JsonOptions)!;
        Require(contexts.Attached.Select(c => c.Id).SequenceEqual(new[] { "local", "ssh:gpu" }) && contexts.Available.Single().Id == "wsl:ubuntu", "Context session scope drift");
        var activity = JsonSerializer.Deserialize<NativeContextActivity>(File.ReadAllText(Path.Combine(directory, "panel-activity.json")), ConversationSnapshot.JsonOptions)!;
        Require(activity.Runtimes[0].Key.SessionId == "session-a" && activity.Runtimes[0].ResidentMemoryBytes == 104857600 && activity.Runs[0].Status == "running", "Activity contract casing drift");
        var run = JsonSerializer.Deserialize<NativeRun>(File.ReadAllText(Path.Combine(directory, "panel-run.json")), ConversationSnapshot.JsonOptions)!;
        Require(run.StdoutTail == "Processed 10 samples", "Run detail drift");
        var objects = JsonSerializer.Deserialize<NativeRuntimeObjects>(File.ReadAllText(Path.Combine(directory, "panel-runtime-objects.json")), ConversationSnapshot.JsonOptions)!;
        Require(objects.TotalCount == 1 && objects.Objects[0].TypeName == "list", "Runtime inspection drift");
        var execution = JsonSerializer.Deserialize<NativeRuntimeExecution>(File.ReadAllText(Path.Combine(directory, "panel-runtime-execution.json")), ConversationSnapshot.JsonOptions)!;
        Require(execution.Text == "[stdout]\n42" && execution.Plots.Length == 0, "Runtime execution fixture drift");
        var agents = JsonSerializer.Deserialize<NativeAgentSnapshot[]>(File.ReadAllText(Path.Combine(directory, "panel-agents.json")), ConversationSnapshot.JsonOptions)!;
        var agentResult = JsonSerializer.Deserialize<NativeAgentResult>(File.ReadAllText(Path.Combine(directory, "panel-agent-result.json")), ConversationSnapshot.JsonOptions)!;
        Require(agents[0].Workflow.FrameId == "session-a" && agents[0].Dynamic.Tasks[0].StoredStepId == agentResult.StepId, "Agent workflow/result identity drift");
        var archiveNode = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "archive.json")));
        var archive = NativeResearchArchive.Decode(archiveNode, "project-a", "session-a")!;
        Require(archive.Confirmation().Files[0].Path == "results/qc.txt" && archive.FrozenAt is null, "Archive fixture drift");
        try { NativeResearchArchive.Decode(archiveNode, "other", "session-a"); throw new Exception("Expected archive scope rejection"); } catch (InvalidDataException) { }
        var inbox = JsonSerializer.Deserialize<NativeInboxEntry[]>(File.ReadAllText(Path.Combine(directory, "inbox.json")), ConversationSnapshot.JsonOptions)!;
        Require(inbox.Length == 2 && inbox[1].ProjectId == "project-b" && inbox[1].Id == "session-b", "Inbox cross-project identity drift");
        var trajectoryNode = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "trajectory.json")));
        var trajectory = NativeTrajectory.Decode(trajectoryNode, "session-a");
        Require(trajectory.Turns[0].Cells[0].DurationMs == 40 && trajectory.Stats.OutputTokens == 20, "Trajectory fixture drift");
        try { NativeTrajectory.Decode(trajectoryNode, "other"); throw new Exception("Expected trajectory scope rejection"); } catch (InvalidDataException) { }
        var outline = JsonSerializer.Deserialize<ConversationOutlineEntry[]>(File.ReadAllText(Path.Combine(directory, "outline.json")), ConversationSnapshot.JsonOptions)!;
        Require(outline.Length == 2 && outline[0].BeforeSeq == 8 && outline[1].BeforeSeq is null && outline[1].UserIndex == 1, "Outline cursor drift");
        var node = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "snapshot.json")))!;
        var snapshot = ConversationSnapshot.Decode(node, "project-a", "session-a");
        Require(snapshot.Items.Length == 2 && snapshot.Approvals[0].ApprovalId == "approval-a", "Shared snapshot drift");
        try { ConversationSnapshot.Decode(node, "other", "session-a"); throw new Exception("Expected scope rejection"); } catch (InvalidDataException) { }
        var cursor = new ConversationCursor("project-a", "session-a");
        Require(cursor.TryAccept(snapshot), "First snapshot rejected");
        Require(!cursor.TryAccept(snapshot) && !cursor.TryAccept(snapshot with { Sequence = 1 }), "Duplicate/older snapshot replayed");
        Require(cursor.TryAccept(snapshot with { Epoch = "host-two", Sequence = 1 }), "Restart did not reset sequence");
        Require(!cursor.TryAccept(snapshot with { Sequence = 100 }), "Retired host response was applied");
        var fake = new Fake(); var client = new NativeConversationClient(fake);
        await client.ApproveAsync("project-a", "session-a", "approval-a", false);
        Require(fake.Args?["approval_id"]?.GetValue<string>() == "approval-a" && fake.Project == "project-a", "Approval lost identity");
        fake.Fail = true;
        try { await client.SendAsync("project-a", "session-a", Guid.NewGuid(), "hello"); } catch (IOException) { }
        Require(fake.Calls == 2, "Ambiguous send was replayed");
        var agentFake = new Fake { Reply = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "panel-agent-result.json"))) };
        var agentClient = new NativeAgentPanelClient(agentFake);
        await agentClient.ResultAsync("project-a", "session-a", "workflow-a", "workflow-a:review");
        Require(agentFake.Args?["session_id"]?.GetValue<string>() == "session-a" && agentFake.Project == "project-a", "Agent request lost scope");
        try { await agentClient.ResultAsync("project-a", "session-a", "wrong", "workflow-a:review"); throw new Exception("Expected result mismatch"); } catch (InvalidDataException) { }
        var approvalArgs = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "panel-agent-action.json")))!;
        agentFake.Fail = true;
        try { await agentClient.ActAsync("project-a", "session-a", "workflow-a", NativeAgentAction.Approve, approvalArgs["expected_version"]!.GetValue<long>()); } catch (IOException) { }
        Require(agentFake.Calls == 3 && agentFake.Args?["expected_version"]?.GetValue<long>() == 7, "Agent approval replayed or lost reviewed version");
        agentFake.Fail = false;
        await agentClient.ActAsync("project-a", "session-a", "workflow-a", NativeAgentAction.Retry, budgets: new Dictionary<string, NativeAgentBudgetOverride> { ["review"] = new(0) });
        Require(agentFake.Args?["budget_overrides"]?["review"]?["max_tokens"]?.GetValue<uint>() == 0, "Unlimited retry budget lost");
        agentFake.Reply = JsonValue.Create(true);
        Require(await agentClient.GetDelegationAsync("project-a", "session-a"), "Missing confirmed delegation state");
        Require(!agentFake.Args!.ContainsKey("enabled"), "Read delegation unexpectedly writes a value");
        agentFake.Fail = true; var callsBeforeToggle = agentFake.Calls;
        try { await agentClient.SetDelegationAsync("project-a", "session-a", false); } catch (IOException) { }
        Require(agentFake.Calls == callsBeforeToggle + 1 && agentFake.Args?["enabled"]?.GetValue<bool>() == false, "Delegation save lost false or was replayed");
        var runtimeFake = new Fake(); var runtimeClient = new NativeContextActivityClient(runtimeFake);
        await runtimeClient.StopRuntimeAsync("project-a", "session-a", "runtime-a", 2);
        Require(runtimeFake.Args?["runtime_generation"]?.GetValue<ulong>() == 2 && runtimeFake.Args?["session_id"]?.GetValue<string>() == "session-a", "Runtime stop lost generation/scope");
        runtimeFake.Fail = true;
        try { await runtimeClient.ExecuteAsync("project-a", "session-a", "local", "python", "print(42)"); } catch (IOException) { }
        Require(runtimeFake.Calls == 2 && runtimeFake.Args?["code"]?.GetValue<string>() == "print(42)", "Uncertain runtime execution replayed or code changed");
        var attachFixture = JsonNode.Parse(File.ReadAllText(Path.Combine(directory, "attach.json")))!.AsObject();
        var attachFake = new Fake { Reply = attachFixture["result"]!.DeepClone() };
        var attachClient = new NativeConversationClient(attachFake);
        var attached = await attachClient.AttachAsync("project-a", "session-a", attachFixture["args"]!["path"]!.GetValue<string>());
        Require(attached.Path == "uploads/notes.csv" && attached.Name == "notes.csv" && attachFake.Command == "native_conversation_attach" && attachFake.Project == "project-a" && attachFake.Args?["session_id"]?.GetValue<string>() == "session-a", "Attach fixture drift");
        attachFake.Fail = true;
        try { await attachClient.AttachAsync("project-a", "session-a", "/tmp/notes.csv"); throw new Exception("Expected lost attach"); }
        catch (IOException) { }
        Require(attachFake.Calls == 2, "Attach was retried");
        var savedItem = JsonSerializer.Deserialize<ConversationItem>(File.ReadAllText(Path.Combine(directory, "attached-item.json")), ConversationSnapshot.JsonOptions)!;
        Require(savedItem.Attachments is ["uploads/notes.csv", "uploads/figure.png"] && savedItem.Text.Contains("uploads/notes.csv"), "Saved snapshot dropped attachments");
        Console.WriteLine("Native conversation fixture, ordering, restart, approval and no-replay tests passed.");
    }
    static void Require(bool value, string message) { if (!value) throw new Exception(message); }
    sealed class Fake : INativeSettingsClient
    {
        public JsonObject? Args; public string? Project; public string? Command; public int Calls; public bool Fail; public JsonNode? Reply;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls++; Args = arguments; Project = projectId; Command = command;
            if (Fail) throw new IOException("Lost response");
            return Task.FromResult<JsonNode?>(Reply);
        }
    }
}
