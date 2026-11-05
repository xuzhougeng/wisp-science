using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;

internal static class NativeGeneratedArtifactsTests
{
    private static void Check(bool ok, string reason) { if (!ok) throw new InvalidOperationException(reason); }
    private static ConversationItem Row(string role, string text, string? tool = null) => new(role, text, tool, null, true, null);
    private static ConversationSnapshot Page(params ConversationItem[] items) => new(ConversationSnapshot.SchemaId, "host", 1, "p", "s", items,
        null, false, false, false, "model", null, null, [], UserOffset: 0);
    private static NativeLocalization Locale() => new(new Dictionary<string, Dictionary<string, string>> {
        ["en"] = new() { ["artifact.table"] = "Table {n}", ["artifact.equation"] = "Equation {n}" },
        ["zh"] = new() { ["artifact.table"] = "表格 {n}", ["artifact.equation"] = "公式 {n}" } }, "en");
    private const string Table = "| Sample | Value |\n| --- | --- |\n| A | 12 |";
    public static async Task RunAsync()
    {
        var locale = Locale(); var outputs = new NativeGeneratedArtifacts();
        var cell = NativeGeneratedArtifacts.ParseCell("# literal | **bold** and `code`");
        Check(cell.Single() is ParagraphBlock && cell.Descendants<EmphasisInline>().Single().DelimiterCount == 2
            && cell.Descendants<CodeInline>().Single().Content == "code"
            && cell.Descendants<LiteralInline>().First().Content.ToString() == "# literal | ",
            "preview cells parse inline formatting without treating headings or literal pipes as table structure");
        Check(NativeTranscriptRows.RendersNothing(Row("file_changed", "a.csv"))
            && !NativeTranscriptRows.RendersNothing(Row("file_changed", "a.csv"), modelView: true),
            "file changes live in generated directories while raw context inspection retains the source row");
        var page = Page(Row("user", "Uploaded files: './inputs/a.csv', ./inputs/a.csv script.py"), Row("tool", Table, "shell"),
            Row("reasoning", Table), Row("file_changed", "C:/project/results/a.csv"), Row("assistant", "Checking results."),
            Row("tool", "done", "shell"), Row("assistant", Table + "\n\n```csv\nname,value\na,1\n```\n\n```fasta\n>sample\nACGT\n```\n\n$$\nx^2\n$$"),
            Row("tool", Table, "attempt_completion"), Row("user", "Update"), Row("file_changed", "C:/project/results/a.csv"), Row("assistant", "Updated."));
        Check(outputs.Update(page, locale) && outputs.Items.Count == 8, "only user files, file changes and assistant/completion materials are collected; duplicate uploads collapse");
        Check(outputs.Items.Select(o => o.Name).SequenceEqual(new[] { "a.csv", "a.csv", "Table 1", "data-1.csv", "alignment-1.fasta", "Equation 1", "Table 2", "a.csv" }), "canonical global numbering spans output types and completion messages");
        var keys = NativeTranscriptRows.Keys(page);
        Check(outputs.At(keys[6]).Length == 5 && outputs.Items[1].Superseded && !outputs.Items[^1].Superseded,
            "file changes attach to the final answer past commentary and earlier versions become unavailable");
        Check(outputs.Items[5].Text == "x^2\n" && outputs.Items[3].Table?.CopyText == "name\tvalue\na\t1", "math and CSV preserve canonical extracted content");
        var table = outputs.Items[2];
        Check(!outputs.Update(page with { Sequence = 9 }, locale) && ReferenceEquals(table.Table, outputs.Items[2].Table), "polling reuses parsed material");
        locale.Preview("zh"); outputs.Update(page, locale);
        Check(outputs.Find(table.Id)?.Name == "表格 1" && ReferenceEquals(table.Table, outputs.Find(table.Id)?.Table), "localization changes labels without replacing stable output identities");
        Check(NativeGeneratedArtifacts.Directory(outputs.Items[1], "c:\\project") == "results"
            && NativeGeneratedArtifacts.Directory(outputs.Items[1], "D:/other") == "results"
            && NativeGeneratedArtifacts.Directory(table, "C:/project") == "", "directory groups distinguish project paths, outside parents and inline material");
        Check(NativeGeneratedArtifacts.FileKind("csv") == null && NativeGeneratedArtifacts.FileKind("script.py") == null
            && NativeGeneratedArtifacts.FileKind("report.DOCX") == "docx", "only recognized file extensions become artifacts");
        var moved = Page(Row("file_changed", "a.csv"), Row("user", "next"), Row("assistant", "Answer"));
        outputs.Update(moved, locale); Check(outputs.Items.Single().SourceItem == 0, "file ownership never crosses the next user turn");
        outputs.Update(page, locale);
        var pending = new List<TaskCompletionSource<NativePanelFileContent>>(); var reads = 0;
        var model = new NativeGeneratedPreviewModel((_, _) => { reads++; var t = new TaskCompletionSource<NativePanelFileContent>(); pending.Add(t); return t.Task; });
        model.Bind(outputs);
        Check(await model.OpenAsync(table.Id) && reads == 0 && model.Selected?.Output?.Table != null, "inline preview performs no file I/O");
        Check(!await model.OpenAsync(outputs.Items[1].Id) && model.Selected?.Id == table.Id, "superseded output cannot replace current selection");
        var firstRead = model.OpenAsync(outputs.Items[0].Id);
        var secondRead = model.OpenAsync(outputs.Items[^1].Id);
        pending[1].SetResult(new("results/a.csv", "text/csv", "new", null, false, null)); await secondRead;
        pending[0].SetResult(new("inputs/a.csv", "text/csv", "old", null, false, null)); await firstRead;
        Check(model.Content?.Text == "new" && !model.Loading, "out-of-order file reads cannot replace the new selection");
        var failed = model.OpenAsync(outputs.Items[0].Id); pending[2].SetException(new IOException("Unavailable")); await failed;
        Check(model.Error == "Unavailable" && reads == 3, "failed reads remain visible without automatic retry");
        var retry = model.RetryAsync(); pending[3].SetResult(new("inputs/a.csv", "text/csv", "retried", null, false, null)); await retry;
        Check(model.Content?.Text == "retried" && model.Error == null, "explicit retry clears the error only for the current output");
        var closed = model.OpenAsync(outputs.Items[0].Id); model.Close(); pending[4].SetResult(new("inputs/a.csv", "text/csv", "late", null, false, null)); await closed;
        Check(model.Selected == null && model.Content == null, "Escape-equivalent close invalidates late reads");
        using var cancellation = new CancellationTokenSource();
        var canceled = model.OpenAsync(outputs.Items[0].Id, cancellation.Token); cancellation.Cancel();
        pending[5].SetResult(new("inputs/a.csv", "text/csv", "canceled", null, false, null)); await canceled;
        Check(model.Selected == null && !model.Loading, "cancellation closes selection rather than leaving a blank preview");
        await model.OpenAsync(table.Id); outputs.Update(page with { SessionId = "another" }, locale); model.Bind(outputs);
        Check(model.Selected == null, "session changes invalidate output identity even when text is identical");
        var recording = new NativeGeneratedPreviewModel(); recording.Bind(outputs); await recording.OpenAsync(outputs.Items[0].Id);
        Check(recording.FileUnavailable && !recording.Loading && recording.Error == null, "recordings explicitly mark absent file bytes");
        outputs.Update(null, locale); recording.Bind(outputs);
        Check(recording.Selected == null && outputs.Items.Count == 0, "empty transcripts clear directories and previews");
    }
}
