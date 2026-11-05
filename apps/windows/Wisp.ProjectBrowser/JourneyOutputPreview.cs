using System.Text;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Small, literal excerpts matching JourneyOutput in research_journey.rs.</summary>
public static class JourneyOutputPreview
{
    public static string Kind(string filename) => Path.GetExtension(filename).ToLowerInvariant() switch
    {
        ".png" or ".jpg" or ".jpeg" or ".gif" or ".webp" or ".svg" => "image",
        ".csv" or ".tsv" => "csv",
        ".md" or ".rmd" or ".qmd" or ".markdown" => "markdown",
        ".pdf" => "pdf", ".docx" => "docx", ".xlsx" => "xlsx", ".pptx" => "pptx",
        ".doc" or ".docm" or ".odt" or ".rtf" or ".epub" or ".xls" or ".xlsm" or ".xlsb" or ".ods" or ".ppt" or ".pps" or ".pot" or ".pptm" or ".ppsx" or ".ppsm" or ".odp" => "document",
        ".html" or ".htm" => "html", ".ipynb" => "notebook",
        ".pdb" or ".mol2" or ".cif" => "structure", ".mol" or ".sdf" => "molecule", ".smiles" or ".smi" => "smiles",
        ".json" => "json",
        ".r" or ".py" or ".pyw" or ".sh" or ".bash" or ".zsh" or ".js" or ".mjs" or ".cjs" or ".ts" or ".tsx" or ".mts" or ".cts" or ".rs" or ".go" or ".java" or ".kt" or ".kts" or ".swift" or ".rb" or ".pl" or ".pm" or ".lua" or ".php" or ".c" or ".h" or ".cpp" or ".cc" or ".cxx" or ".hpp" or ".hh" or ".cs" or ".m" or ".sql" or ".css" or ".scss" or ".less" or ".xml" or ".xsl" or ".rss" or ".yaml" or ".yml" or ".toml" or ".ini" or ".cfg" or ".conf" or ".diff" or ".patch" or ".jsonl" => "code",
        _ => NativeScientificPreview.Kind(filename) is { } kind ? kind : "text"
    };
    public static bool CanLoad(string kind) => kind is "image" or "csv" or "markdown" or "text" or "json" or "code";
    public static string Excerpt(string text) => string.Concat(text.EnumerateRunes().Take(600).Select(r => r.ToString()));
    public static string[][] Table(string text)
    {
        var lines = Excerpt(text).ReplaceLineEndings("\n").Split('\n').Take(7).ToArray();
        var tab = lines.FirstOrDefault()?.Contains('\t') == true;
        return lines.Select(line => line.Split(tab ? '\t' : ',').Take(4).Select(cell => tab ? cell : cell.Trim().Trim('"')).ToArray()).ToArray();
    }
    public static (string Text, bool Heading)[] Markdown(string text) => Excerpt(text).ReplaceLineEndings("\n").Split('\n')
        .Where(line => !string.IsNullOrWhiteSpace(line)).Take(7).Select(line => (line.StartsWith('#') ? line.TrimStart('#').Trim() : line, line.StartsWith('#'))).ToArray();
}

/// <summary>Visible cards share a bounded read queue; they never modify source selection.</summary>
public sealed class JourneyThumbnailLoader(INativeJourneyClient client, string projectId) : IDisposable
{
    private readonly SemaphoreSlim gate = new(4);
    private readonly CancellationTokenSource lifetime = new();
    public async Task<JourneyArtifact> LoadAsync(string version, CancellationToken token)
    {
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(token, lifetime.Token);
        await gate.WaitAsync(linked.Token);
        try
        {
            linked.Token.ThrowIfCancellationRequested();
            var result = await client.ThumbnailAsync(projectId, version, linked.Token);
            linked.Token.ThrowIfCancellationRequested();
            if (result.VersionId != version) throw new InvalidDataException("Journey preview version mismatch");
            return result;
        }
        finally { gate.Release(); }
    }
    public void Dispose() => lifetime.Cancel();
}
