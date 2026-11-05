using Markdig.Extensions.Tables;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using MarkdownBlock = Markdig.Syntax.Block;
using XamlInline = Microsoft.UI.Xaml.Documents.Inline;

namespace Wisp.Science.Preview;

internal static partial class TranscriptView
{
    private static FrameworkElement SkillMarkdown(string source, WispDesign design)
        => ProseBlocks(NativeSkillMarkdown.Parse(source), design);

    private static StackPanel ProseBlocks(IEnumerable<MarkdownBlock> blocks, WispDesign design, bool quote = false, bool response = false, bool commentary = false,
        double? proseSize = null, double? leading = null)
    {
        var panel = new StackPanel(); RichTextBlock? rich = null;
        var size = proseSize ?? (response ? design.Typography.ResponseSize(design.Localization.Locale, commentary) : design.Typography.MarkdownSize);
        var zh = design.Localization.Locale == "zh";
        void Flush() { if (rich != null) { panel.Children.Add(rich); rich = null; } }
        RichTextBlock Text()
        {
            if (rich != null) return rich;
            rich = NewRich(design); rich.FontSize = size;
            rich.FontFamily = new FontFamily(design.Typography.MarkdownFontSource(design.Localization.Locale));
            rich.Foreground = design.Brush(quote || commentary ? "text-muted" : "text");
            // Wrapped inline-code chips may be taller than a normal text line.
            rich.LineHeight = size * (leading ?? (response ? design.Typography.ResponseLeading(design.Localization.Locale, commentary) : 1.65));
            rich.LineStackingStrategy = LineStackingStrategy.MaxHeight;
            if (response) rich.CharacterSpacing = zh ? 10 : -4;
            return rich;
        }
        void Add(MarkdownBlock block, double indent = 0, string marker = "", bool listItem = false, bool continuation = false)
        {
            if (block is ListBlock list)
            {
                var number = int.TryParse(list.OrderedStart, out var first) ? first : 1;
                var step = size * (indent == 0 ? 1.35 : 1.3);
                var firstItem = true;
                foreach (var item in list.OfType<ListItemBlock>())
                {
                    if (!NativeSkillMarkdown.HasVisibleContent(item)) { number++; continue; }
                    var before = Text().Blocks.Count; var firstChild = true;
                    foreach (var child in item)
                    {
                        Add(child, indent + step, firstChild ? (list.IsOrdered ? $"{number}. " : indent == 0 ? "• " : "◦ ") : "", true, !firstChild);
                        firstChild = false;
                    }
                    if (rich != null && rich.Blocks.Count > before && rich.Blocks[before] is Paragraph firstParagraph)
                        firstParagraph.Margin = new Thickness(firstParagraph.Margin.Left,
                            size * (firstItem ? .2 : zh ? .42 : .3), 0, firstParagraph.Margin.Bottom);
                    number++; firstItem = false;
                }
                if (rich?.Blocks.LastOrDefault() is Paragraph last)
                    last.Margin = new Thickness(last.Margin.Left, last.Margin.Top, 0, size * (indent == 0 ? .65 : .08));
                return;
            }
            if (block is QuoteBlock quoted)
            {
                Flush(); panel.Children.Add(new Border { Child = ProseBlocks(quoted, design, true, response, commentary, proseSize, leading),
                    Margin = new Thickness(indent, size * .45, 0, size * .7), Padding = new Thickness(size, size * .15, 0, size * .15),
                    BorderBrush = design.Brush("clay-soft"), BorderThickness = new Thickness(3, 0, 0, 0) }); return;
            }
            if (block is ThematicBreakBlock)
            {
                Flush(); panel.Children.Add(new Border { Height = 1, Background = design.Brush("border"),
                    Margin = new Thickness(indent, size * .85, 0, size * .85) }); return;
            }
            if (response && block is Markdig.Extensions.Mathematics.MathBlock math)
            {
                Flush(); panel.Children.Add(new NativeRichPreview(design, "math", math.Lines.ToString(), design.L("公式", "Formula"))); return;
            }
            if (block is CodeBlock code)
            {
                Flush(); var element = response ? ResponseCode(code.Lines.ToString(), (code as FencedCodeBlock)?.Info, design) : SkillCode(code.Lines.ToString(), design);
                element.Margin = new Thickness(indent, 0, 0, 12); panel.Children.Add(element); return;
            }
            if (block is Table table)
            {
                Flush(); var element = SkillTable(table, design, response); element.Margin = new Thickness(indent, 0, 0, 10);
                panel.Children.Add(element); return;
            }
            if (block is LeafBlock leaf)
            {
                var target = Text(); var paragraph = new Paragraph { Margin = new Thickness(indent, continuation ? size * .18 : 0, 0, listItem ? 0 : size * (zh ? .7 : .55)) };
                if (continuation) { paragraph.Foreground = design.Brush("text-muted"); paragraph.FontSize = size * .96; }
                if (response && !quote && !listItem && block is ParagraphBlock lead && NativeResponseMarkdown.HasStrongLead(lead))
                {
                    paragraph.Margin = new Thickness(indent, panel.Children.Count == 0 && target.Blocks.Count == 0 ? 0 : size * 1.05, 0, size * .5);
                    paragraph.Inlines.Add(new InlineUIContainer { Child = new Border { Width = 3, Height = size * 1.2,
                        Background = design.Brush("clay"), Margin = new Thickness(0, 0, size * .55, 0) } });
                }
                if (block is HeadingBlock heading)
                {
                    var (scale, line) = heading.Level switch { 1 => (zh ? 1.38 : 1.55, zh ? 1.45 : 1.3),
                        2 => (zh ? 1.18 : 1.28, zh ? 1.45 : 1.35), 3 => (zh ? 1.06 : 1.12, zh ? 1.45 : 1.4), _ => (1.02, 1.45) };
                    paragraph.FontSize = size * scale; paragraph.LineHeight = paragraph.FontSize * line; paragraph.FontWeight = FontWeights.SemiBold;
                    paragraph.Margin = new Thickness(indent, panel.Children.Count == 0 && target.Blocks.Count == 0 || heading.Level == 1 ? 0 : paragraph.FontSize * (zh ? 1.15 : 1), 0, paragraph.FontSize * (zh ? .45 : .4));
                }
                if (marker.Length > 0)
                {
                    var markerText = new TextBlock { Text = marker, FontFamily = design.Font(), FontSize = size * .88 };
                    markerText.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
                    paragraph.TextIndent = -markerText.DesiredSize.Width;
                    paragraph.Inlines.Add(new Run { Text = marker, FontFamily = design.Font(), Foreground = design.Brush("clay-strong"), FontSize = size * .88 });
                }
                if (leaf.Inline != null) foreach (var inline in leaf.Inline) paragraph.Inlines.Add(SkillInline(inline, design, target, response, size));
                else paragraph.Inlines.Add(new Run { Text = leaf.Lines.ToString() });
                target.Blocks.Add(paragraph); return;
            }
            if (block is ContainerBlock container) foreach (var child in container) Add(child, indent, marker, listItem, continuation);
        }
        foreach (var block in blocks) Add(block);
        Flush();
        if (response && panel.Children.LastOrDefault() is RichTextBlock lastRich && lastRich.Blocks.LastOrDefault() is Paragraph lastParagraph)
            lastParagraph.Margin = new Thickness(lastParagraph.Margin.Left, lastParagraph.Margin.Top, 0, 0);
        return panel;
    }

    private static XamlInline SkillInline(Markdig.Syntax.Inlines.Inline inline, WispDesign design, FrameworkElement owner, bool response = false, double? proseSize = null)
    {
        if (response && inline is LinkInline { IsImage: true }) return RenderInline(inline, design, true);
        var codeText = inline is CodeInline code ? code.Content
            : response && inline is LinkInline localLink ? NativeResponseMarkdown.LocalLinkCode(localLink) : null;
        if (codeText != null)
        {
            var size = (proseSize ?? design.Typography.MarkdownSize) * .86;
            var text = new TextBlock { Text = codeText, FontFamily = design.Font(true), FontSize = size,
                Foreground = design.Brush("text"), TextWrapping = TextWrapping.Wrap, IsTextSelectionEnabled = true };
            var chip = new Border { Child = text, Padding = new Thickness(size * .35, size * .1, size * .35, size * .1),
                Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(5) };
            owner.SizeChanged += (_, e) => chip.MaxWidth = Math.Max(1, e.NewSize.Width - 48);
            return new InlineUIContainer { Child = chip };
        }
        if (inline is ContainerInline container)
        {
            Span span = new();
            if (inline is LinkInline link)
            {
                if (Uri.TryCreate(link.Url, UriKind.Absolute, out var uri) && uri.Scheme is "https" or "http")
                    span = new Hyperlink { NavigateUri = uri };
                if (!string.IsNullOrEmpty(link.Url)) span.Foreground = design.Brush("clay-strong");
            }
            if (inline is EmphasisInline emphasis)
            {
                if (emphasis.DelimiterChar == '~' && emphasis.DelimiterCount == 2) span.TextDecorations = Windows.UI.Text.TextDecorations.Strikethrough;
                else if (emphasis.DelimiterCount >= 2) span.FontWeight = FontWeights.SemiBold;
                else if (design.Localization.Locale == "zh") span.FontWeight = FontWeights.Medium;
                else span.FontStyle = Windows.UI.Text.FontStyle.Italic;
            }
            foreach (var child in container) span.Inlines.Add(SkillInline(child, design, owner, response, proseSize));
            return span;
        }
        return RenderInline(inline, design, true);
    }

    private static FrameworkElement SkillTable(Table table, WispDesign design, bool response = false)
    {
        var rows = table.OfType<TableRow>().ToArray(); var columns = NativeTranscriptTables.ColumnCount(table);
        var grid = new Grid(); var preferredWidths = new double[columns]; var minimumWidths = new double[columns];
        for (var c = 0; c < columns; c++) grid.ColumnDefinitions.Add(new());
        for (var r = 0; r < rows.Length; r++)
        {
            grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); var column = 0;
            foreach (var cell in rows[r].OfType<TableCell>())
            {
                var text = new RichTextBlock { FontFamily = new FontFamily(design.Typography.MarkdownFontSource(design.Localization.Locale)),
                    FontSize = response ? design.Typography.ResponseTableSize : design.FontSize(13), Foreground = design.Brush("text"), TextWrapping = response ? TextWrapping.WrapWholeWords : TextWrapping.Wrap, IsTextSelectionEnabled = true };
                if (response) { text.LineHeight = text.FontSize * design.Typography.ResponseLeading(design.Localization.Locale); text.LineStackingStrategy = LineStackingStrategy.MaxHeight; }
                if (rows[r].IsHeader) text.FontWeight = FontWeights.SemiBold;
                var paragraph = new Paragraph();
                foreach (var part in cell.OfType<ParagraphBlock>())
                    if (part.Inline != null) foreach (var inline in part.Inline) paragraph.Inlines.Add(SkillInline(inline, design, text, response, response ? text.FontSize : null));
                text.Blocks.Add(paragraph);
                if (response)
                {
                    text.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
                    var minimum = 0d;
                    foreach (var word in NativeResponseMarkdown.MinimumWidthWords(NativeTranscriptTables.CellText(cell)))
                    {
                        var measure = new RichTextBlock { FontFamily = text.FontFamily, FontSize = text.FontSize, FontWeight = text.FontWeight };
                        var wordParagraph = new Paragraph(); wordParagraph.Inlines.Add(new Run { Text = word }); measure.Blocks.Add(wordParagraph);
                        measure.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
                        minimum = Math.Max(minimum, measure.DesiredSize.Width);
                    }
                    var spanCount = Math.Max(1, cell.ColumnSpan);
                    for (var c = column; c < Math.Min(columns, column + spanCount); c++)
                    {
                        preferredWidths[c] = Math.Max(preferredWidths[c], (text.DesiredSize.Width + 20) / spanCount);
                        minimumWidths[c] = Math.Max(minimumWidths[c], Math.Ceiling((minimum + 22) / spanCount));
                    }
                }
                var border = new Border { Child = text, Padding = new Thickness(10, 6, 10, 6),
                    BorderBrush = design.Brush("border"), BorderThickness = new Thickness(column == 0 ? 1 : 0, r == 0 ? 1 : 0, 1, 1),
                    Background = rows[r].IsHeader ? design.Brush("bg-elev") : null };
                Grid.SetRow(border, r); Grid.SetColumn(border, Math.Min(column, columns - 1));
                var span = Math.Min(Math.Max(1, cell.ColumnSpan), columns - column); Grid.SetColumnSpan(border, span);
                grid.Children.Add(border); column += span;
            }
        }
        if (!response) return grid;
        for (var c = 0; c < columns; c++)
        {
            grid.ColumnDefinitions[c].Width = new GridLength(Math.Max(1, preferredWidths[c]), GridUnitType.Star);
            grid.ColumnDefinitions[c].MinWidth = minimumWidths[c];
        }
        var scroll = new ScrollViewer { Content = grid, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled };
        grid.Width = Math.Max(1, minimumWidths.Sum());
        scroll.SizeChanged += (_, e) => grid.Width = Math.Max(e.NewSize.Width, minimumWidths.Sum());
        var copy = new Button { Content = design.T("table.copy"), HorizontalAlignment = HorizontalAlignment.Right,
            Padding = new Thickness(10, 4, 10, 4), CornerRadius = new(99), FontSize = design.FontSize(11.5),
            FontFamily = new FontFamily(design.Typography.MarkdownFontSource(design.Localization.Locale)), FontWeight = FontWeights.SemiBold,
            Background = design.Brush("bg-elev"), Foreground = design.Brush("text-muted"), BorderBrush = design.Brush("border-strong") };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(copy, design.T("table.copy"));
        copy.Click += (_, _) => { try { var data = new Windows.ApplicationModel.DataTransfer.DataPackage(); data.SetText(NativeTranscriptTables.Copy(table)); Windows.ApplicationModel.DataTransfer.Clipboard.SetContent(data); } catch { } };
        var content = new StackPanel(); content.Children.Add(new Border { Child = copy, Padding = new(11, 6, 11, 6),
            Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new(1, 1, 1, 0), CornerRadius = new(10, 10, 0, 0) }); content.Children.Add(scroll);
        return content;
    }
}
