using System.Text;
using System.Security.Cryptography;
using Markdig;
using Markdig.Extensions.Tables;
using Markdig.Syntax;
using Markdig.Syntax.Inlines;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeTranscriptTable(string Id, string Name, Table Content, int Rows, int Columns, string CopyText);

/// <summary>Read-only tables from the displayed transcript page. These are not
/// registered files. Cache unchanged messages during streaming/polling, and
/// retain source identities so selection cannot silently move to another table.</summary>
public sealed class NativeTranscriptTables
{
    private static readonly MarkdownPipeline Pipeline = new MarkdownPipelineBuilder().UsePipeTables().Build();
    private Dictionary<string, (string Text, Table[] Tables)> cache = [];
    public IReadOnlyList<NativeTranscriptTable> Items { get; private set; } = [];
    public string? SelectedId { get; private set; }
    public NativeTranscriptTable? Selected => Items.FirstOrDefault(item => item.Id == SelectedId);
    public bool Select(string id)
    {
        if (!Items.Any(item => item.Id == id)) return false;
        SelectedId = id; return true;
    }
    public void Dismiss() => SelectedId = null;

    public bool Update(ConversationSnapshot? page)
    {
        var next = new Dictionary<string, (string Text, Table[] Tables)>();
        var items = new List<NativeTranscriptTable>();
        if (page != null)
        {
            var keys = NativeTranscriptRows.Keys(page);
            for (var index = 0; index < page.Items.Length; index++)
            {
                var row = page.Items[index];
                // Match the WebView's completion-output projection as well as
                // assistant Markdown; never treat user examples or raw logs as results.
                if (row.Role != "assistant" && !NativeTranscriptActivity.Completion(row)) continue;
                var key = keys[index];
                if (!cache.TryGetValue(key, out var parsed) || parsed.Text != row.Text)
                    parsed = (row.Text, Markdown.Parse(row.Text, Pipeline).Descendants<Table>().ToArray());
                next[key] = parsed;
                var occurrences = new Dictionary<string, int>();
                for (var ordinal = 0; ordinal < parsed.Tables.Length; ordinal++)
                {
                    var table = parsed.Tables[ordinal];
                    var rows = table.OfType<TableRow>().ToArray();
                    var columns = ColumnCount(table);
                    var text = Copy(table);
                    var identity = Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(text)));
                    var occurrence = occurrences.GetValueOrDefault(identity);
                    occurrences[identity] = occurrence + 1;
                    items.Add(new($"{key}/table:{identity}/{occurrence}", $"表格 {items.Count + 1}", table,
                        rows.Count(row => !row.IsHeader), columns, text));
                }
            }
        }
        cache = next;
        if (Items.SequenceEqual(items)) return false;
        Items = items;
        if (Selected == null) SelectedId = null;
        return true;
    }

    /// <summary>Visible cell text as TSV, including the header; formatting and
    /// link targets are not copied as Markdown syntax.</summary>
    public static string Copy(Table table) => string.Join("\n", table.OfType<TableRow>().Select(row =>
        string.Join("\t", row.OfType<TableCell>().Select(CellText))));

    // Pipe-parser definitions can include an unused column for an escaped pipe.
    // The resolved cells are the authoritative visible table dimensions.
    public static int ColumnCount(Table table) => Math.Max(1, table.OfType<TableRow>()
        .Select(row => row.OfType<TableCell>().Sum(cell => Math.Max(1, cell.ColumnSpan))).DefaultIfEmpty().Max());

    private static string CellText(TableCell cell)
    {
        var text = new StringBuilder();
        foreach (var block in cell.Descendants<LeafBlock>())
        {
            if (text.Length > 0) text.Append(' ');
            if (block.Inline != null) Append(block.Inline);
            else text.Append(block.Lines);
        }
        return text.ToString().Replace('\t', ' ').Replace('\r', ' ').Replace('\n', ' ').Trim();

        void Append(Inline inline)
        {
            switch (inline)
            {
                case LiteralInline literal: text.Append(literal.Content); break;
                case CodeInline code: text.Append(code.Content); break;
                case HtmlEntityInline entity: text.Append(entity.Transcoded); break;
                case AutolinkInline link: text.Append(link.Url); break;
                case LineBreakInline: text.Append(' '); break;
                case HtmlInline html when html.Tag.StartsWith("<br", StringComparison.OrdinalIgnoreCase): text.Append(' '); break;
                case ContainerInline container:
                    foreach (var child in container) Append(child);
                    break;
            }
        }
    }
}
