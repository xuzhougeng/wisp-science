using System.Text;

namespace Wisp.ProjectBrowser;

/// <summary>Bounded CSV preview data. Quoted commas, quotes and newlines remain cell content.</summary>
public sealed record NativeCsvTable(string[] Headers, string[][] Rows, bool Truncated)
{
    public const int MaxCharacters = 1024 * 1024;
    public const int VisibleRows = 500;
    // Bound XAML allocation even for unusually wide scientific matrices.
    public const int VisibleColumns = 64;
    private const int MaxCells = 100_000;
    public int Columns => Math.Max(Headers.Length, Rows.Select(row => row.Length).DefaultIfEmpty().Max());
    public string CopyText => string.Join("\n", new[] { Headers }.Concat(Rows).Select(row =>
        string.Join("\t", row.Select(cell => cell.Replace('\t', ' ').Replace('\r', ' ').Replace('\n', ' ')))));

    public static NativeCsvTable Parse(string source, bool truncated = false)
    {
        var length = Math.Min(source.Length, MaxCharacters);
        truncated |= source.Length > length;
        var records = new List<string[]>(); var row = new List<string>(); var cell = new StringBuilder();
        var quoted = false; var touched = false; var cells = 0; var stopped = false;
        for (var i = source.StartsWith('\uFEFF') ? 1 : 0; i < length; i++)
        {
            var ch = source[i];
            if (quoted)
            {
                if (ch == '"' && i + 1 < length && source[i + 1] == '"') { cell.Append('"'); i++; }
                else if (ch == '"') quoted = false;
                else cell.Append(ch);
                continue;
            }
            if (ch == '"' && cell.Length == 0) { quoted = true; touched = true; }
            else if (ch == ',') { row.Add(cell.ToString()); cell.Clear(); touched = true; cells++; }
            else if (ch is '\r' or '\n')
            {
                FinishRow();
                if (ch == '\r' && i + 1 < length && source[i + 1] == '\n') i++;
            }
            else { cell.Append(ch); touched = true; }
            if (cells >= MaxCells) { truncated = true; stopped = true; break; }
        }
        // A bounded read may end in a partial record; do not present it as complete.
        if (!stopped && !truncated) FinishRow();
        return new(records.FirstOrDefault() ?? [], records.Skip(1).ToArray(), truncated || quoted);

        void FinishRow()
        {
            if (touched || row.Count > 0 || cell.Length > 0)
            { row.Add(cell.ToString()); cells++; records.Add(row.ToArray()); }
            row.Clear(); cell.Clear(); touched = false;
        }
    }
}
