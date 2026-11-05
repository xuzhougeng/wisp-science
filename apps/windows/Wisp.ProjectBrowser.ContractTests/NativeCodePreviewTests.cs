using System.Text.Json;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeCodePreviewTests
{
    public static void Run()
    {
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        var source = "\r\n# 样本🙂\r\n\tvalue = 'α'\r\n\r\n";
        var code = NativeCodePreview.Create(source, "python");
        Check(code.Text == "\n# 样本🙂\n\tvalue = 'α'\n\n" && code.Gutter == "1\n2\n3\n4\n5" && code.LineCount == 5,
            "CRLF source keeps indentation, Unicode and both trailing blank lines aligned with its gutter");
        Check(source.Contains("\r\n") && code.Language == "python", "display normalization leaves the original copy/export source intact");
        Check(NativeCodePreview.Create("", null) is { Text: "", Gutter: "1", LineCount: 1, Language: null }, "empty files retain the first source line");
        Check(NativeCodePreview.Create("a\rb", "unknown") is { Text: "a\nb", Gutter: "1\n2", Language: null }, "legacy line endings remain aligned without guessing a grammar");
        Check(NativeCodePreview.FileLanguage("artifact-version:opaque", "analysis.R", "code") == "r"
            && NativeCodePreview.FileLanguage(".wisp/store/blob", "analysis.py", "text/plain") == "python"
            && NativeCodePreview.FileLanguage("script.ts", "analysis.py", "code") == "typescript"
            && NativeCodePreview.FileLanguage("opaque", "opaque", "json") == "json", "path, display name and explicit JSON kind retain WebView language precedence");
        Check(NativeCodePreview.FileLanguage("unknown.log", "unknown.log", "text") == null
            && NativeCodePreview.FileLanguage("py", "py", "text") == null, "plain text and extensionless files do not acquire a language");
        var catalog = NativeArtifactCatalog.Collect([], [new("source", "analysis.py", "text/x-python", ".wisp/store/opaque", null, 0, "scripts/analysis.py")],
            "C:/project", new NativeLocalization(new Dictionary<string, Dictionary<string, string>> { ["en"] = new() }, "en"));
        Check(catalog.Single().Kind == "code" && catalog.Single().Registered?.Id == "source"
            && NativeGeneratedArtifacts.FileKind("scripts/analysis.py") == null,
            "registered source files receive the code badge without auto-promoting scripts from transcript output");
        var compact = "{\"样本\":\"α🙂\",\"count\":9007199254740993,\"values\":[1,2]}";
        var pretty = NativeCodePreview.FormatFile(compact, "json");
        using var parsed = JsonDocument.Parse(pretty);
        Check(pretty.Contains('\n') && pretty.Contains("样本") && parsed.RootElement.GetProperty("count").GetInt64() == 9007199254740993,
            "JSON becomes readable without corrupting Unicode or large integer values");
        var duplicates = NativeCodePreview.FormatFile("{\"z\":0,\"a\":{\"b\":1,\"a\":2},\"z\":3}", "json");
        using var sorted = JsonDocument.Parse(duplicates);
        Check(sorted.RootElement.EnumerateObject().Select(p => p.Name).SequenceEqual(new[] { "a", "z" })
            && sorted.RootElement.GetProperty("a").EnumerateObject().First().Name == "a"
            && sorted.RootElement.GetProperty("z").GetInt32() == 3, "JSON ordering and duplicate keys match the canonical map formatter");
        using var unicodeKeys = JsonDocument.Parse(NativeCodePreview.FormatFile("{\"\U00010000\":1,\"\uE000\":2}", "json"));
        Check(unicodeKeys.RootElement.EnumerateObject().First().Name == "\uE000", "JSON key ordering follows Unicode scalars rather than UTF-16 surrogate order");
        var incomplete = "{\"count\":\n";
        Check(NativeCodePreview.FormatFile(incomplete, "json") == incomplete
            && NativeCodePreview.FormatFile(compact, "python") == compact, "invalid JSON and non-JSON source stay unchanged");
        var content = new NativePanelFileContent("report.json", "application/json", pretty, null, true, 9999);
        var head = NativeArtifactTextPreview.Create(content);
        Check(head.Truncated && head.Text == pretty && head.TotalBytes == 9999, "formatted code retains independent truncation metadata");
        var tokens = NativeCodeHighlight.Tokenize(code.Text, code.Language);
        Check(tokens.Any(token => code.Text.Substring(token.Start, token.Length) == "# 样本🙂")
            && tokens.Any(token => code.Text.Substring(token.Start, token.Length) == "'α'"), "highlight spans use the same normalized source as the gutter");
    }
}
