using Markdig.Syntax;
using Markdig.Syntax.Inlines;

namespace Wisp.ProjectBrowser;

public static class NativeResponseMarkdown
{
    public static string CodeText(string value) => value.Replace("\r\n", "\n").Replace('\r', '\n');
    // CJK text can break between glyphs; Latin identifiers keep a word-sized
    // minimum. This prevents short headers becoming a column of single letters.
    public static IEnumerable<string> MinimumWidthWords(string value)
        => System.Text.RegularExpressions.Regex.Matches(value,
            @"[\u2e80-\u9fff\uac00-\ud7af\uf900-\ufaff]|[^\s\u2e80-\u9fff\uac00-\ud7af\uf900-\ufaff]+")
            .Select(match => match.Value);
    // WebView marks only paragraphs whose first non-whitespace HTML is <strong>.
    // A bold phrase after punctuation, a link or ordinary text is not a lead.
    public static bool HasStrongLead(ParagraphBlock paragraph)
        => paragraph.Inline?.FirstOrDefault(inline => inline is not LiteralInline literal
            || !string.IsNullOrWhiteSpace(literal.Content.ToString()))
            is EmphasisInline { DelimiterCount: 2, DelimiterChar: '*' or '_' };
}
