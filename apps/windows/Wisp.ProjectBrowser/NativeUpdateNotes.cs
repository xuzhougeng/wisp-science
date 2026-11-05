using Markdig;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using System.Text;

namespace Wisp.ProjectBrowser;

/// Release metadata comments are hidden by the WebView Markdown renderer.
public static class NativeUpdateNotes
{
    // Inline offsets must refer to the original Markdown, including earlier blocks.
    private static readonly MarkdownPipeline Pipeline = new MarkdownPipelineBuilder().UsePreciseSourceLocation().Build();

    public static string ForDisplay(string markdown)
    {
        var document = Markdown.Parse(markdown, Pipeline);
        var comments = new List<(int Start, int Length)>();
        foreach (var block in document.Descendants<HtmlBlock>())
        {
            var start = block.Span.Start;
            while (start <= block.Span.End && char.IsWhiteSpace(markdown[start])) start++;
            if (!markdown.AsSpan(start).StartsWith("<!--", StringComparison.Ordinal)) continue;
            var end = markdown.IndexOf("-->", start + 4, StringComparison.Ordinal);
            comments.Add((start, (end < 0 ? block.Span.End + 1 : end + 3) - start));
        }
        foreach (var inline in document.Descendants<HtmlInline>())
            if (inline.Tag.StartsWith("<!--", StringComparison.Ordinal))
                comments.Add((inline.Span.Start, inline.Span.Length));
        var visible = new StringBuilder(markdown);
        foreach (var (start, length) in comments.OrderByDescending(comment => comment.Start)) visible.Remove(start, length);
        return visible.ToString();
    }
}
