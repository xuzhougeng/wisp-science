using Markdig.Syntax;
using Markdig.Syntax.Inlines;

namespace Wisp.ProjectBrowser;

public static class NativeResponseMarkdown
{
    // This renderer has no verified file opener. Local Markdown links should
    // look like inert code, not like the external links that actually navigate.
    public static string? LocalLinkCode(LinkInline link)
    {
        if (link.IsImage || string.IsNullOrWhiteSpace(link.Url) || link.Url.StartsWith('#')) return null;
        if (Uri.TryCreate(link.Url, UriKind.Absolute, out var uri) && !uri.IsFile) return null;
        string Text(Inline inline) => inline switch
        {
            LiteralInline literal => literal.Content.ToString(),
            CodeInline code => code.Content,
            LineBreakInline => " ",
            ContainerInline container => string.Concat(container.Select(Text)),
            _ => ""
        };
        return Text(link);
    }

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
