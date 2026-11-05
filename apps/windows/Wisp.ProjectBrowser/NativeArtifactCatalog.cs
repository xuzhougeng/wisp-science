using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeArtifactEntry(string Id, string Name, string Kind, string Group, string Metadata,
    string? DisplayPath, NativeGeneratedOutput? Output, NativePanelArtifact? Registered);
public sealed record NativeArtifactGroup(string Key, string Label, NativeArtifactEntry[] Items);

/// <summary>WebView current_artifacts projection: newest visible file references,
/// then registered artifacts; directory groups precede inline kinds.</summary>
public static class NativeArtifactCatalog
{
    public static NativeArtifactEntry[] Collect(IEnumerable<NativeGeneratedOutput> generated,
        IEnumerable<NativePanelArtifact> registered, string projectRoot, NativeLocalization locale,
        IReadOnlySet<string>? missingPaths = null)
    {
        var seen = new HashSet<string>(StringComparer.Ordinal); var result = new List<NativeArtifactEntry>();
        foreach (var output in generated.Reverse())
        {
            if (output.Superseded) continue;
            if (output.Path is { } path && (missingPaths?.Contains(path) == true || Hidden(path, projectRoot)
                || !seen.Add(Identity(path, projectRoot)))) continue;
            result.Add(Entry(output.Id, output.Name, output.Kind, output.Path, output, null));
        }
        result.Reverse();
        foreach (var artifact in registered)
        {
            var displayPath = artifact.LogicalPath ?? artifact.Location ?? artifact.Path;
            if (Hidden(displayPath, projectRoot) || !seen.Add(Identity(displayPath, projectRoot))) continue;
            var kind = NativeGeneratedArtifacts.FileKind(artifact.Location ?? artifact.Path)
                ?? NativeGeneratedArtifacts.FileKind(artifact.Name)
                ?? (NativeCodePreview.FileLanguage(artifact.Location ?? artifact.Path, artifact.Name, "") != null ? "code" : "file");
            result.Add(Entry("artifact:" + artifact.Id, artifact.Name, kind, displayPath, null, artifact));
        }
        return result.ToArray();

        NativeArtifactEntry Entry(string id, string name, string kind, string? path, NativeGeneratedOutput? output, NativePanelArtifact? registeredItem)
        {
            var group = path == null ? "@" + kind : Directory(path, projectRoot);
            var metadata = output?.Table is { } table
                ? locale.Format("artifact.meta.table", ("rows", table.Rows.Length.ToString()), ("cols", table.Headers.Length.ToString()))
                : path == null && kind == "latex" ? locale.Text("artifact.latex")
                : path == null && kind == "fasta" ? locale.Format("artifact.meta.fasta", ("seqs", Math.Max(1, (output?.Text ?? "").Split('\n').Count(line => line.StartsWith('>'))).ToString()))
                : kind is "fasta" or "msa" ? locale.Text("artifact.kind." + kind)
                : path?.Replace('\\', '/').Split('/') is { Length: > 1 } parts && parts[^2].Length > 0 ? parts[^2] + "/"
                : locale.Format("artifact.meta.file", ("kind", kind));
            return new(id, name, kind, group, metadata, path, output, registeredItem);
        }
    }
    public static NativeArtifactGroup[] Groups(IEnumerable<NativeArtifactEntry> entries, NativeLocalization locale) =>
        entries.GroupBy(item => item.Group).OrderBy(group => group.Key.StartsWith('@') ? 1 : 0)
            .ThenBy(group => group.Key, StringComparer.Ordinal)
            .Select(group => new NativeArtifactGroup(group.Key, Label(group.Key, locale), group.ToArray())).ToArray();
    public static string Label(string group, NativeLocalization locale)
    {
        if (group == ".") return locale.Text("artifact.group.root");
        if (!group.StartsWith('@')) return group;
        var kind = group[1..]; var key = kind is "markdown" or "code" or "notebook" ? "text" : kind;
        return key is "table" or "latex" or "csv" or "fasta" or "msa" or "text" ? locale.Text("artifact.group." + key) : kind;
    }
    public static string Identity(string path, string projectRoot)
    {
        static string Slash(string value)
        {
            value = value.Trim().Replace('\\', '/');
            while (value.Contains("//", StringComparison.Ordinal)) value = value.Replace("//", "/");
            while (value.StartsWith("./", StringComparison.Ordinal)) value = value[2..];
            return value.TrimEnd('/');
        }
        path = Slash(path); var root = Slash(projectRoot);
        if (root.Length > 1 && root[1] == ':') { path = path.ToLowerInvariant(); root = root.ToLowerInvariant(); }
        return root.Length > 0 && path.StartsWith(root + "/", StringComparison.Ordinal) ? path[(root.Length + 1)..] : path;
    }
    public static bool Hidden(string path, string projectRoot) => Identity(path, projectRoot).Split('/')
        .Any(part => part.StartsWith('.') && part is not ("." or ".."));
    private static string Directory(string path, string root)
    {
        var output = new NativeGeneratedOutput("", "", "", "", 0, "", "", path, null, null);
        var directory = NativeGeneratedArtifacts.Directory(output, root);
        return directory.Length == 0 ? "." : directory + "/";
    }
}
