using Wisp.ProjectBrowser;

static class NativeTranscriptTests
{
    public static void Run()
    {
        UsageSummary();
        CodeHighlight();
        ToolImagePath();
        Console.WriteLine("Native transcript rendering contract tests passed.");
    }

    private static void UsageSummary()
    {
        // Matches the host usage_item JSON shape and the WebView usage line wording.
        var summary = TranscriptPresentation.UsageSummary("usage",
            """{"input":2817787,"output":3974,"reasoning":0,"cached":2798154,"ctx_tokens":149060,"max_context":1048576}""");
        Expect(summary == "输入 2817.8k · 输出 4.0k tokens · 缓存 2798.2k", summary ?? "null");

        var reasoning = TranscriptPresentation.UsageSummary("usage",
            """{"input":950,"output":30,"reasoning":120,"cached":0,"ctx_tokens":100,"max_context":1000}""");
        Expect(reasoning == "输入 950 · 输出 30 tokens · 思考 120", reasoning ?? "null");

        Expect(TranscriptPresentation.UsageSummary("assistant", "hello") == null, "non-usage role must stay null");
        Expect(TranscriptPresentation.UsageSummary("usage", "not json") == null, "malformed usage must stay null");
        Expect(TranscriptPresentation.FmtTokens(999) == "999", "fmt below 1k");
        Expect(TranscriptPresentation.FmtTokens(1000) == "1.0k", "fmt at 1k");
    }

    private static void CodeHighlight()
    {
        Expect(NativeCodeHighlight.NormalizeLanguage("Python") == "python", "case-insensitive alias");
        Expect(NativeCodeHighlight.NormalizeLanguage("py") == "python", "py alias");
        Expect(NativeCodeHighlight.NormalizeLanguage("Rscript") == "r", "r alias");
        Expect(NativeCodeHighlight.NormalizeLanguage("bash") == "bash", "bash direct");
        Expect(NativeCodeHighlight.NormalizeLanguage(null) == null, "missing info");
        Expect(NativeCodeHighlight.NormalizeLanguage("notalanguage") == null, "unknown stays unsupported");
        Expect(!NativeCodeHighlight.Supported("notalanguage"), "supported flag");

        // Python: keywords, comments, strings; triple-quoted strings stay one token.
        var pythonSource = "import os  # load\nname = \"a#b\"\nfor i in range(3):\n    print(f\"{i}\")\ndoc = \"\"\"x # y\"\"\"\n";
        var python = NativeCodeHighlight.Tokenize(pythonSource, "python");
        Expect(HasToken(pythonSource, python, "import", NativeCodeHighlight.Keyword), "python keyword");
        Expect(HasToken(pythonSource, python, "# load", NativeCodeHighlight.Comment), "python comment");
        Expect(HasToken(pythonSource, python, "\"a#b\"", NativeCodeHighlight.StringToken), "hash inside string stays string");
        Expect(HasToken(pythonSource, python, "for", NativeCodeHighlight.Keyword), "python for");
        Expect(HasToken(pythonSource, python, "\"\"\"x # y\"\"\"", NativeCodeHighlight.StringToken), "triple-quoted string one token");
        Expect(!HasToken(pythonSource, python, "os", NativeCodeHighlight.Keyword), "identifier not keyword");

        // JSON: literals only, no comments.
        var jsonSource = "{\n  \"k\": true, // not a comment\n  \"n\": 1.5e3\n}";
        var json = NativeCodeHighlight.Tokenize(jsonSource, "json");
        Expect(HasToken(jsonSource, json, "true", NativeCodeHighlight.Keyword), "json literal");
        Expect(HasToken(jsonSource, json, "\"k\"", NativeCodeHighlight.StringToken), "json string");
        Expect(HasToken(jsonSource, json, "1.5e3", NativeCodeHighlight.Number), "json number");
        Expect(!HasToken(jsonSource, json, "// not a comment", NativeCodeHighlight.Comment), "json has no comments");

        // R: hash comments and strings.
        var rSource = "x <- 1 # note\nlabel <- \"a\"\n";
        var r = NativeCodeHighlight.Tokenize(rSource, "r");
        Expect(HasToken(rSource, r, "# note", NativeCodeHighlight.Comment), "r comment");
        Expect(HasToken(rSource, r, "\"a\"", NativeCodeHighlight.StringToken), "r string");

        // C-family block comments span lines; unknown languages stay plain.
        var rustSource = "let x = 1; /* block\ncomment */\n";
        var rust = NativeCodeHighlight.Tokenize(rustSource, "rust");
        Expect(HasToken(rustSource, rust, "let", NativeCodeHighlight.Keyword), "rust let");
        Expect(HasToken(rustSource, rust, "/* block\ncomment */", NativeCodeHighlight.Comment), "block comment spans lines");
        Expect(NativeCodeHighlight.Tokenize("SELECT a FROM t", "notalanguage").Count == 0, "unknown language plain");
        Expect(NativeCodeHighlight.Tokenize("", "python").Count == 0, "empty code plain");
    }

    private static bool HasToken(string source, IReadOnlyList<NativeCodeHighlight.Token> tokens, string value, string kind)
    {
        foreach (var token in tokens)
            if (token.Kind == kind && token.Start >= 0 && token.Start + token.Length <= source.Length
                && source.Substring(token.Start, token.Length) == value)
                return true;
        return false;
    }

    private static void ToolImagePath()
    {
        Expect(TranscriptPresentation.ToolImagePath("Image: E:\\x\\y.png (resized for model)") == "E:\\x\\y.png",
            "resized suffix trimmed");
        Expect(TranscriptPresentation.ToolImagePath("Image: /tmp/a.png") == "/tmp/a.png", "plain path");
        Expect(TranscriptPresentation.ToolImagePath("Error: no image") == null, "non-image result");
        Expect(TranscriptPresentation.ToolImagePath("") == null, "empty result");
    }

    private static void Expect(bool condition, string label)
    {
        if (!condition) throw new InvalidOperationException("Transcript contract drift: " + label);
    }
}
