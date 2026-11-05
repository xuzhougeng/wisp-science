using Wisp.ProjectBrowser;

internal static class NativeCsvTableTests
{
    public static void Run()
    {
        var table = NativeCsvTable.Parse("\uFEFFname,notes,count\r\n\"a,b\",\"line 1\nline \"\"2\"\"\",4\r\nlast,,\r\n\r\n");
        Check(table.Headers.SequenceEqual(new[] { "name", "notes", "count" }) && table.Rows.Length == 2, "BOM, CRLF and blank lines");
        Check(table.Rows[0].SequenceEqual(new[] { "a,b", "line 1\nline \"2\"", "4" }), "quoted commas, newlines and escaped quotes");
        Check(table.Rows[1].SequenceEqual(new[] { "last", "", "" }), "empty cells and trailing delimiter");
        Check(table.CopyText.Contains("a,b\tline 1 line \"2\"\t4") && !table.Truncated, "copy flattens cell newlines to TSV");
        var wide = NativeCsvTable.Parse("a,b\n1,2,3");
        Check(wide.Columns == 3 && wide.Rows.Single().Length == 3, "ragged input retains extra cells");
        Check(NativeCsvTable.Parse("").Headers.Length == 0, "empty document");
        var bounded = NativeCsvTable.Parse("a\ncomplete\npartial", true);
        Check(bounded.Truncated && bounded.Rows.Single().Single() == "complete", "bounded reads omit partial final record");
        var longText = NativeCsvTable.Parse("a\n" + new string('x', NativeCsvTable.MaxCharacters + 2));
        Check(longText.Truncated && longText.Rows.Length == 0, "character budget");
        var dense = NativeCsvTable.Parse("a\n" + string.Concat(Enumerable.Repeat(",\n", 100_001)));
        Check(dense.Truncated && dense.Rows.Length <= 50_000, "cell budget");
        var many = NativeCsvTable.Parse("header\n" + string.Join("\n", Enumerable.Range(0, 501)));
        Check(many.Rows.Length == 501 && many.CopyText.EndsWith("500") && NativeCsvTable.VisibleRows == 500, "render limit does not truncate copied records");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
}
