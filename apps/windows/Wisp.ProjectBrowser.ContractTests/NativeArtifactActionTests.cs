using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeArtifactActionTests
{
    public static async Task RunAsync()
    {
        var editor = new NativeArtifactEditor(); editor.Toggle("original"); editor.Draft = "print(42)\n\n";
        Check(editor.Message("a.png", "python", "en") == "I edited the code that produced a.png. Please re-run this version and regenerate the artifact:\n```python\nprint(42)\n```", "rerun is the WebView draft text");
        Check(editor.Message("{name}.png", "python", "zh")!.StartsWith("我修改了生成 {name}.png 的代码"), "filenames are literal data");
        editor.Draft = "  \n"; Check(editor.Message("a", "python", "en") == null, "empty edit cannot be inserted");
        editor.Cancel(); Check(!editor.Editing && editor.Message("a", "python", "en") == null, "cancel leaves edit mode");
        editor.Toggle("restored"); Check(editor.Draft == "restored", "reopening editing resets to provenance");
        editor.Reset(); Check(!editor.Editing && editor.Draft.Length == 0, "navigation resets editor");

        var host = new Host(); var client = new NativePanelClient(host);
        var artifacts = new[] { new NativePanelArtifact("a", "a.png", "image/png", "figures/a.png", null, 0, null),
            new NativePanelArtifact("b", "b.csv", "text/csv", "data/b.csv", null, 0, null) };
        WorkspaceArtifactModel Reader(string scope = "p") => new(client, scope, "s", artifacts);
        var reader = Reader(); await reader.OpenAsync("a"); var copied = 0;
        Task Copy(NativePanelExport source, string path, CancellationToken token)
        { Check(source.Path == host.Source && path == host.Destination, "copy uses original source and chosen destination"); copied++; return Task.CompletedTask; }
        await reader.ExportAsync(_ => Task.FromResult<string?>(null), Copy);
        Check(copied == 0 && reader.ExportError == null && !reader.ExportBusy, "picker cancellation never copies");
        await reader.ExportAsync(_ => Task.FromResult<string?>(host.Destination), Copy);
        Check(copied == 1 && reader.ExportedPath == host.Destination, "explicit save copies original once");
        await reader.ExportAsync(_ => Task.FromResult<string?>(host.Source), Copy);
        Check(copied == 1 && reader.ExportError != null, "same file is never overwritten");
        var chooser = new TaskCompletionSource<string?>();
        var save = reader.ExportAsync(_ => chooser.Task, Copy);
        await reader.ExportAsync(_ => Task.FromResult<string?>(host.Destination), Copy);
        Check(copied == 1 && reader.ExportBusy, "duplicate click is ignored while picker open");
        reader.Close(); chooser.SetResult(host.Destination); await save;
        Check(copied == 1 && reader.ExportedPath == null, "closing invalidates a pending picker");
        await reader.OpenAsync("a"); host.FailExport = true;
        await reader.ExportAsync(_ => Task.FromResult<string?>(host.Destination), Copy);
        Check(reader.Content != null && reader.ExportError != null && !reader.ExportBusy, "export error preserves preview");
        host.FailExport = false;
        using var tabs = new WorkspaceArtifactTabs();
        await tabs.OpenAsync(Reader(), "a"); var key = tabs.SelectedKey!;
        await tabs.OpenAsync(Reader(), "b"); await tabs.OpenAsync(Reader(), "a");
        Check(tabs.Items.Count == 2 && tabs.SelectedKey == key, "center tabs deduplicate and reselect by scope and path");
        tabs.Select(null); Check(tabs.Selected == null && tabs.Items.Count == 2, "conversation selection keeps files open");
        await tabs.OpenAsync(Reader("other"), "a"); Check(tabs.Items.Count == 3, "same path in another scope stays separate");
        tabs.Close(tabs.SelectedKey!); Check(tabs.Selected == null && tabs.Items.Count == 2, "closing selected file returns to chat");
        tabs.Close(key); Check(tabs.Items.Count == 1, "close only requested tab");
        host.Hold = new(); var pendingReader = Reader(); var pending = tabs.OpenAsync(pendingReader, "a");
        tabs.Close(tabs.SelectedKey!); host.Hold.SetResult(); await pending;
        Check(pendingReader.Content == null && tabs.Items.Count == 1, "late read cannot reopen a closed center tab");
        Check(host.Calls.All(c => c.StartsWith("native_conversation_panel_")), "no model send or execution request");
    }
    private static void Check(bool ok, string message) { if (!ok) throw new InvalidOperationException(message); }
    private sealed class Host : INativeSettingsClient
    {
        public string Source = Path.Combine(Path.GetTempPath(), "artifact-original.png");
        public string Destination = Path.Combine(Path.GetTempPath(), "artifact-copy.png");
        public bool FailExport; public TaskCompletionSource? Hold; public List<string> Calls = [];
        public async Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add(command); var id = args["artifact_id"]!.GetValue<string>();
            Check(args["session_id"]!.GetValue<string>() == "s", "explicit session scope");
            if (command.EndsWith("export"))
            { if (FailExport) throw new IOException("export failed"); return JsonSerializer.SerializeToNode(new NativePanelExport(Source, "original.png", 200), ConversationSnapshot.JsonOptions); }
            if (Hold is { } hold) await hold.Task;
            if (command.EndsWith("readartifact")) return JsonSerializer.SerializeToNode(new NativePanelFileContent(id, "text/plain", "preview", null, false, 7), ConversationSnapshot.JsonOptions);
            return JsonSerializer.SerializeToNode(new NativeArtifactProvenanceReply("wisp.native-conversations.v1", projectId!, "s", id, id, null), ConversationSnapshot.JsonOptions);
        }
    }
}
