using System.Globalization;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeStorageEntry(string Key, ulong Bytes)
{
    public string LabelKey => "settings.storage." + (Key is "database" or "python" or "plugins" or "workspace" ? Key : "other");
    public string ColorToken => Key switch { "database" => "clay", "python" => "info", "plugins" => "warn", "workspace" => "ok", _ => "border-strong" };
}
public sealed record NativeStorageProject(string Id, string Name, string Path, ulong Bytes);
public sealed record NativeStorageSelection(string? ProjectId, string Path, ulong TotalBytes, IReadOnlyList<NativeStorageEntry> Entries);

/// The last successful scan and local selection for one window and database.
public sealed class NativeStorageBrowser
{
    public NativeStoragePresentation? Snapshot { get; private set; }
    public string? ProjectId { get; private set; }
    public void Select(string? id) => ProjectId = Snapshot?.Select(id).ProjectId;

    public async Task RefreshAsync(INativeSettingsClient client, string? scope, CancellationToken token)
    {
        var reply = await client.InvokeAsync("get_storage_usage", new(), scope, token);
        // A host may finish a directory scan after the user has left the page.
        token.ThrowIfCancellationRequested();
        if (reply is not JsonObject obj || obj["data_dir"] is not JsonValue dir || !dir.TryGetValue<string>(out _)
            || obj["total_bytes"] is not JsonValue size || !size.TryGetValue<ulong>(out _))
            throw new InvalidDataException("Invalid get_storage_usage response.");
        Snapshot = NativeStoragePresentation.From(obj);
        Select(ProjectId);
    }
}

/// Read-only projection of get_storage_usage. Browsing a project never activates it.
public sealed record NativeStoragePresentation(string DataDirectory, ulong TotalBytes,
    IReadOnlyList<NativeStorageEntry> Entries, IReadOnlyList<NativeStorageProject> Projects)
{
    public static NativeStoragePresentation From(JsonNode value) => new(
        Text(value, "data_dir"), Size(value["total_bytes"]),
        Rows(value["entries"]).Select(row => new NativeStorageEntry(Text(row, "key"), Size(row["bytes"]))).ToArray(),
        Rows(value["projects"]).Select(row => new NativeStorageProject(Text(row, "id"), Text(row, "name"), Text(row, "path"), Size(row["bytes"]))).ToArray());

    public NativeStorageSelection Select(string? projectId)
    {
        var project = projectId == null ? null : Projects.FirstOrDefault(row => row.Id == projectId);
        return project == null ? new(null, DataDirectory, TotalBytes, Entries)
            : new(project.Id, project.Path, project.Bytes, [new("workspace", project.Bytes)]);
    }

    public static string FormatBytes(ulong size) => size < 1024 ? $"{size} B"
        : size < 1024UL * 1024 ? (size / 1024d).ToString("0.0", CultureInfo.InvariantCulture) + " KB"
        : size < 1024UL * 1024 * 1024 ? (size / (1024d * 1024)).ToString("0.0", CultureInfo.InvariantCulture) + " MB"
        : (size / (1024d * 1024 * 1024)).ToString("0.0", CultureInfo.InvariantCulture) + " GB";
    public static double Fraction(ulong bytes, ulong total) => total == 0 ? 0 : Math.Clamp((double)bytes / total, 0, 1);
    public static int Columns(double width, int count) => !double.IsFinite(width) || width <= 0 ? 1
        : Math.Max(1, Math.Min(Math.Max(1, count), (int)Math.Min(int.MaxValue, Math.Floor((width + 8) / 288))));
    private static ulong Size(JsonNode? value) => value is JsonValue number && number.TryGetValue<ulong>(out var size) ? size : 0;
    private static string Text(JsonNode value, string key) => value[key]?.GetValue<string>() ?? "";
    private static IEnumerable<JsonObject> Rows(JsonNode? value) => (value as JsonArray)?.OfType<JsonObject>() ?? [];
}
