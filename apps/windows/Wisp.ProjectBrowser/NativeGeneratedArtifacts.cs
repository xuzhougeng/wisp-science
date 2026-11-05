using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeGeneratedOutput(string Id, string ProjectId, string SessionId, string SourceKey,
    int SourceItem, string Name, string Kind, string? Path, string? Text, NativeCsvTable? Table, bool Superseded = false);

/// <summary>The transcript's generated outputs, distinct from registered artifact files.
/// Extraction follows ui/src/app_support/artifacts.rs; unchanged rows reuse parsed material.</summary>
public sealed class NativeGeneratedArtifacts
{
    private static readonly Markdig.MarkdownPipeline CellPipeline = CreateCellPipeline();
    private static Markdig.MarkdownPipeline CreateCellPipeline()
    {
        var builder = new Markdig.MarkdownPipelineBuilder();
        builder.BlockParsers.Clear(); builder.BlockParsers.Add(new Markdig.Parsers.ParagraphBlockParser());
        return builder.Build();
    }
    public static Markdig.Syntax.MarkdownDocument ParseCell(string text) => Markdig.Markdown.Parse(text, CellPipeline);
    private sealed record Material(string Kind, string? Path = null, string? Text = null, NativeCsvTable? Table = null);
    private sealed record Parsed(Material Value, string Identity);
    private Dictionary<string, (string Fingerprint, Parsed[] Outputs)> cache = [];
    public IReadOnlyList<NativeGeneratedOutput> Items { get; private set; } = [];
    public NativeGeneratedOutput[] At(string sourceKey) => Items.Where(item => item.SourceKey == sourceKey).ToArray();
    public NativeGeneratedOutput? Find(string id) => Items.FirstOrDefault(item => item.Id == id);
    public bool Update(ConversationSnapshot? snapshot, NativeLocalization locale)
    {
        var next = new Dictionary<string, (string Fingerprint, Parsed[] Outputs)>();
        var result = new List<NativeGeneratedOutput>(); var tableNumber = 0; var csvNumber = 0; var equationNumber = 0;
        if (snapshot != null)
        {
            var keys = NativeTranscriptRows.Keys(snapshot);
            for (var index = 0; index < snapshot.Items.Length; index++)
            {
                var row = snapshot.Items[index]; var key = keys[index]; var fingerprint = row.Role + "\n" + row.ToolName + "\n" + row.Text;
                if (!cache.TryGetValue(key, out var parsed) || parsed.Fingerprint != fingerprint)
                    parsed = (fingerprint, Extract(row).Select(value => new Parsed(value, Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(JsonSerializer.Serialize(value)))))).ToArray());
                next[key] = parsed;
                var owner = row.Role == "file_changed" ? FileOwner(snapshot.Items, index) : index;
                var occurrences = new Dictionary<string, int>(); var files = new HashSet<string>(StringComparer.Ordinal);
                foreach (var entry in parsed.Outputs)
                {
                    var value = entry.Value;
                    if (value.Path != null && !files.Add(value.Path)) continue;
                    var name = value.Path != null ? value.Path.Split('/', '\\').Last() : value.Kind switch
                    {
                        "table" => locale.Format("artifact.table", ("n", (++tableNumber).ToString())),
                        "csv" => $"data-{++csvNumber}.csv", "fasta" => $"alignment-{csvNumber}.fasta",
                        "latex" => locale.Format("artifact.equation", ("n", (++equationNumber).ToString())), _ => value.Kind
                    };
                    if (value.Path != null)
                        for (var prior = 0; prior < result.Count; prior++)
                            if (result[prior].Path == value.Path) result[prior] = result[prior] with { Superseded = true };
                    var occurrence = occurrences.GetValueOrDefault(entry.Identity); occurrences[entry.Identity] = occurrence + 1;
                    result.Add(new(key + "/generated:" + entry.Identity + "/" + occurrence, snapshot.ProjectId, snapshot.SessionId,
                        keys[owner], owner, name, value.Kind, value.Path, value.Text, value.Table));
                }
            }
        }
        cache = next;
        if (Items.SequenceEqual(result)) return false;
        Items = result; return true;
    }
    private static IEnumerable<Material> Extract(ConversationItem row)
    {
        if (row.Role == "file_changed") { if (File(row.Text) is { } changed) yield return changed; yield break; }
        if (row.Role == "user")
        {
            foreach (var word in Regex.Split(row.Text, "[\\s,\"']+")) if (File(word) is { } file) yield return file;
            yield break;
        }
        if (row.Role != "assistant" && !(row.Role == "tool" && row.ToolName == "attempt_completion" && row.Text.Length > 0)) yield break;
        var lines = Lines(row.Text);
        for (var i = 0; i + 1 < lines.Length; i++)
        {
            if (!TableRow(lines[i]) || !Separator(lines[i + 1])) continue;
            var headers = Cells(lines[i]); var rows = new List<string[]>(); var end = i + 2;
            while (end < lines.Length && TableRow(lines[end])) rows.Add(Cells(lines[end++]));
            yield return new("table", Table: new(headers, rows.ToArray(), false)); i = end - 1;
        }
        for (var i = 0; i < lines.Length; i++)
        {
            if (!lines[i].Trim().StartsWith("```", StringComparison.Ordinal)) continue;
            var language = lines[i].Trim().TrimStart('`').Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries).FirstOrDefault()?.ToLowerInvariant();
            var start = ++i; while (i < lines.Length && !lines[i].Trim().StartsWith("```", StringComparison.Ordinal)) i++;
            var body = string.Join("\n", lines[start..i]); if (body.Length == 0) continue;
            if (language is "csv" or "tsv")
            {
                // The WebView transcript scanner uses its simple comma-cell parser for both fence labels.
                var data = Lines(body).Select(line => line.Split(',').Select(cell => cell.Trim().Trim('"')).ToArray()).ToArray();
                if (data.Length > 0) yield return new("csv", Table: new(data[0], data.Skip(1).ToArray(), false));
            }
            else if (language is "fasta" or "fa") yield return new("fasta", Text: body);
        }
        for (var i = 0; i < lines.Length; i++)
        {
            if (lines[i].Trim().StartsWith("```", StringComparison.Ordinal))
            { for (i++; i < lines.Length && !lines[i].Trim().StartsWith("```", StringComparison.Ordinal); i++) { } continue; }
            if (!lines[i].Trim().StartsWith('$')) continue;
            var body = new List<string>();
            for (i++; i < lines.Length && !lines[i].Trim().EndsWith('$'); i++) body.Add(lines[i]);
            if (i < lines.Length) body.Add(lines[i].Trim().TrimEnd('$'));
            yield return new("latex", Text: string.Join("\n", body));
        }
    }
    private static string[] Lines(string text)
    {
        if (text.Length == 0) return [];
        var lines = text.Replace("\r\n", "\n").Split('\n'); return text.EndsWith('\n') ? lines[..^1] : lines;
    }
    private static string[] Cells(string line) => line.Trim().Trim('|').Split('|').Select(cell => cell.Trim()).ToArray();
    private static bool TableRow(string line) => line.Trim().Length > 0 && line.Contains('|');
    private static bool Separator(string line) => Cells(line).All(cell => cell.Contains('-') && cell.All(c => c is '-' or ':'));
    private static Material? File(string word)
    {
        var path = word.Trim().Trim('`').Trim('"').Trim('\'');
        while (path.StartsWith("./", StringComparison.Ordinal)) path = path[2..];
        while (path.StartsWith(".\\", StringComparison.Ordinal)) path = path[2..];
        var shorthand = Regex.Match(path, @"\.(png|jpg|jpeg|gif|webp|svg)[/\\]\.pdf", RegexOptions.IgnoreCase);
        if (shorthand.Success) path = path[..(shorthand.Index + shorthand.Groups[1].Length + 1)];
        var kind = FileKind(path); return kind == null ? null : new(kind, Path: path);
    }
    public static string? FileKind(string path) => !path.Contains('.') ? null : path.Split('.').Last().ToLowerInvariant() switch
    {
        "csv" or "tsv" => "csv", "pdf" => "pdf", "png" or "jpg" or "jpeg" or "gif" or "webp" or "svg" => "image",
        "pdb" or "mol2" or "cif" => "structure", "sdf" or "mol" => "molecule", "smi" or "smiles" => "smiles",
        "aln" or "clustal" or "clustalw" or "sto" or "stockholm" or "stk" or "afa" or "mfa" => "msa",
        "fasta" or "fa" or "fas" or "fna" or "faa" or "ffn" or "frn" => "fasta",
        "md" or "rmd" or "qmd" or "markdown" => "markdown", "docx" => "docx", "xlsx" => "xlsx", "pptx" => "pptx",
        "doc" or "docm" or "odt" or "rtf" or "epub" or "xls" or "xlsm" or "xlsb" or "ods" or "ppt" or "pps" or "pot" or "pptm" or "ppsx" or "ppsm" or "odp" => "document",
        "bib" or "tex" or "latex" or "nwk" or "newick" or "treefile" or "tre" or "txt" or "log" => "text",
        "html" or "htm" => "html", "ipynb" => "notebook", "json" => "json", _ => null
    };
    private static int FileOwner(ConversationItem[] items, int source)
    {
        for (var i = source + 1; i < items.Length; i++)
        {
            if (items[i].Role == "user") break;
            if (items[i].Role == "assistant" && !string.IsNullOrWhiteSpace(items[i].Text) && !Commentary(items, i)) return i;
        }
        return source;
    }
    private static bool Commentary(ConversationItem[] items, int index)
    {
        if (items[index].Text.StartsWith("Error: ", StringComparison.Ordinal)) return false;
        for (var i = index + 1; i < items.Length; i++)
        {
            var row = items[i];
            if (row.Role is "usage" or "compaction" or "file_changed" or "app_context" or "system" or "checkpoint" or "review_transition" or "reasoning"
                || row.Role == "assistant" && string.IsNullOrWhiteSpace(row.Text) || row.Role == "tool" && row.ToolName == "attempt_completion") continue;
            return !NativeTranscriptRows.IsContextTombstone(row) && (row.Role == "acp_tool" || row.Role == "tool"
                && row.ToolName is not ("monitor_run" or "wisp_monitor_run" or "generate_image" or "generate_video"));
        }
        return false;
    }
    public static string Directory(NativeGeneratedOutput output, string? projectRoot)
    {
        if (output.Path == null) return "";
        var path = output.Path.Trim().Replace('\\', '/'); var root = (projectRoot ?? "").Trim().Replace('\\', '/').TrimEnd('/');
        while (path.StartsWith("./", StringComparison.Ordinal)) path = path[2..];
        var comparison = root.Length > 1 && root[1] == ':' ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal;
        var inside = root.Length > 0 && path.StartsWith(root + "/", comparison);
        if (inside) path = path[(root.Length + 1)..];
        var slash = path.LastIndexOf('/'); if (slash <= 0) return "";
        var parent = path[..slash];
        var outside = !inside && (path.StartsWith('/') || path.Length > 1 && path[1] == ':' || path.Contains("://", StringComparison.Ordinal));
        return outside ? parent.Split('/').Last() : parent;
    }
}
