using Markdig.Extensions.Tables;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using Wisp.ProjectBrowser;

internal static class NativeSkillMarkdownTests
{
    public static void Run()
    {
        var document = NativeSkillMarkdown.Parse("""
            # Study

            A `path with spaces.csv`, ~~retired~~ and **current** method.

            3. First stage
               - Nested input
                 - Deeper input
               - Nested output
            4. Second stage

            > A quoted instruction.
            >
            > - A list inside a quote

            | Measure | Value |
            | --- | --- |
            | `cell\|count` | **23** |

            ```python
            print("<unchanged>")
            ```

            ![Visible image description](file:///C:/private.png)
            ![Remote description](https://example.invalid/private.png)
            [Reference](https://example.invalid/paper)

            <script>inert()</script>
            """);
        var lists = document.Descendants<ListBlock>().ToArray();
        Check(lists[0].IsOrdered && lists[0].OrderedStart == "3" && lists[0].Count == 2,
            "ordered lists preserve their starting number and direct item boundaries");
        Check(lists[0].Descendants<ListBlock>().Count() == 2 && lists[0].Descendants<ListItemBlock>().All(NativeSkillMarkdown.HasVisibleContent),
            "nested list levels and content remain available to native indentation");
        Check(document.Descendants<QuoteBlock>().Single().Descendants<ListBlock>().Count() == 1,
            "quote contents preserve nested structures");
        var table = document.Descendants<Table>().Single();
        Check(NativeTranscriptTables.ColumnCount(table) == 2 && NativeTranscriptTables.Copy(table).Contains("cell|count\t23"),
            "table cells retain escaped pipes, inline code and emphasis without extra columns");
        Check(NativeSkillMarkdown.Parse(@"`cell\|count`").Descendants<CodeInline>().Single().Content == @"cell\|count",
            "a pipe in ordinary inline code keeps its literal backslash");
        Check(document.Descendants<FencedCodeBlock>().Single().Lines.ToString() == "print(\"<unchanged>\")",
            "fenced code is not interpreted as HTML or inline markup");
        Check(document.Descendants<EmphasisInline>().Any(e => e.DelimiterChar == '~' && e.DelimiterCount == 2)
            && document.Descendants<EmphasisInline>().Any(e => e.DelimiterChar == '*' && e.DelimiterCount == 2),
            "strikethrough and bold retain distinct semantic delimiters");
        var links = document.Descendants<LinkInline>().ToArray();
        Check(links.Count(l => l.Url == "") == 2 && links.All(l => !l.IsImage)
            && links.Any(l => l.Url == "https://example.invalid/paper"),
            "local and remote images remain alt text while ordinary document links retain their target");
        Check(!document.Descendants<HtmlBlock>().Any() && !document.Descendants<HtmlInline>().Any(),
            "skill HTML remains inert text");
        var empty = NativeSkillMarkdown.Parse("- \n- Kept").Descendants<ListItemBlock>().ToArray();
        Check(empty.Length == 2 && !NativeSkillMarkdown.HasVisibleContent(empty[0]) && NativeSkillMarkdown.HasVisibleContent(empty[1]),
            "empty list items do not produce orphan bullets");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
