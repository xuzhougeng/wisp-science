using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeComposerReadingTests
{
    public static void Run()
    {
        static ConversationItem Item(string role, string text) => new(role, text, null, null, null, null);
        static void Check(bool pass, string message) { if (!pass) throw new InvalidOperationException(message); }
        var usage = Item("usage", """{"input":9999999,"ctx_tokens":500,"max_context":1000,"context_usage":{"system_prompt":100,"conversation":400}}""");
        var total = NativeContextUsage.Latest([usage])!;
        Check(total.Used == 500 && total.Label.Contains("50%") && total.Rows.Count == 7 && total.Rows.Count(row => row.Tokens > 0) == 2, "context uses the active window and includes empty categories, not cumulative billing");
        Check(total.CompactLabel == "50%" && new NativeContextUsage(10, 0, []).CompactLabel == "—",
            "toolbar usage stays compact while unknown limits never claim zero usage");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> { ["en"] = new()
        { ["context_usage.system_prompt"] = "System prompt", ["context_usage.conversation"] = "Conversation",
          ["context_usage.open_pct"] = "Open context usage, {pct}%", ["context_usage.open_unknown"] = "Context window unknown",
          ["context_usage.total_estimated"] = "~{used} / {max} Tokens", ["context_usage.total_used"] = "{used} Tokens" } }, "en");
        var localized = NativeContextUsage.Latest([usage], locale)!;
        Check(localized.Rows.Where(row => row.Tokens > 0).Select(row => row.Label).SequenceEqual(new[] { "System prompt", "Conversation" })
            && localized.LocalizedLabel(locale) == "Open context usage, 50%", "usage popover labels follow the current shared locale");
        Check(new NativeContextUsage(10, 0, []).LocalizedTotal(locale) == "10 Tokens"
            && new NativeContextUsage(10, 0, []).LocalizedLabel(locale) == "Context window unknown", "unknown window never invents a localized percentage or capacity");
        var compacted = Item("compaction", """{"after":100,"strategy":"manual","undone":false}""");
        var afterCompact = NativeContextUsage.Latest([usage, compacted])!;
        Check(afterCompact.Used == 100 && afterCompact.Rows.Count == 7 && afterCompact.Rows.Take(6).All(row => row.Tokens == 0)
            && afterCompact.Rows[^1].Tokens == 100, "compaction discards stale buckets without hiding empty category rows");
        Check(new NativeContextUsage(695, 1000, []).Percent == 70 && new NativeContextUsage(905, 1000, []).Tone == "err"
            && new NativeContextUsage(900, 1000, []).Tone == "warn" && new NativeContextUsage(694, 1000, []).Tone == "text-muted",
            "WebView half-up percentage and 70/90 warning bands are respected");
        Check(new NativeContextUsage(ulong.MaxValue, 1, []).Percent == 100 && new NativeContextUsage(ulong.MaxValue, ulong.MaxValue, []).Percent == 100,
            "percentage arithmetic cannot overflow or exceed the visible window");
        Check(NativeContextUsage.FormatTokens(1200) == "1.2K" && NativeContextUsage.FormatTokens(1500000) == "1.5M"
            && NativeContextUsage.FormatLimit(128000) == "128K" && NativeContextUsage.FormatLimit(2000000) == "2M", "shared compact token formatting");
        Check(total.Rows.Select(row => row.Color).Distinct().Count() == 7 && total.Rows.Count(row => row.Expandable) == 6
            && total.SegmentShare(total.Rows[0]) == .1, "distinct category colors and the full-capacity denominator match the usage panel");
        Check(NativeContextUsage.Latest([usage, compacted with { Text = """{"after":100,"strategy":"auto_continue"}""" }])?.Used == 500,
            "auto-continue does not compact the context window");
        Check(NativeContextUsage.Latest([usage, compacted with { Text = """{"after":100,"undone":true}""" }])?.Used == 500, "undone compaction is ignored");
        Check(NativeContextUsage.Latest([usage, compacted, usage])?.Used == 500, "fresh usage supersedes prior compaction");
        Check(NativeContextUsage.Latest([usage, Item("usage", "[broken"), Item("usage", "{\"ctx_tokens\":\"bad\"}")])?.Used == 500,
            "malformed optional metadata does not hide valid earlier usage");
        Check(NativeContextUsage.Latest([Item("usage", "{\"ctx_tokens\":10}")])?.Label.Contains("未知") == true, "unknown limits never imply zero percent");
        var page = new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", 1, "p", "s", [Item("user", "question"), Item("assistant", "answer")],
            null, false, false, false, "m", null, null, [], UserOffset: 8);
        var quote = NativeComposerQuote.From(page, 1, "line one\r\nline two")!;
        Check(quote.UserIndex == 8 && quote.Source.Contains("第 9 轮") && quote.SessionId == "s", "history quotes carry absolute turn identity");
        Check(NativeComposerQuote.Compose("draft", [quote]).EndsWith("> line one\n> line two\n\ndraft"), "all quote lines stay visibly quoted before the draft");
        Check(NativeComposerQuote.From(page, -1, "bad") == null && NativeComposerQuote.From(page, 2, "bad") == null,
            "out-of-page selections cannot fabricate a turn source");
        foreach (var invalid in new[] { "[]", "{}", "{\"text\":null,\"page\":1,\"jump\":false}",
            "{\"text\":\"text\",\"page\":\"1\",\"jump\":false}", "{\"text\":\"text\",\"page\":0,\"jump\":false}",
            "{\"text\":\"text\",\"page\":1.5,\"jump\":false}", "{\"text\":\"text\",\"page\":1,\"jump\":\"yes\"}" })
        {
            using var json = System.Text.Json.JsonDocument.Parse(invalid);
            Check(NativeDocumentSelection.Parse(json.RootElement) == null, "invalid PDF selection is rejected without throwing");
        }
        using var selectionJson = System.Text.Json.JsonDocument.Parse("{\"text\":\"line one\\nline two\",\"page\":2,\"jump\":true,\"path\":\"untrusted.pdf\"}");
        Check(NativeDocumentSelection.Parse(selectionJson.RootElement) is { Page: 2, Jump: true, Text: "line one\nline two" },
            "PDF message accepts only selection data; file identity comes from the mounted native panel");
        Check(!new NativeDocumentSelection(new string('x', 32769), 1, false).Valid, "PDF quote size is bounded");
        foreach (var (kind, position, expected) in new[] {
            ("docx", "\"page\":2,\"endPage\":3", "预览第 2–3 页"),
            ("pptx", "\"page\":4,\"endPage\":4", "第 4 张幻灯片"),
            ("xlsx", "\"sheet\":\"中文表\",\"cells\":\"B2:C3\"", "工作表 中文表 · B2:C3") }) {
            using var json = System.Text.Json.JsonDocument.Parse("{\"text\":\"value\",\"jump\":false,\"location\":{\"kind\":\"" + kind + "\"," + position + "},\"path\":\"forged.docx\"}");
            var selection = NativeDocumentSelection.Parse(json.RootElement, kind);
            Check(selection?.Location.Label == expected, "Office selection carries its actual page/sheet position");
            Check(NativeDocumentSelection.Parse(json.RootElement, "pdf") == null, "renderer cannot substitute a different document kind");
        }
        foreach (var location in new[] {
            "null", "{}", "{\"kind\":\"xlsx\",\"sheet\":\"name\\nforged\",\"cells\":\"A1\"}",
            "{\"kind\":\"xlsx\",\"sheet\":\"Sheet1\",\"cells\":\"A0\"}",
            "{\"kind\":\"xlsx\",\"sheet\":\"Sheet1\",\"cells\":\"A1\",\"page\":1}" }) {
            using var json = System.Text.Json.JsonDocument.Parse("{\"text\":\"x\",\"jump\":false,\"location\":" + location + "}");
            Check(NativeDocumentSelection.Parse(json.RootElement, "xlsx") == null, "invalid Office locator is rejected");
        }
        var sourceText = ">seq\r\nACGT\rTT😀\nend";
        var sourceSelection = NativeDocumentSelection.FromSource(sourceText, 6, 10, true, true)!;
        Check(sourceSelection.Text == "ACGT\rTT😀\n" && sourceSelection.Location is { Line: 2, EndLine: 3, Unsaved: true },
            "source quote covers exact selected text with CRLF/CR line positions and unsaved marker");
        Check(NativeDocumentSelection.FromSource(sourceText, 14, 1, false) == null
            && NativeDocumentSelection.FromSource(sourceText, -1, 2, false) == null,
            "source quote rejects surrogate splits and invalid offsets");
        var sourceQuote = new NativeComposerQuote("s", -1, "document", sourceSelection.Text, new("p", "aligned.fa", sourceSelection.Location));
        Check(sourceQuote.Message.Contains("未保存草稿") && sourceQuote.Message.Contains("> ACGT\n> TT😀\n> "),
            "source quotes label unsaved content and quote each Windows newline");
        var quoteLocale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> {
            ["en"] = new() { ["attachment.quote"] = "Quoted text" }, ["zh"] = new() { ["attachment.quote"] = "引用文本" } }, "en");
        var message = sourceQuote.Message;
        foreach (var language in new[] { "en", "zh" })
        {
            quoteLocale.Preview(language);
            Check(sourceQuote.SourceLabel(quoteLocale) == "aligned.fa" && sourceQuote.Tooltip() == "aligned.fa\n\n" + sourceText.Substring(6, 10),
                "file quote captions and tooltips match WebView's literal source path");
            Check(sourceQuote.Message == message && sourceQuote.Document?.Location.Unsaved == true,
                "display language does not rewrite quote provenance or outgoing content");
            Check(quote.SourceLabel(quoteLocale) == (language == "en" ? "Quoted text" : "引用文本") && quote.Tooltip() == quote.Text,
                "ordinary transcript selection uses the shared localized quote caption");
        }
        var registeredQuote = sourceQuote with { Document = sourceQuote.Document! with { Path = "artifact:snapshot" } };
        Check(registeredQuote.SourceLabel(quoteLocale) == "artifact:snapshot", "reference captions never replace immutable artifact identities");
        var excerptQuote = new NativeComposerQuote("s", 3, "excerpt", "Evidence", ExcerptLabel: "Study {title} · turns 4–5");
        Check(excerptQuote.SourceLabel(quoteLocale) == "Study {title} · turns 4–5" && excerptQuote.Tooltip() == "Study {title} · turns 4–5\n\nEvidence",
            "excerpt captions preserve the selected title without interpreting user placeholders");
        Console.WriteLine("Native reading: context windows, compaction, malformed rows and source-aware quotes passed.");
    }
}
