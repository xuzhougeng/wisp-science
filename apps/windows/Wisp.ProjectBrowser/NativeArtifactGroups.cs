using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Artifact panel grouping: type labels with counts, images first,
/// matching the macOS alignment plan's D1 information structure.</summary>
public static class NativeArtifactGroups
{
    public sealed record Group(string Label, int Rank, List<NativePanelArtifact> Items);

    /// <summary>Filter by name, then group by friendly type label. Groups keep a
    /// stable category rank (images, data, documents, text, other) and sort by
    /// label inside each rank; items keep the host order.</summary>
    public static IReadOnlyList<Group> Collect(IEnumerable<NativePanelArtifact> artifacts, string query = "")
    {
        var groups = new List<Group>();
        foreach (var artifact in artifacts)
        {
            if (query.Length > 0 && !artifact.Name.Contains(query, StringComparison.CurrentCultureIgnoreCase)) continue;
            var (label, rank) = Label(artifact.Kind);
            var group = groups.FirstOrDefault(g => g.Label == label);
            if (group is null) groups.Add(group = new Group(label, rank, []));
            group.Items.Add(artifact);
        }
        return groups
            .OrderBy(g => g.Rank)
            .ThenBy(g => g.Label, StringComparer.CurrentCultureIgnoreCase)
            .ToList();
    }

    /// <summary>Friendly zh label for a MIME kind; unknown kinds pass through.</summary>
    public static (string Label, int Rank) Label(string? kind)
    {
        var value = (kind ?? "").Trim().ToLowerInvariant();
        var slash = value.IndexOf('/');
        var type = slash > 0 ? value[..slash] : value;
        var subtype = slash > 0 ? value[(slash + 1)..] : "";
        if (type == "image") return ($"图片 · {subtype.ToUpperInvariant()}", 0);
        if (value == "application/pdf") return ("PDF 文档", 1);
        if (value.StartsWith("application/vnd.openxmlformats-officedocument") || value is "application/msword" or "application/vnd.ms-excel" or "application/vnd.ms-powerpoint")
            return ("Office 文档", 2);
        if (value is "application/zip" or "application/gzip" or "application/x-tar" or "application/x-7z-compressed")
            return ("压缩包", 3);
        if (type == "text") return (subtype switch
        {
            "plain" => "纯文本",
            "markdown" => "Markdown",
            "csv" => "CSV",
            "html" => "HTML",
            "x-r" => "R 代码",
            "x-python" or "x-python3" => "Python 代码",
            "x-sh" or "x-shellscript" => "Shell",
            "" => "文本",
            _ => "文本 · " + subtype,
        }, 4);
        if (type == "application") return (subtype.Length > 0 ? "数据 · " + subtype : "其他", 5);
        return (value.Length > 0 ? value : "其他", 6);
    }
}
