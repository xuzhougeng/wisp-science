using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeArtifactQuoteTests
{
    public static async Task RunAsync()
    {
        var host = new Host(); var client = new NativePanelClient(host);
        var item = new NativePanelArtifact("snapshot", "paper.pdf", "application/pdf", ".wisp/store/blob", null, 0, "papers/current.pdf");
        var reader = new WorkspaceArtifactModel(client, "p", "s", [item]);
        var selection = new NativeDocumentSelection("Quoted finding", 2, false);
        await reader.OpenAsync(item.Id); var content = reader.Content!;
        var request = reader.DocumentQuote(item, content, selection)!;
        Check(request is { ProjectId: "p", SessionId: "s", Path: "artifact:snapshot" } && request.Selection == selection,
            "registered quote keeps the artifact identity, not its storage path or mutable workspace alias");
        Check(reader.DocumentQuote(item with { Id = "forged" }, content, selection) == null
            && reader.DocumentQuote(item, content with { Path = "forged.pdf" }, selection) == null,
            "only the mounted item and content may supply a quote");
        Check(reader.DocumentQuote(item, content, selection with { Location = new("docx", Page: 2) }) == null
            && reader.DocumentQuote(item, content, selection with { Text = " " }) == null,
            "a rendered selection must match the document kind and limits");
        await reader.OpenAsync(item.Id);
        Check(reader.DocumentQuote(item, content, selection) == null, "reloading retires the old rendered content");
        content = reader.Content!; reader.Close();
        Check(reader.DocumentQuote(item, content, selection) == null, "closed readers reject late selection callbacks");
        host.Truncated = true; await reader.OpenAsync(item.Id);
        Check(reader.DocumentQuote(item, reader.Content!, selection) == null, "truncated documents cannot quote a fabricated page");
        host.Truncated = false;

        var generated = WorkspaceArtifactModel.ForGenerated(client, "p", "s",
            [new("generated", "p", "s", "source", 0, "report.pdf", "pdf", "results/report.pdf", null, null)]);
        await generated.OpenAsync("generated");
        Check(generated.DocumentQuote(generated.Selected!, generated.Content!, selection)?.Path == "results/report.pdf",
            "generated outputs keep their actual file source without inventing a registered id");
        var fork = generated.Fork(); await fork.OpenAsync("generated");
        Check(fork.DocumentQuote(fork.Selected!, fork.Content!, selection)?.Path == "results/report.pdf",
            "opening in the main area preserves the generated source mapping");

        host.Source = "# 示例 😀\r\nalpha = 42\r\nalpha = 42\r\nprint(alpha)\r\n";
        var sourceItem = new NativePanelArtifact("code-snapshot", "analysis.py", "code", ".wisp/store/code-blob", null, 0, "analysis.py");
        var codeReader = new WorkspaceArtifactModel(client, "p", "s", [sourceItem]);
        await codeReader.OpenAsync(sourceItem.Id);
        var sourceContent = codeReader.Content!;
        var displayed = NativeArtifactSelection.SourceText(sourceContent, sourceItem.Name, sourceItem.Kind)!;
        var codeQuote = NativeDocumentSelection.FromSource(displayed, displayed.LastIndexOf("alpha = 42", StringComparison.Ordinal), 10, false)!;
        Check(codeReader.DocumentQuote(sourceItem, sourceContent, codeQuote)?.Path == "artifact:code-snapshot"
            && codeQuote.Location.Line == 3, "source selection retains the registered snapshot and the selected occurrence's line");
        Check(codeReader.DocumentQuote(sourceItem, sourceContent, codeQuote with { Text = "alpha = 99" }) == null
            && codeReader.DocumentQuote(sourceItem, sourceContent, codeQuote with { Location = codeQuote.Location with { Line = 1, EndLine = 1 } }) == null
            && codeReader.DocumentQuote(sourceItem, sourceContent, codeQuote with { Location = codeQuote.Location with { Unsaved = true } }) == null,
            "source quotes reject fabricated text, positions and unsaved status");
        await codeReader.OpenAsync(sourceItem.Id);
        Check(codeReader.DocumentQuote(sourceItem, sourceContent, codeQuote) == null, "reload invalidates source quote snapshots too");
        var sourceFork = codeReader.Fork(); await sourceFork.OpenAsync(sourceItem.Id);
        Check(sourceFork.DocumentQuote(sourceItem, sourceFork.Content!, codeQuote)?.Path == "artifact:code-snapshot",
            "center reader forks retain source snapshot identity");
        var json = sourceContent with { Path = "config.json", Text = "{\"value\":42}" };
        Check(NativeArtifactSelection.SourceText(json, "config.json", "json") == null,
            "formatted JSON cannot mislabel displayed lines as original source locations");
        Check(NativeArtifactSelection.SourceText(sourceContent with { Truncated = true }, sourceItem.Name, sourceItem.Kind) == null
            && NativeArtifactSelection.SourceText(sourceContent with { Text = string.Concat(Enumerable.Repeat("x\n", 8001)) }, sourceItem.Name, sourceItem.Kind) == null
            && NativeArtifactSelection.SourceText(sourceContent with { Path = "notes.md" }, "notes.md", "markdown") == null,
            "truncated and rendered Markdown content do not masquerade as complete code sources");
        sourceFork.Close(); codeReader.Close(); host.Source = null;

        host.Source = "# Results\n\nA **bold** result with `alpha = 42`.";
        var markdownItem = sourceItem with { Name = "notes.md", Kind = "markdown" };
        var markdownReader = new WorkspaceArtifactModel(client, "p", "s", [markdownItem]);
        await markdownReader.OpenAsync(markdownItem.Id); var markdownContent = markdownReader.Content!;
        var previewQuote = NativeDocumentSelection.FromPreview("A bold result with alpha = 42.", true)!;
        Check(markdownReader.DocumentQuote(markdownItem, markdownContent, previewQuote)?.Path == "artifact:code-snapshot",
            "rendered Markdown retains the registered snapshot instead of its live alias");
        Check(markdownReader.DocumentQuote(markdownItem with { Name = "other.md" }, markdownContent, previewQuote) == null
            && markdownReader.DocumentQuote(markdownItem, markdownContent with { Text = "replacement" }, previewQuote) == null,
            "preview quotes require mounted item and content identity");
        await markdownReader.OpenAsync(markdownItem.Id);
        Check(markdownReader.DocumentQuote(markdownItem, markdownContent, previewQuote) == null,
            "rendered quotes from an earlier read cannot survive reload");
        markdownContent = markdownReader.Content!; markdownReader.Close();
        Check(markdownReader.DocumentQuote(markdownItem, markdownContent, previewQuote) == null,
            "a closed rendered preview rejects late quote callbacks");
        host.Source = null;

        var conversation = new WorkspaceConversationModel(new NativeConversationClient(host));
        await conversation.OpenAsync("p", "s"); conversation.Draft = "Keep this question";
        bool Accept(NativeArtifactDocumentQuote quote) => conversation.AddDocumentQuote(quote.ProjectId, quote.SessionId, quote.Path, quote.Selection);
        Check(Accept(request) && conversation.Quotes.Single().Document?.Path == "artifact:snapshot"
            && conversation.Draft == "Keep this question", "artifact selections enter the existing draft with stable provenance");
        Check(Accept(request) && conversation.Quotes.Length == 1, "the same artifact selection is deduplicated");
        await conversation.OpenAsync("p", "other");
        Check(!Accept(request) && conversation.Quotes.Length == 0, "a stale modal cannot quote into another session");
        await conversation.OpenAsync("other-project", "s");
        Check(!Accept(request), "a stale center reader cannot quote into another project");
        await conversation.OpenAsync("p", "s"); host.ReadOnly = true; await conversation.RefreshAsync();
        Check(!Accept(request), "read-only recordings cannot accept artifact quotes");
        Check(host.Calls.All(command => !command.Contains("send") && !command.Contains("enqueue")), "quoting never sends or queues a model request");
        conversation.Pause(); generated.Close(); fork.Close(); reader.Close();
    }
    private static void Check(bool ok, string message) { if (!ok) throw new InvalidOperationException(message); }
    private sealed class Host : INativeSettingsClient
    {
        public bool Truncated, ReadOnly; public string? Source; private ulong sequence;
        public readonly List<string> Calls = [];
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add(command); var session = args["session_id"]?.GetValue<string>() ?? "s";
            object? value = command switch
            {
                "native_conversation_panel_readartifact" or "native_conversation_panel_readfile" => Source is { } source
                    ? new NativePanelFileContent(".wisp/store/code-blob", "text/plain", source, null, Truncated, null)
                    : new NativePanelFileContent(".wisp/store/blob", "application/pdf", null, "cGRm", Truncated, 3),
                "native_conversation_panel_artifact_provenance" => new NativeArtifactProvenanceReply("wisp.native-conversations.v1", projectId!, session, args["artifact_id"]!.GetValue<string>(), "papers/current.pdf", null),
                "native_conversation_panel_file_provenance" => new NativeFileProvenanceReply("wisp.native-conversations.v1", projectId!, session, args["path"]!.GetValue<string>(), null),
                "native_conversation_snapshot" => new ConversationSnapshot(ConversationSnapshot.SchemaId, "memory", ++sequence, projectId!, session, [], null, false, false, ReadOnly, "model", null, null, []),
                _ => null,
            };
            return Task.FromResult(value == null ? null : JsonSerializer.SerializeToNode(value, ConversationSnapshot.JsonOptions));
        }
    }
}
