using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeGeneratedQuoteTests
{
    public static async Task RunAsync()
    {
        var pending = new Queue<TaskCompletionSource<NativePanelFileContent>>();
        Task<NativePanelFileContent> Read()
        {
            var result = new TaskCompletionSource<NativePanelFileContent>(); pending.Enqueue(result); return result.Task;
        }
        var model = new NativeGeneratedPreviewModel((_, _) => Read(), (_, _) => Read());
        foreach (var (kind, mime, location) in new[] {
            ("docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", new NativeDocumentLocation("docx", Page: 1)),
            ("xlsx", "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", new NativeDocumentLocation("xlsx", Sheet: "Results", Cells: "B2")),
            ("pptx", "application/vnd.openxmlformats-officedocument.presentationml.presentation", new NativeDocumentLocation("pptx", Page: 2)) })
        {
            var output = new NativeGeneratedOutput(kind, "p", "s", "source", 0, "report." + kind, kind, "results/report." + kind, null, null);
            model.Bind([output]); var opening = model.OpenAsync(output.Id);
            var entry = model.Selected!;
            var content = new NativePanelFileContent("untrusted-alias", mime, null, "Ynl0ZXM=", false, null);
            var selection = new NativeDocumentSelection("Quoted result", location, false);
            Check(model.DocumentQuotePath(entry, content, selection) == null, "loading previews cannot quote");
            pending.Dequeue().SetResult(content); await opening;
            Check(model.DocumentQuotePath(entry, content, selection) == output.Path,
                "Office quote identity comes from the mounted output, not its content alias");
            Check(model.DocumentQuotePath(entry, content, selection with { Jump = true }) == output.Path,
                "jump intent retains the same source and position");
            Check(model.DocumentQuotePath(entry with { DisplayPath = "forged" }, content, selection) == null
                && model.DocumentQuotePath(entry, content with { Path = "forged" }, selection) == null,
                "callbacks must use the mounted entry and content references");
            Check(model.DocumentQuotePath(entry, content, selection with { Text = " " }) == null
                && model.DocumentQuotePath(entry, content, selection with { Location = new("pdf", Page: 1) }) == null,
                "empty and wrong-kind selections are rejected");
            var retry = model.RetryAsync();
            Check(model.DocumentQuotePath(entry, content, selection) == null, "retry retires the previous content immediately");
            pending.Dequeue().SetResult(content with { Truncated = true }); await retry;
            Check(model.DocumentQuotePath(model.Selected!, model.Content!, selection) == null, "truncated Office content cannot quote");
            model.Close();
            Check(model.DocumentQuotePath(entry, content, selection) == null, "closed previews reject late callbacks");
        }

        var artifact = new NativePanelArtifact("snapshot", "report.docx", "docx", ".wisp/store/blob", null, 0, "reports/live.docx");
        var registered = new NativeArtifactEntry("artifact:snapshot", artifact.Name, "docx", "reports", "", artifact.LogicalPath, null, artifact);
        model.BindCatalog([registered]); var registeredRead = model.OpenAsync(registered.Id);
        var document = new NativePanelFileContent("reports/live.docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", null, "Ynl0ZXM=", false, null);
        pending.Dequeue().SetResult(document); await registeredRead;
        var word = new NativeDocumentSelection("Snapshot text", new NativeDocumentLocation("docx", Page: 1), false);
        Check(model.DocumentQuotePath(registered, document, word) == "artifact:snapshot",
            "registered inline Office previews preserve immutable artifact identity");
        model.BindCatalog([registered with { Registered = artifact with { Id = "replacement" } }]);
        Check(model.Selected == null && model.DocumentQuotePath(registered, document, word) == null,
            "catalog replacement invalidates the old registered quote");

        var source = new NativeGeneratedOutput("generated", "p", "s", "row", 0, "report.docx", "docx", "results/report.docx", null, null);
        model.Bind([source]); var generatedRead = model.OpenAsync(source.Id); var before = model.Selected!;
        pending.Dequeue().SetResult(document); await generatedRead;
        model.Bind([source with { SessionId = "other" }]);
        Check(model.Selected == null && model.DocumentQuotePath(before, document, word) == null,
            "changed conversation scope invalidates a value-similar generated file");
        model.Bind([source with { Path = "bad\npath.docx" }]); var invalidRead = model.OpenAsync(source.Id);
        pending.Dequeue().SetResult(document); await invalidRead;
        Check(model.DocumentQuotePath(model.Selected!, model.Content!, word) == null, "control characters cannot enter quote sources");
        model.Close();

        var script = source with { Name = "analysis.py", Kind = "code", Path = "scripts/analysis.py" };
        var scriptContent = new NativePanelFileContent("scripts/analysis.py", "text/plain", "# synthetic\nalpha = 42\nprint(alpha)", null, false, null);
        var scriptQuote = NativeDocumentSelection.FromSource(scriptContent.Text!, 12, 10, false)!;
        model.Bind([script]); var scriptRead = model.OpenAsync(script.Id); var scriptEntry = model.Selected!;
        pending.Dequeue().SetResult(scriptContent); await scriptRead;
        Check(model.DocumentQuotePath(scriptEntry, scriptContent, scriptQuote) == script.Path,
            "generated source previews preserve the real file source");
        Check(model.DocumentQuotePath(scriptEntry, scriptContent, scriptQuote with { Text = "fabricated" }) == null,
            "generated source selections must occur at the reported source position");
        var scriptRetry = model.RetryAsync();
        Check(model.DocumentQuotePath(scriptEntry, scriptContent, scriptQuote) == null, "source reload immediately retires its selection");
        pending.Dequeue().SetResult(scriptContent with { Text = "replacement" }); await scriptRetry;
        Check(model.DocumentQuotePath(model.Selected!, model.Content!, scriptQuote) == null, "new content cannot reuse an earlier source selection");
        model.Close();

        var jsonOutput = script with { Name = "config.json", Kind = "json", Path = "results/config.json" };
        var jsonContent = scriptContent with { Path = jsonOutput.Path!, Text = "{\"z\":42,\"a\":1}", Mime = "application/json" };
        var jsonQuote = NativeDocumentSelection.FromPreview("\"z\": 42")!;
        model.Bind([jsonOutput]); var jsonRead = model.OpenAsync(jsonOutput.Id); var jsonEntry = model.Selected!;
        pending.Dequeue().SetResult(jsonContent); await jsonRead;
        Check(model.DocumentQuotePath(jsonEntry, jsonContent, jsonQuote) == jsonOutput.Path,
            "formatted generated JSON preserves the mounted file path without source-line metadata");
        model.Bind([jsonOutput with { SessionId = "other" }]);
        Check(model.DocumentQuotePath(jsonEntry, jsonContent, jsonQuote) == null,
            "formatted preview quotes cannot cross conversation replacement");
        model.Close();
    }
    private static void Check(bool ok, string message) { if (!ok) throw new InvalidOperationException(message); }
}
