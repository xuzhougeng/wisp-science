using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeArtifactFilesTests
{
    public static async Task RunAsync()
    {
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        static NativeGeneratedOutput File(string id, string path, string kind = "csv") => new(id, "p", "s", "row", 0, path.Split('/').Last(), kind, path, null, null);
        var csv = File("generated-csv", "results/data.csv"); var image = File("generated-image", "figures/a.png", "image");
        NativeArtifactEntry Entry(NativeGeneratedOutput item) => new(item.Id, item.Name, item.Kind, "results/", "", item.Path, item, null);
        Check(NativeArtifactFileActions.OpensInModal(Entry(csv)) && NativeArtifactFileActions.OpensInModal(Entry(image))
            && NativeArtifactFileActions.OpensInModal(Entry(File("pdf", "report.pdf", "pdf"))), "file CSV, image and PDF use the WebView modal route");
        Check(!NativeArtifactFileActions.OpensInModal(Entry(csv with { Path = null }))
            && !NativeArtifactFileActions.OpensInModal(Entry(File("text", "report.md", "markdown"))), "inline CSV and other file kinds retain the inline route");
        Check(NativeArtifactFileActions.Items.Select(item => item.Action).SequenceEqual(new[] {
            NativeArtifactFileAction.Viewer, NativeArtifactFileAction.Center, NativeArtifactFileAction.Attach,
            NativeArtifactFileAction.Reveal, NativeArtifactFileAction.FileManager, NativeArtifactFileAction.Provenance,
            NativeArtifactFileAction.Download }), "file actions follow WebView ordering");

        var host = new Host(); var client = new NativePanelClient(host);
        var reader = WorkspaceArtifactModel.ForGenerated(client, "p", "s", [csv, image, csv with { Id = "obsolete", Superseded = true }]);
        Check(reader.Find("obsolete") == null, "superseded generated files cannot open");
        await reader.OpenAsync(csv.Id);
        Check(reader.Content?.Text == "name,value\na,1" && host.Calls.SequenceEqual(new[] { "read:results/data.csv", "provenance:results/data.csv" }), "generated IDs never reach registered-artifact RPCs");
        await reader.OpenAsync(image.Id);
        Check(!reader.CanUseLibrary && reader.CanReadProvenance && !reader.LoadingProvenance && !reader.LibraryBusy && host.Calls.Last() == "provenance:figures/a.png",
            "unregistered files use path provenance and do not invent library identities");
        var fork = reader.Fork(); await fork.OpenAsync(csv.Id);
        Check(fork.Content?.Path == csv.Path, "center-tab forks preserve file-path routing");
        var before = host.Calls.Count; var copies = 0;
        await fork.ExportItemAsync(csv.Id, _ => Task.FromResult<string?>(null), (_, _, _) => { copies++; return Task.CompletedTask; });
        Check(host.Calls.Skip(before).SequenceEqual(new[] { "export:results/data.csv" }) && copies == 0,
            "cancelled tile download resolves the original without reading a preview or copying bytes");
        await fork.ExportItemAsync(csv.Id, _ => Task.FromResult<string?>(Path.Combine(Path.GetTempPath(), "wisp-export-destination.csv")),
            (source, _, _) => { Check(source.TotalBytes == 9999, "export uses the full source descriptor"); copies++; return Task.CompletedTask; });
        Check(copies == 1 && fork.ExportedPath != null, "explicit download copies once");
        await fork.ExportItemAsync(csv.Id, _ => { fork.Close(); return Task.FromResult<string?>(Path.Combine(Path.GetTempPath(), "late.csv")); },
            (_, _, _) => { copies++; return Task.CompletedTask; });
        Check(copies == 1, "closing while the destination picker is open prevents a late copy");
        var rejected = false;
        try { WorkspaceArtifactModel.ForGenerated(client, "other", "s", [csv]); } catch (InvalidOperationException) { rejected = true; }
        Check(rejected, "generated readers reject another project before IO");
        host.FailRead = true; await reader.OpenAsync(csv.Id);
        Check(reader.Content == null && reader.ContentError == "Simulated read failure", "failed file reads stay visible and do not retry automatically");
        await reader.OpenAsync(csv.Id);
        Check(reader.Content?.Path == csv.Path && reader.ContentError == null, "explicit viewer retry clears the error and reads the current file");
        host.Hold = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var late = reader.OpenAsync(csv.Id); reader.Close(); host.Hold.SetResult(); await late;
        Check(reader.Selected == null && reader.Content == null, "late file reads cannot reopen a closed viewer");
        reader.Close(); fork.Close();
        await RegisteredInlineAsync();
        TextPreview();
    }
    private static async Task RegisteredInlineAsync()
    {
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        var host = new RegisteredHost();
        var panel = new WorkspacePanelModel(new NativePanelClient(host), "p", "s", new NativePanelTabs("artifacts", "artifacts", NativePanelTabs.All));
        await panel.RefreshAsync("artifacts");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>());
        var entries = NativeArtifactCatalog.Collect([], panel.Artifacts, "C:/review", locale);
        var markdown = entries.Single(item => item.Kind == "markdown");
        Check(!NativeArtifactFileActions.OpensInModal(markdown) && NativeArtifactFileActions.OpensInModal(entries.Single(item => item.Kind == "csv")),
            "registered Markdown stays inline while registered CSV uses the modal, just like generated files");
        var model = new NativeGeneratedPreviewModel((_, _) => throw new InvalidOperationException("Registered read used a generated path"), panel.ReadRegisteredOutputAsync);
        model.BindCatalog(entries); await model.OpenAsync(markdown.Id);
        Check(model.Content?.Text == "# Report" && host.Reads == 1 && model.Selected?.Registered?.Id == "report",
            "inline registered preview retains its actual artifact identity and scoped reader");
        model.BindCatalog(entries.Select(item => item with { Metadata = "localized label" }).ToArray());
        Check(model.Selected?.Id == markdown.Id && model.Content != null && host.Reads == 1, "presentation refresh preserves the registered selection without another read");
        var rejected = false;
        try { await panel.ReadRegisteredOutputAsync(host.Artifacts[0] with { Id = "other" }, default); }
        catch (InvalidOperationException) { rejected = true; }
        Check(rejected && host.Reads == 1, "unlisted artifacts cannot be read through the current panel");
        host.Fail = true; await model.OpenAsync(markdown.Id);
        Check(model.Error == "Read failed" && model.Content == null, "registered read failures expose an explicit retry state");
        await model.RetryAsync(); Check(model.Content?.Text == "# Report" && model.Error == null, "registered Retry keeps the ID-based reader");
        host.Hold = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var late = model.OpenAsync(markdown.Id);
        model.BindCatalog(entries.Select(item => item.Id == markdown.Id ? item with { Registered = item.Registered! with { Path = "changed.md" } } : item).ToArray());
        host.Hold.SetResult(); await late;
        Check(model.Selected == null && model.Content == null, "replacing registered source metadata invalidates a pending read");
        host.Hold = null; model.BindCatalog(entries); await model.OpenAsync(markdown.Id); model.BindCatalog([]);
        Check(model.Selected == null, "removing a registered artifact closes its preview");
    }
    private static void TextPreview()
    {
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        var head = NativeArtifactTextPreview.Create(new("report.md", "text/markdown", "# 结果\nα🙂", null, true, 9999));
        Check(head.Text == "# 结果\nα🙂" && head.ShownBytes == 15 && head.TotalBytes == 9999 && head.Truncated,
            "truncated previews retain their UTF-8 text head instead of replacing it with a note");
        var many = NativeArtifactTextPreview.Create(new("long.log", "text/plain", string.Concat(Enumerable.Repeat("x\n", 8001)), null, false, null));
        Check(many.Truncated && many.Text.Length == 16000 && many.ShownBytes == 16000 && many.TotalBytes == 16002,
            "line-heavy files stop at the WebView 8000-line limit");
        var full = NativeArtifactTextPreview.Create(new("note.txt", "text/plain", "last line", null, false, null));
        Check(!full.Truncated && full.TotalBytes == 9, "small complete text has no truncation warning");
        Check(NativeArtifactTextPreview.IsMarkdown(".wisp/snapshot/blob", "report.QMD", "text/plain")
            && NativeArtifactTextPreview.IsMarkdown("converted.rtf", "converted.rtf", "document")
            && !NativeArtifactTextPreview.IsMarkdown("source.py", "source.py", "code"),
            "registered names recover document kinds without interpreting ordinary code as Markdown");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> {
            ["en"] = new() { ["preview.text_truncated"] = "Showing {shown} of {total}" },
            ["zh"] = new() { ["preview.text_truncated"] = "显示 {shown}，共 {total}" } });
        Check(head.Notice(locale) == "Showing 15 B of 9.8 KB" && full.Notice(locale) == null, "text preview notice reports shown and source sizes");
        locale.Preview("zh"); Check(head.Notice(locale) == "显示 15 B，共 9.8 KB", "text preview notice follows locale changes");
    }
    private sealed class RegisteredHost : INativeSettingsClient
    {
        public NativePanelArtifact[] Artifacts = [new("report", "report.md", "text/markdown", ".wisp/snapshot/report.md", null, 0, "reports/report.md"),
            new("table", "table.csv", "text/csv", ".wisp/snapshot/table.csv", null, 0, "reports/table.csv")];
        public int Reads; public bool Fail; public TaskCompletionSource? Hold;
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (projectId != "p" || args["session_id"]?.GetValue<string>() != "s") throw new InvalidOperationException("Wrong registered preview scope");
            if (command == "native_conversation_panel_artifacts") return JsonSerializer.SerializeToNode(Artifacts, ConversationSnapshot.JsonOptions);
            if (command != "native_conversation_panel_readartifact" || args["artifact_id"]?.GetValue<string>() != "report" || args.ContainsKey("path"))
                throw new InvalidOperationException("Unexpected file or mutating command");
            Reads++; if (Hold != null) await Hold.Task;
            if (Fail) { Fail = false; throw new IOException("Read failed"); }
            return JsonSerializer.SerializeToNode(new NativePanelFileContent(".wisp/snapshot/report.md", "text/markdown", "# Report", null, false, null), ConversationSnapshot.JsonOptions);
        }
    }
    private sealed class Host : INativeSettingsClient
    {
        public List<string> Calls { get; } = [];
        public TaskCompletionSource? Hold;
        public bool FailRead;
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (projectId != "p" || args["session_id"]?.GetValue<string>() != "s" || args.ContainsKey("artifact_id"))
                throw new InvalidOperationException("Wrong file reader scope");
            var path = args["path"]!.GetValue<string>();
            if (command == "native_conversation_panel_file_provenance")
            {
                Calls.Add("provenance:" + path);
                return JsonSerializer.SerializeToNode(new NativeFileProvenanceReply("wisp.native-conversations.v1", "p", "s", path, null), ConversationSnapshot.JsonOptions);
            }
            if (command == "native_conversation_panel_readfile")
            {
                Calls.Add("read:" + path); if (Hold is { } hold) await hold.Task;
                if (FailRead) { FailRead = false; throw new IOException("Simulated read failure"); }
                return JsonSerializer.SerializeToNode(new NativePanelFileContent(path, "text/csv", "name,value\na,1", null, false, 14), ConversationSnapshot.JsonOptions);
            }
            if (command == "native_conversation_panel_export")
            {
                Calls.Add("export:" + path);
                return JsonSerializer.SerializeToNode(new NativePanelExport(Path.Combine(Path.GetTempPath(), "wisp-original.csv"), "data.csv", 9999), ConversationSnapshot.JsonOptions);
            }
            throw new InvalidOperationException("Unexpected registered or mutating command: " + command);
        }
    }
}
