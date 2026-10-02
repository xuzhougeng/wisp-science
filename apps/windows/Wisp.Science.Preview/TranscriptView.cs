using Markdig;
using Markdig.Extensions.TaskLists;
using Markdig.Extensions.Tables;
using Markdig.Extensions.Mathematics;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using Windows.ApplicationModel.DataTransfer;
using XamlInline = Microsoft.UI.Xaml.Documents.Inline;
using MarkdownBlock = Markdig.Syntax.Block;

namespace Wisp.Science.Preview;

internal static class TranscriptView
{
    private static readonly MarkdownPipeline Pipeline = new MarkdownPipelineBuilder()
        .UsePipeTables().UseTaskLists().UseAutoLinks().UseMathematics().Build();

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
            else sections.Children.Add(RenderMarkdown(section.Text, design));
        }
        return sections;
    }

    /// <summary>Markdown to app-owned controls. Flow blocks share one selectable
    /// RichTextBlock; tables, code fences and images break out as elements.</summary>
    public static FrameworkElement RenderMarkdown(string text, WispDesign design)
    {
        var panel = new StackPanel { Spacing = 0 };
        RichTextBlock? rich = null;
        foreach (var block in Markdown.Parse(text, Pipeline))
        {
            switch (block)
            {
                case MathBlock math:
                    Flush();
                    panel.Children.Add(new NativeRichPreview(design, "math", math.Lines.ToString(), "公式"));
                    break;
                case FencedCodeBlock fenced:
                    Flush();
                    panel.Children.Add(CodeElement(fenced.Lines.ToString(), fenced.Info, design));
                    break;
                case CodeBlock code:
                    Flush();
                    panel.Children.Add(CodeElement(code.Lines.ToString(), null, design));
                    break;
                case Table table:
                    Flush();
                    panel.Children.Add(TableElement(table, design));
                    break;
                default:
                    rich ??= NewRich(design);
                    AppendBlock(rich, block, design);
                    break;
            }
        }
        Flush();
        return panel;

        void Flush() { if (rich != null) { panel.Children.Add(rich); rich = null; } }
    }

    private static RichTextBlock NewRich(WispDesign design) => new()
    {
        FontSize = design.FontSize(14), FontFamily = design.Font(), Foreground = design.Brush("text"),
        IsTextSelectionEnabled = true, TextWrapping = TextWrapping.Wrap
    };

    private static FrameworkElement Code(string value, WispDesign design) => new ScrollViewer
    {
        MaxHeight = 300, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
        Content = new TextBlock { Text = value, FontFamily = design.Font(true), FontSize = design.FontSize(12, true),
            IsTextSelectionEnabled = true, Foreground = design.Brush("text-muted"), Padding = new Thickness(10), TextWrapping = TextWrapping.NoWrap }
    };

    /// <summary>Read-only code block with a language label and a copy action.
    /// Highlight colors come from the shared palette; unknown languages stay plain.</summary>
    private static FrameworkElement CodeElement(string value, string? info, WispDesign design)
    {
        var language = NativeCodeHighlight.NormalizeLanguage(info);
        var body = new TextBlock
        {
            FontFamily = design.Font(true), FontSize = design.FontSize(12, true),
            IsTextSelectionEnabled = true, TextWrapping = TextWrapping.NoWrap,
            Foreground = design.Brush("text"), Padding = new Thickness(10, 8, 10, 10)
        };
        ApplyHighlight(body.Inlines, value, language, design);
        var copy = new Button { Content = "复制", FontSize = design.FontSize(11), Padding = new Thickness(8, 2, 8, 2) };
        copy.Click += (_, _) =>
        {
            try { var package = new DataPackage { RequestedOperation = DataPackageOperation.Copy }; package.SetText(value); Clipboard.SetContent(package); }
            catch { } // Clipboard may be locked by another process; selection still works.
        };
        var header = new Grid { Padding = new Thickness(10, 6, 6, 0) };
        header.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        header.Children.Add(new TextBlock { Text = language ?? info?.Trim() ?? "", FontSize = design.FontSize(11),
            FontFamily = design.Font(), Foreground = design.Brush("text-faint"), VerticalAlignment = VerticalAlignment.Center });
        Grid.SetColumn(copy, 1); header.Children.Add(copy);
        var stack = new StackPanel();
        if (value.Length > 0) stack.Children.Add(header);
        stack.Children.Add(new ScrollViewer { MaxHeight = 340, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, Content = body });
        return new Border
        {
            Background = design.Brush("bg-sunken"), CornerRadius = new CornerRadius(8),
            Margin = new Thickness(0, 6, 0, 12), Child = stack
        };
    }

    private static void ApplyHighlight(InlineCollection inlines, string value, string? language, WispDesign design)
    {
        XamlInline Plain(int start, int length) => new Run { Text = value[start..(start + length)] };
        if (language is null)
        {
            if (value.Length > 0) inlines.Add(Plain(0, value.Length));
            return;
        }
        var position = 0;
        foreach (var token in NativeCodeHighlight.Tokenize(value, language))
        {
            if (token.Start > position) inlines.Add(Plain(position, token.Start - position));
            inlines.Add(new Run
            {
                Text = value.Substring(token.Start, token.Length),
                Foreground = design.Brush(token.Kind switch
                {
                    NativeCodeHighlight.Comment => "text-faint",
                    NativeCodeHighlight.StringToken => "clay",
                    NativeCodeHighlight.Number => "traj-tool-bar",
                    _ => "traj-model-bar",
                })
            });
            position = token.Start + token.Length;
        }
        if (position < value.Length) inlines.Add(Plain(position, value.Length - position));
    }

    private static FrameworkElement TableElement(Table table, WispDesign design)
    {
        var rows = table.OfType<TableRow>().ToArray();
        var columns = table.ColumnDefinitions.Count;
        if (columns == 0) columns = rows.Length == 0 ? 1 : rows.Max(row => row.OfType<TableCell>().Sum(cell => Math.Max(1, cell.ColumnSpan)));
        var grid = new Grid { Margin = new Thickness(0, 0, 0, 12) };
        for (var c = 0; c < columns; c++) grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        for (var r = 0; r < rows.Length; r++) grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var lastRow = rows.Length - 1;
        for (var r = 0; r < rows.Length; r++)
        {
            var isHeader = rows[r].IsHeader;
            var column = 0;
            foreach (var cell in rows[r].OfType<TableCell>())
            {
                var text = new TextBlock { FontFamily = design.Font(), FontSize = design.FontSize(13),
                    TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true,
                    Foreground = design.Brush("text") };
                if (isHeader) text.FontWeight = FontWeights.SemiBold;
                text.Inlines.Add(CellInline(cell, design));
                var border = new Border
                {
                    Child = text, Padding = new Thickness(8, 6, 8, 6),
                    BorderBrush = design.Brush(isHeader ? "border-strong" : "border"),
                    BorderThickness = new Thickness(0, 0, 0, r == lastRow ? 0 : 1),
                    Background = isHeader ? design.Brush("bg-sunken") : null
                };
                Grid.SetRow(border, r);
                Grid.SetColumn(border, Math.Min(column, columns - 1));
                if (cell.ColumnSpan > 1) Grid.SetColumnSpan(border, Math.Min(cell.ColumnSpan, columns - column));
                grid.Children.Add(border);
                column += Math.Max(1, cell.ColumnSpan);
            }
        }
        return grid;
    }

    private static XamlInline CellInline(TableCell cell, WispDesign design)
    {
        var span = new Span();
        foreach (var child in cell)
        {
            if (child is ParagraphBlock paragraph && paragraph.Inline != null)
                foreach (var inline in paragraph.Inline) span.Inlines.Add(RenderInline(inline, design));
            else if (child is LeafBlock leaf) span.Inlines.Add(new Run { Text = leaf.Lines.ToString() });
        }
        return span;
    }

    private static void AppendBlock(RichTextBlock rich, MarkdownBlock block, WispDesign design, string prefix = "")
    {
        if (block is LeafBlock leaf)
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
                var bullet = list.IsOrdered ? $"{index}. " : "• ";
                var first = true;
                foreach (var child in item)
                {
                    AppendBlock(rich, child, design, first ? bullet : ""); first = false;
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
            case MathInline math: return new InlineUIContainer { Child = new NativeRichPreview(design, "math", math.Content.ToString(), "公式", false) };
            case LiteralInline literal: return new Run { Text = literal.Content.ToString() };
            case CodeInline code: return new Run { Text = code.Content, FontFamily = design.Font(true), FontSize = design.FontSize(12, true), Foreground = design.Brush("clay-strong") };
            case LineBreakInline: return new LineBreak();
            case TaskList task: return new Run { Text = task.Checked ? "☑ " : "☐ " };
            case HtmlInline html: return new Run { Text = html.Tag }; // Display HTML as inert text; never embed web content.
            case AutolinkInline link:
                return Uri.TryCreate(link.Url, UriKind.Absolute, out var auto) && auto.Scheme is "https" or "http"
                    ? new Hyperlink { NavigateUri = auto, Foreground = design.Brush("clay-strong"), Inlines = { new Run { Text = link.Url } } }
                    : new Run { Text = link.Url, Foreground = design.Brush("clay-strong") };
            case ContainerInline container:
                Span span = new();
                if (inline is EmphasisInline emphasis)
                {
                    if (emphasis.DelimiterCount >= 2) span.FontWeight = FontWeights.SemiBold;
                    else span.FontStyle = Windows.UI.Text.FontStyle.Italic;
                }
                if (inline is LinkInline target)
                {
                    if (target.IsImage)
                    {
                        var image = target.Url is { } imageUrl ? LocalImage(imageUrl) : null;
                        if (image != null) return new InlineUIContainer { Child = image };
                        return new Run { Text = "[图片] " + target.Url, Foreground = design.Brush("text-faint") };
                    }
                    if (Uri.TryCreate(target.Url, UriKind.Absolute, out var uri) && uri.Scheme is "https" or "http")
                        span = new Hyperlink { NavigateUri = uri };
                    span.Foreground = design.Brush("clay-strong");
                }
                foreach (var child in container) span.Inlines.Add(RenderInline(child, design));
                return span;
            default: return new Run { Text = inline.ToString() ?? "" };
        }
    }

    /// <summary>Render local images (absolute path or file:// URI) inline.
    /// Remote images stay as link text; the transcript never fetches the network.</summary>
    private static Image? LocalImage(string url)
    {
        string? path;
        try
        {
            if (Uri.TryCreate(url, UriKind.Absolute, out var uri))
            {
                if (uri.Scheme != "file") return null;
                path = uri.LocalPath;
            }
            else if (Path.IsPathRooted(url)) path = url;
            else return null;
            if (!File.Exists(path)) return null;
            return new Image
            {
                Source = new BitmapImage(new Uri(path)),
                MaxHeight = 360, MaxWidth = 680, Stretch = Stretch.Uniform,
                HorizontalAlignment = HorizontalAlignment.Left, Margin = new Thickness(0, 4, 0, 4)
            };
        }
        catch { return null; }
    }
}
