using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeArtifactCatalogTests
{
    public static async Task RunAsync()
    {
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> {
            ["en"] = new() { ["artifact.group.root"] = "Project root", ["artifact.group.table"] = "Tables",
                ["artifact.group.csv"] = "CSV", ["artifact.group.fasta"] = "FASTA", ["artifact.group.latex"] = "Equations",
                ["artifact.meta.table"] = "{rows} rows · {cols} columns", ["artifact.meta.fasta"] = "{seqs} sequences",
                ["artifact.meta.file"] = "{kind} file" },
            ["zh"] = new() { ["artifact.group.table"] = "表格", ["artifact.meta.table"] = "{rows} 行 · {cols} 列" } }, "en");
        static NativeGeneratedOutput File(string id, string path, bool superseded = false) => new(id, "p", "s", "row", 0,
            path.Split('/', '\\').Last(), "csv", path, null, null, superseded);
        var table = new NativeGeneratedOutput("table", "p", "s", "row", 0, "Table 1", "table", null, null, new(["a", "b"], [["1", "2"]], false));
        var generated = new[] { File("old", "results/a.csv"), File("new", @"C:\PROJECT\results\a.csv"),
            File("hidden", "results/.cache/tmp.csv"), File("removed", "results/removed.csv"), File("obsolete", "old.csv", true), table,
            table with { Id = "csv", Name = "data-1.csv", Kind = "csv" },
            table with { Id = "fasta", Name = "alignment-1.fasta", Kind = "fasta", Table = null, Text = ">a\nACT\n>b\nACT" },
            table with { Id = "latex", Name = "Equation 1", Kind = "latex", Table = null, Text = "x" } };
        NativePanelArtifact[] registered = [new("duplicate", "a.csv", "text/csv", ".wisp/snapshot/a.csv", null, 1, "results/A.csv"),
            new("snapshot", "report.csv", "text/csv", ".wisp/snapshot/report.csv", null, 2, "reports/report.csv"),
            new("remote", "remote.csv", "text/csv", "ssh://host/scratch/remote.csv", null, 3, null),
            new("hidden-db", "secret.csv", "text/csv", ".wisp/snapshot/secret.csv", null, 4, "results/.hidden/secret.csv"),
            new("root", "README.md", "text/markdown", "C:/project/README.md", null, 5, null)];
        var current = NativeArtifactCatalog.Collect(generated, registered, "C:/project", locale,
            new HashSet<string> { "results/removed.csv", "ssh://host/scratch/remote.csv" });
        Check(current.Length == 8 && current[0].Id == "new", "latest normalized file identity wins while hidden, missing and superseded references disappear");
        Check(current.All(item => item.Registered?.Id != "duplicate") && current.Any(item => item.Registered?.Id == "remote"),
            "registered paths deduplicate against transcript files, but remote registered references bypass scraped-path missing checks");
        var snapshot = current.Single(item => item.Registered?.Id == "snapshot");
        Check(snapshot.Group == "reports/" && snapshot.DisplayPath == "reports/report.csv" && snapshot.Registered?.Path == ".wisp/snapshot/report.csv",
            "visible logical paths control grouping while the original registered id/path stays available to the viewer");
        var groups = NativeArtifactCatalog.Groups(current, locale);
        Check(groups.Select(group => group.Key).SequenceEqual(new[] { ".", "reports/", "results/", "scratch/", "@csv", "@fasta", "@latex", "@table" }),
            "project-relative directories sort first, followed by inline kinds in canonical order");
        Check(current.Single(item => item.Id == "table").Metadata == "1 rows · 2 columns"
            && current.Single(item => item.Id == "fasta").Metadata == "2 sequences", "inline metadata uses real dimensions and sequence count");
        Check(NativeArtifactCatalog.Identity("./RESULTS//a.csv", "C:/project") == "results/a.csv"
            && NativeArtifactCatalog.Identity("results/A.csv", "/work/project") != NativeArtifactCatalog.Identity("results/a.csv", "/work/project"),
            "Windows workspace identities fold case; Unix paths remain case sensitive");
        Check(NativeArtifactCatalog.Hidden("C:/project/.hidden/../report.csv", "C:/project")
            && !NativeArtifactCatalog.Hidden("../report.csv", "C:/project"), "hidden path components are excluded without treating parent components as dotfiles");
        var preview = new NativeGeneratedPreviewModel(); preview.Bind(current.Where(item => item.Output != null).Select(item => item.Output!).ToArray());
        await preview.OpenAsync("table");
        preview.Bind(current.Where(item => item.Output != null).Select(item => item.Output! with { }).ToArray());
        Check(preview.Selected?.Id == "table", "replacing an unchanged catalog preserves the selected preview");
        locale.Preview("zh"); var translated = NativeArtifactCatalog.Collect(generated, registered, "C:/project", locale);
        preview.Bind(translated.Where(item => item.Output != null).Select(item => item.Output!).ToArray());
        Check(preview.Selected?.Id == "table" && NativeArtifactCatalog.Groups(translated, locale).Single(group => group.Key == "@table").Label == "表格",
            "group labels can translate without changing selected source identity");
        preview.Bind(Array.Empty<NativeGeneratedOutput>());
        Check(preview.Selected == null && NativeArtifactCatalog.Groups([], locale).Length == 0, "empty catalogs clear selection and yield no placeholder groups");
    }
}
