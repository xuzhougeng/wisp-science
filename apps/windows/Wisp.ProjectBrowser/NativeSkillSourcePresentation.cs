using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// <summary>Presentation of the existing shared SkillInstallSource payload.</summary>
public static class NativeSkillSourcePresentation
{
    public sealed record Entry(string Label, string Value, string? Url = null, bool Code = false);
    public static JsonObject? Parse(JsonNode? value)
    {
        if (value == null) return null;
        if (value is not JsonObject source || new[] { "repository", "source_url", "git_ref", "commit", "package_path" }
            .Any(key => source[key] is not JsonValue field || !field.TryGetValue<string>(out _)))
            throw new InvalidDataException("Invalid skill installation source.");
        return (JsonObject)source.DeepClone();
    }
    public static Entry[] Entries(JsonObject source)
    {
        var repository = NativeSkillsModel.S(source, "repository"); var commit = NativeSkillsModel.S(source, "commit");
        var path = NativeSkillsModel.S(source, "package_path");
        var encoded = string.Join('/', path.Split('/').Select(Uri.EscapeDataString));
        var root = "https://github.com/" + string.Join('/', repository.Split('/').Select(Uri.EscapeDataString));
        string? Safe(string url) => Uri.TryCreate(url, UriKind.Absolute, out var uri) && uri.Scheme is "http" or "https" ? url : null;
        return [
            new("GitHub", repository, Safe($"{root}/tree/{Uri.EscapeDataString(commit)}/{encoded}")),
            new("Source URL", NativeSkillsModel.S(source, "source_url"), Safe(NativeSkillsModel.S(source, "source_url"))),
            new("SKILL.md", "SKILL.md", Safe($"{root}/blob/{Uri.EscapeDataString(commit)}/{(encoded.Length == 0 ? "" : encoded + "/")}SKILL.md")),
            new("Ref", NativeSkillsModel.S(source, "git_ref")), new("Commit", commit, Code: true), new("Package", path.Length == 0 ? "/" : path)
        ];
    }
}
