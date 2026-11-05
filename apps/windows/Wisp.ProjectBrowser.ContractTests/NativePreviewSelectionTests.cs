using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativePreviewSelectionTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static void Run()
    {
        const string markdown = "# 中文 😀\n\nA **bold** result with `alpha = 42` and [evidence](https://example.invalid/private).\n\n> Quoted finding\n\n- First\n- Second\n\n| Item | Value |\n| --- | --- |\n| sample | 42 |\n\n```python\nprint(alpha)\n```\n";
        var content = new NativePanelFileContent("notes.md", "text/markdown", markdown, null, false, null);
        var text = NativeArtifactSelection.PreviewText(content, "notes.md", "markdown")!;
        foreach (var excerpt in new[] { "中文 😀", "A bold result with alpha = 42 and evidence.", "Quoted finding", "• First\r\n• Second", "sample", "print(alpha)" })
            Check(NativeArtifactSelection.Allows(content, "notes.md", "markdown", NativeDocumentSelection.FromPreview(excerpt)!),
                "visible Markdown prose, inline code, lists, table cells and fences can be quoted");
        Check(!text.Contains("https://example.invalid/private")
            && !NativeArtifactSelection.Allows(content, "notes.md", "markdown", NativeDocumentSelection.FromPreview("fabricated result")!),
            "hidden link targets and fabricated text cannot enter a preview quote");
        var nested = content with { Text = "- Outer\n\n  > - Quoted list\n  > - Next finding\n\n- Last" };
        Check(NativeArtifactSelection.Allows(nested, "notes.md", "markdown",
            NativeDocumentSelection.FromPreview("• Quoted list\n• Next finding")!),
            "lists inside block quotes match their independently rendered marker style");
        var json = content with { Path = "config.json", Mime = "application/json", Text = "{\"z\":42,\"a\":1}" };
        var pretty = NativeArtifactSelection.PreviewText(json, "config.json", "json")!;
        Check(pretty.IndexOf("\"a\"", StringComparison.Ordinal) < pretty.IndexOf("\"z\"", StringComparison.Ordinal)
            && NativeArtifactSelection.Allows(json, "config.json", "json", NativeDocumentSelection.FromPreview("\"a\": 1,\n  \"z\": 42")!),
            "formatted and reordered JSON selections quote displayed text");
        var quote = NativeDocumentSelection.FromPreview("\"z\": 42")!;
        Check(quote.Location is { Kind: "preview", Line: null, Page: null } && quote.Location.Label == "预览选区"
            && !NativeArtifactSelection.Allows(json, "config.json", "json", quote with { Location = new("source", Line: 3, EndLine: 3) }),
            "formatted preview text never fabricates an original-file line");
        Check(!(quote with { Location = quote.Location with { Line = 3 } }).Valid
            && !(quote with { Location = quote.Location with { Unsaved = true } }).Valid
            && NativeDocumentSelection.FromPreview(" ") == null
            && NativeDocumentSelection.FromPreview("before\uFFFCafter") == null
            && NativeDocumentSelection.FromPreview(new string('x', 32769)) == null,
            "preview positions, object placeholders, whitespace and size bounds are validated");
        var clipped = content with { Path = "large.txt", Mime = "text/plain", Text = string.Concat(Enumerable.Repeat("visible\n", 8000)) + "not displayed", Truncated = true };
        Check(NativeArtifactSelection.Allows(clipped, "large.txt", "text", NativeDocumentSelection.FromPreview("visible")!)
            && !NativeArtifactSelection.Allows(clipped, "large.txt", "text", NativeDocumentSelection.FromPreview("not displayed")!),
            "a truncated preview may quote its visible head but not its undisplayed tail");
        NativeRenderedTextSpan[] spans = [new(2, 4, "A "), new(6, 7, "alpha = 42", true), new(9, 17, " result."),
            new(18, 22, "\n", true), new(24, 28, "Next")];
        Check(NativeRenderedTextSelection.Capture(spans, 2, 28) == "A alpha = 42 result.\nNext"
            && NativeRenderedTextSelection.Capture(spans, 6, 7) == "alpha = 42"
            && NativeRenderedTextSelection.Capture(spans, 10, 16) == "result",
            "rendered selections expand inline-code containers, keep paragraph boundaries and clip ordinary runs");
        Check(NativeRenderedTextSelection.Capture([new(2, 3, "wrong length")], 2, 3) == "",
            "invalid render offsets fail closed");
        var longText = new string('x', 32769);
        Check(NativeRenderedTextSelection.Capture([new(0, longText.Length, longText)], 0, longText.Length) == ""
            && NativeRenderedTextSelection.Capture([new(0, longText.Length, longText)], 0, longText.Length, int.MaxValue) == longText,
            "copy can preserve a long rendered selection while quoting remains bounded");
        var prompt = new NativeComposerQuote("s", -1, "document", quote.Text, new("p", "artifact:json", quote.Location));
        Check(prompt.Source.Contains("artifact:json") && prompt.Source.Contains("预览选区") && !prompt.Source.Contains("第 3"),
            "composer provenance labels the preview source without guessing file positions");
    }
}
