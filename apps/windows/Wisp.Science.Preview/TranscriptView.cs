using Markdig;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using XamlInline = Microsoft.UI.Xaml.Documents.Inline;
using MarkdownBlock = Markdig.Syntax.Block;

namespace Wisp.Science.Preview;

internal static class TranscriptView
{
    public static FrameworkElement Create(BrowserMessage message, WispDesign design)
    {
        var sections = new StackPanel { Spacing = 12 };
        foreach (var section in TranscriptPresentation.Sections(message))
        {
            if (section.ToolName is { } tool)
            {
                var header = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
                header.Children.Add(design.Icon("terminal", 15));
                header.Children.Add(new TextBlock { Text = (section.IsResult ? "工具结果 · " : "工具调用 · ") + tool,
                    FontSize = design.FontSize(12), FontFamily = design.Font(), Foreground = design.Brush("text-muted") });
                var expander = new Expander { Header = header, HorizontalAlignment = HorizontalAlignment.Stretch,
                    HorizontalContentAlignment = HorizontalAlignment.Stretch, Background = design.Brush("bg-sunken"),
                    BorderBrush = design.Brush("border"), Content = Code(section.Text, design), IsExpanded = false };
                sections.Children.Add(expander);
            }
            else
            {
                var rich = new RichTextBlock { FontSize = design.FontSize(14), FontFamily = design.Font(), Foreground = design.Brush("text"),
                    IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap };
                foreach (var block in Markdown.Parse(section.Text)) AppendBlock(rich, block, design);
                sections.Children.Add(rich);
            }
        }
        return sections;
    }

    private static FrameworkElement Code(string value, WispDesign design) => new ScrollViewer
    {
        MaxHeight = 300, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
        Content = new TextBlock { Text = value, FontFamily = design.Font(true), FontSize = design.FontSize(12, true),
            IsTextSelectionEnabled = true, Foreground = design.Brush("text-muted"), Padding = new Thickness(10), TextWrapping = TextWrapping.NoWrap }
    };

    private static void AppendBlock(RichTextBlock rich, MarkdownBlock block, WispDesign design, string prefix = "")
    {
        if (block is CodeBlock code)
        {
            var paragraph = new Paragraph { Margin = new Thickness(0, 6, 0, 12), FontFamily = design.Font(true), FontSize = design.FontSize(12, true) };
            paragraph.Inlines.Add(new Run { Text = code.Lines.ToString() });
            rich.Blocks.Add(paragraph);
        }
        else if (block is LeafBlock leaf)
        {
            var paragraph = new Paragraph { Margin = new Thickness(0, 0, 0, 12) };
            if (block is HeadingBlock heading) { paragraph.FontSize = design.FontSize(heading.Level <= 2 ? 18 : 15); paragraph.FontWeight = FontWeights.SemiBold; }
            if (prefix.Length > 0) paragraph.Inlines.Add(new Run { Text = prefix });
            if (leaf.Inline != null) foreach (var inline in leaf.Inline) paragraph.Inlines.Add(RenderInline(inline, design));
            else paragraph.Inlines.Add(new Run { Text = leaf.Lines.ToString() });
            rich.Blocks.Add(paragraph);
        }
        else if (block is ListBlock list)
        {
            var index = int.TryParse(list.OrderedStart, out var start) ? start : 1;
            foreach (var item in list.OfType<ListItemBlock>())
            {
                var first = true;
                foreach (var child in item)
                {
                    AppendBlock(rich, child, design, first ? (list.IsOrdered ? $"{index}. " : "• ") : ""); first = false;
                }
                index++;
            }
        }
        else if (block is ContainerBlock container)
            foreach (var child in container) AppendBlock(rich, child, design, block is QuoteBlock ? "│ " : prefix);
    }

    private static XamlInline RenderInline(Markdig.Syntax.Inlines.Inline inline, WispDesign design)
    {
        switch (inline)
        {
            case LiteralInline literal: return new Run { Text = literal.Content.ToString() };
            case CodeInline code: return new Run { Text = code.Content, FontFamily = design.Font(true), FontSize = design.FontSize(12, true), Foreground = design.Brush("clay-strong") };
            case LineBreakInline: return new LineBreak();
            case HtmlInline html: return new Run { Text = html.Tag }; // Display HTML as inert text; never embed web content.
            case AutolinkInline link: return new Run { Text = link.Url, Foreground = design.Brush("clay-strong") };
            case ContainerInline container:
                Span span = new();
                if (inline is EmphasisInline emphasis)
                {
                    if (emphasis.DelimiterCount >= 2) span.FontWeight = FontWeights.SemiBold;
                    else span.FontStyle = Windows.UI.Text.FontStyle.Italic;
                }
                if (inline is LinkInline target)
                {
                    if (!target.IsImage && Uri.TryCreate(target.Url, UriKind.Absolute, out var uri) && uri.Scheme is "https" or "http")
                        span = new Hyperlink { NavigateUri = uri };
                    else if (target.IsImage) span.Inlines.Add(new Run { Text = "[图片] " });
                    span.Foreground = design.Brush("clay-strong");
                }
                foreach (var child in container) span.Inlines.Add(RenderInline(child, design));
                return span;
            default: return new Run { Text = inline.ToString() ?? "" };
        }
    }
}
