using Markdig;
using Markdig.Extensions.Tables;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using System.Text;

namespace Wisp.ProjectBrowser;

/// <summary>Skill documents are inert Markdown, separate from transcript artifact rendering.</summary>
public static class NativeSkillMarkdown
{
    private static readonly MarkdownPipeline Pipeline = new MarkdownPipelineBuilder()
        .UsePipeTables().UseEmphasisExtras().DisableHtml().Build();
    public static MarkdownDocument Parse(string text)
    {
        var document = Markdown.Parse(text, Pipeline);
        foreach (var table in document.Descendants<Table>()) NativeTranscriptTables.UnescapeCodePipes(table);
        foreach (var link in document.Descendants<LinkInline>().Where(link => link.IsImage))
        { link.IsImage = false; link.Url = ""; }
        return document;
    }
    public static bool HasVisibleContent(Block block) => block switch {
        ThematicBreakBlock => true,
        LeafBlock leaf => leaf.Inline?.Any() == true || !string.IsNullOrWhiteSpace(leaf.Lines.ToString()),
        ContainerBlock container => container.Any(HasVisibleContent),
        _ => false };

    /// <summary>Visible document text for preview-quote validation. Link targets,
    /// Markdown markers and hidden image URLs are not part of the selection.</summary>
    public static string VisibleText(string source)
    {
        var text = new StringBuilder();
        void InlineText(Inline inline)
        {
            switch (inline)
            {
                case LiteralInline literal: text.Append(literal.Content); break;
                case CodeInline code: text.Append(code.Content); break;
                case LineBreakInline line: text.Append(line.IsHard ? '\n' : ' '); break;
                case AutolinkInline link: text.Append(link.Url); break;
                case HtmlInline html: text.Append(html.Tag); break;
                case ContainerInline container: foreach (var child in container) InlineText(child); break;
            }
        }
        void BlockText(Block block, int depth = 0, string marker = "")
        {
            if (block is ListBlock list)
            {
                var number = int.TryParse(list.OrderedStart, out var start) ? start : 1;
                foreach (var item in list.OfType<ListItemBlock>())
                {
                    if (!HasVisibleContent(item)) { number++; continue; }
                    var first = true;
                    foreach (var child in item)
                    {
                        BlockText(child, depth + 1, first ? list.IsOrdered ? $"{number}. " : depth == 0 ? "• " : "◦ " : "");
                        first = false;
                    }
                    number++;
                }
            }
            // The renderer starts a separate prose surface for block quotes;
            // list markers inside it restart at the outermost list style.
            else if (block is QuoteBlock quoted) foreach (var child in quoted) BlockText(child);
            else if (block is CodeBlock code) text.Append(code.Lines.ToString()).Append('\n');
            else if (block is LeafBlock leaf)
            {
                text.Append(marker);
                if (leaf.Inline != null) foreach (var inline in leaf.Inline) InlineText(inline);
                else text.Append(leaf.Lines.ToString());
                text.Append('\n');
            }
            else if (block is ContainerBlock container) foreach (var child in container) BlockText(child, depth, marker);
        }
        foreach (var block in Parse(source)) BlockText(block);
        return text.ToString();
    }
}
