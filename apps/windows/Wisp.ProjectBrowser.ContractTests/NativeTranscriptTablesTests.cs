using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeTranscriptTablesTests
{
    public static void Run()
    {
        static void Check(bool ok, string message) { if (!ok) throw new InvalidOperationException(message); }
        static ConversationItem Row(string role, string text, string? tool = null, bool? ok = null) => new(role, text, tool, null, ok, null);
        const string markdown = "| 状态 | 样本 |\n| :--- | ---: |\n| **完成** | `A` &amp; [B](https://example.test) |\n| 进行中 | O\\|001 😀 |";
        var page = new ConversationSnapshot(ConversationSnapshot.SchemaId, "host", 1, "p", "s",
            [Row("user", markdown), Row("assistant", "## Result\n\n" + markdown), Row("tool", markdown, "attempt_completion", true)],
            null, false, false, false, "m", null, null, [], UserOffset: 0);
        var tables = new NativeTranscriptTables();
        Check(tables.Update(page) && tables.Items.Count == 2, "assistant and completion tables join artifacts without collecting user input");
        var first = tables.Items[0];
        Check(first.Name == "表格 1" && first.Rows == 2 && first.Columns == 2,
            $"table dimensions exclude header row: {first.Name}, {first.Rows} x {first.Columns}, {first.CopyText}");
        Check(first.CopyText == "状态\t样本\n完成\tA & B\n进行中\tO|001 😀", "TSV preserves visible Unicode/cell text and strips Markdown formatting");
        Check(tables.Select(first.Id) && !tables.Select("missing") && tables.Selected == first, "selection validates source identity");
        Check(!tables.Update(page with { Sequence = 2 }) && ReferenceEquals(first.Content, tables.Items[0].Content)
            && tables.Selected?.Id == first.Id, "polling reuses parsed tables and keeps preview selection");
        var changed = page with { Items = [page.Items[0], page.Items[1] with { Text = markdown.Replace("完成", "更新") }, page.Items[2]] };
        Check(tables.Update(changed) && tables.Items[0].CopyText.Contains("更新") && tables.Selected == null,
            "streaming edits replace table data without silently changing an open preview");
        tables.Select(tables.Items[0].Id);
        var selectedId = tables.SelectedId;
        tables.Update(changed with { Items = [changed.Items[0], changed.Items[1] with { Text = "| new |\n|---|\n| table |\n\n" + changed.Items[1].Text }, changed.Items[2]] });
        Check(tables.SelectedId == selectedId && tables.Selected?.Name == "表格 2",
            "inserting another table before the selection preserves the selected source, not its ordinal");
        Check(tables.Update(changed with { Items = [changed.Items[0], Row("assistant", "No table now.")] }) && tables.Selected == null,
            "removed tables close preview rather than pointing to another result");
        tables.Update(page); tables.Select(tables.Items[0].Id);
        tables.Update(page with { UserOffset = 10 });
        Check(tables.Selected == null, "identical tables on another history page do not inherit selection");
        tables.Select(tables.Items[0].Id); tables.Update(page with { SessionId = "other" });
        Check(tables.Selected == null, "identical tables in another session do not inherit selection");
        tables.Select(tables.Items[0].Id); tables.Dismiss();
        Check(tables.Selected == null && tables.Items.Count == 2, "Escape-equivalent dismiss keeps collected artifacts");
        tables.Update(page with { Items = [Row("assistant", "```markdown\n" + markdown + "\n```\n\n    | a | b |\n    |---|---|\n    | c | d |"),
            Row("tool", markdown, "shell", true), Row("reasoning", markdown), Row("tool", markdown, "attempt_completion", false)] });
        Check(tables.Items.Count == 0, "code examples, logs, reasoning and failed completion output do not become table artifacts");
        tables.Update(page with { Items = [Row("assistant", "> " + markdown.Replace("\n", "\n> "))] });
        Check(tables.Items.Single().CopyText == first.CopyText, "nested Markdown tables use the same parser and visible cell text");
        tables.Update(page with { Items = [Row("assistant", "| unfinished | table |\nordinary paragraph")] });
        Check(tables.Items.Count == 0, "unfinished streaming table syntax is not materialized");
        tables.Update(page); tables.Update(null);
        Check(tables.Items.Count == 0 && tables.Selected == null, "closing a transcript clears its projection");
        Console.WriteLine("Native transcript tables: collection, TSV, cache, streaming, page/session identity and dismissal passed.");
    }
}
