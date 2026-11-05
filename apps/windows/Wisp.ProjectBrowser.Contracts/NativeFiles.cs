using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

public interface INativeFilesEnvelope
{
    string Schema { get; }
    string ProjectId { get; }
    string? SessionId { get; }
}
public sealed record NativeFileLocation(string ContextId, string Label, string Kind);
public sealed record NativeFileLocations(string Schema, string ProjectId, string? SessionId, string LocalRoot,
    bool ReadOnly, NativeFileLocation[] Locations) : INativeFilesEnvelope;
public sealed record NativeFileDirectory(string Schema, string ProjectId, string? SessionId, string ContextId,
    string Path, NativePanelFile[] Entries) : INativeFilesEnvelope;
public sealed record NativeFileSearchHit(string Path, string Name, bool IsDir, ulong Size);
public sealed record NativeFilePathPair(string RequestedPath, string RelativePath, string AbsolutePath);
public sealed record NativeFilePaths(string Schema, string ProjectId, string? SessionId, NativeFilePathPair[] Paths) : INativeFilesEnvelope;
public sealed record NativeFileDrag(string ProjectId, string? SessionId, string Path)
{
    public const string Format = "application/x-wisp-native-workspace-path";
    public bool Valid => !string.IsNullOrWhiteSpace(ProjectId) && (SessionId == null || !string.IsNullOrWhiteSpace(SessionId)) && NativeFilePathRules.Relative(Path);
}
public static class NativeFilePathRules
{
    // Clipboard references are message text only. They never authorize a scoped
    // Files read/write; those commands still require Relative and ExactUnderRoot.
    public static bool WindowsClipboardPath(string? path)
    {
        if (string.IsNullOrWhiteSpace(path) || path.Length > 32767 || path.Any(char.IsControl)) return false;
        var value = path.Replace('\\', '/');
        string tail;
        if (value.Length >= 3 && char.IsAsciiLetter(value[0]) && value[1] == ':' && value[2] == '/') tail = value[3..];
        else if (value.StartsWith("//") && !value.StartsWith("//?/") && !value.StartsWith("//./"))
        {
            tail = value[2..];
            if (tail.TrimEnd('/').Split('/').Length < 2) return false;
        }
        else return false;
        return !tail.Contains(':') && !tail.Contains('?') && !tail.Contains('*')
            && (tail.Length == 0 || !tail.TrimEnd('/').Split('/').Any(part => part is "" or "." or ".."));
    }
    public static bool Relative(string? path) => !string.IsNullOrWhiteSpace(path) && !path.Contains('\\') && !path.Contains(':')
        && !path.Any(char.IsControl) && !path.Split('/').Any(p => p is "" or "." or "..");
    private static string Normalize(string path)
    {
        var value = path.Replace('\\', '/');
        if (value.StartsWith("//?/UNC/", StringComparison.OrdinalIgnoreCase)) value = "//" + value[8..];
        else if (value.StartsWith("//?/", StringComparison.Ordinal)) value = value[4..];
        return value.TrimEnd('/');
    }
    public static bool ExactUnderRoot(string absolute, string root, string relative)
    {
        if (!Relative(relative) || string.IsNullOrWhiteSpace(root) || string.IsNullOrWhiteSpace(absolute)) return false;
        var canonicalRoot = Normalize(root);
        var windows = canonicalRoot.StartsWith("//") || canonicalRoot.Length > 1 && canonicalRoot[1] == ':';
        if (!windows && !root.StartsWith('/')) return false;
        return string.Equals(Normalize(absolute), canonicalRoot + "/" + relative, windows ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal);
    }
}
public sealed record NativeFilePreview(string Schema, string ProjectId, string? SessionId, string ContextId,
    string RequestedPath, NativePanelFileContent Content) : INativeFilesEnvelope;
public sealed record NativeFileTransferItem(string SourcePath, string? DestinationPath, string? RunId, string Status, string? Error);
public sealed record NativeFileTransfer(string Schema, string ProjectId, string? SessionId, string ContextId,
    string Path, NativeFileTransferItem[] Items) : INativeFilesEnvelope;

public interface INativeFilesClient
{
    Task<NativeFileLocations> LocationsAsync(string project, string? session, CancellationToken token = default);
    Task<NativeFileDirectory> DirectoryAsync(string project, string? session, string context, string path, CancellationToken token = default);
    Task<NativePanelFileContent> ReadAsync(string project, string? session, string context, string path, CancellationToken token = default);
    Task<NativeFileTransfer> UploadAsync(string project, string? session, string context, string path, IReadOnlyList<string> sources, CancellationToken token = default);
    Task<NativeFileSearchHit[]> SearchAsync(string project, string? session, string query, CancellationToken token = default);
    Task<NativeFileTransfer> DownloadAsync(string project, string? session, string context, string path, string destination, CancellationToken token = default);
    Task<NativeRun> TransferRunAsync(string project, string? session, string context, string runId, CancellationToken token = default);
    Task<NativeFilePaths> PathsAsync(string project, string? session, IReadOnlyList<string> paths, string root, CancellationToken token = default);
}

/// <summary>Uses the shared wisp.native-files.v1 contract; every response is checked against its caller's scope.</summary>
public sealed class NativeFilesClient(INativeSettingsClient transport) : INativeFilesClient
{
    public const string SchemaId = "wisp.native-files.v1";
    public async Task<NativeFilePaths> PathsAsync(string project, string? session, IReadOnlyList<string> paths, string root, CancellationToken token = default)
    {
        if (paths.Count == 0 || paths.Distinct().Count() != paths.Count || paths.Any(p => !NativeFilePathRules.Relative(p)))
            throw new ArgumentException("Choose unique project paths", nameof(paths));
        var result = await Call<NativeFilePaths>("paths", project, session, new() { ["context_id"] = "local",
            ["paths"] = new JsonArray(paths.Select(p => (JsonNode?)JsonValue.Create(p)).ToArray()) }, token);
        if (result.Paths == null || !result.Paths.Select(p => p.RequestedPath).SequenceEqual(paths)
            || result.Paths.Any(p => !NativeFilePathRules.ExactUnderRoot(p.AbsolutePath, root, p.RelativePath)))
            throw new InvalidDataException("Project path response identity mismatch");
        return result;
    }
    public async Task<NativeFileSearchHit[]> SearchAsync(string project, string? session, string query, CancellationToken token = default)
    {
        var result = (await transport.InvokeAsync(session == null ? "native_project_file_search" : "native_conversation_panel_searchfiles",
            new() { ["session_id"] = session, ["query"] = query }, project, token).ConfigureAwait(false))
            ?.Deserialize<NativeFileSearchHit[]>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing file search response");
        if (result.Length > 200 || result.Select(h => h.Path).Distinct().Count() != result.Length
            || result.Any(h => string.IsNullOrWhiteSpace(h.Name) || string.IsNullOrWhiteSpace(h.Path)
                || h.Path.Contains('\\') || h.Path.Contains(':') || h.Path.Any(char.IsControl)
                || h.Path.Split('/').Any(p => p is "" or "." or "..") || h.Path.Split('/').Last() != h.Name))
            throw new InvalidDataException("Invalid project file search response");
        return result;
    }
    public async Task<NativeFileTransfer> DownloadAsync(string project, string? session, string context, string path, string destination, CancellationToken token = default)
    {
        if (context == "local" || string.IsNullOrWhiteSpace(destination)) throw new ArgumentException("Choose a remote file and a save destination");
        var result = await Call<NativeFileTransfer>("download", project, session, new() {
            ["context_id"] = context, ["path"] = path, ["destination_path"] = destination }, token);
        if (result.ContextId != context || result.Path != path || result.Items is not { Length: 1 }
            || result.Items[0].SourcePath != path || result.Items[0].DestinationPath != destination
            || string.IsNullOrWhiteSpace(result.Items[0].RunId) || result.Items[0].Status is not ("submitted" or "queued" or "running" or "succeeded" or "failed"))
            throw new InvalidDataException("Download result was not confirmed; refresh before trying again");
        return result;
    }
    public async Task<NativeRun> TransferRunAsync(string project, string? session, string context, string runId, CancellationToken token = default)
    {
        var run = session == null ? (await transport.InvokeAsync("native_project_file_run", new() { ["run_id"] = runId, ["context_id"] = context }, project, token))?.Deserialize<NativeRun>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing project transfer") : await new NativeContextActivityClient(transport).ReadRunAsync(project, session, runId, token);
        if (run.Id != runId || run.ContextId != context || run.FrameId != session || run.Kind != "file_transfer")
            throw new InvalidDataException("File transfer identity mismatch");
        return run;
    }
    private async Task<T> Call<T>(string action, string project, string? session, JsonObject args, CancellationToken token) where T : class, INativeFilesEnvelope
    {
        args["session_id"] = session;
        var result = (await transport.InvokeAsync((session == null ? "native_project_file_" : "native_conversation_panel_file_") + action, args, project, token).ConfigureAwait(false))
            ?.Deserialize<T>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing file browser response");
        if (result.Schema != SchemaId || result.ProjectId != project || result.SessionId != session)
            throw new InvalidDataException("File browser response scope mismatch");
        return result;
    }
    public async Task<NativeFileLocations> LocationsAsync(string project, string? session, CancellationToken token = default)
    {
        var result = await Call<NativeFileLocations>("locations", project, session, new(), token);
        if (result.Locations is null || result.Locations.Count(l => l.ContextId == "local" && l.Kind == "local") != 1
            || result.Locations.Any(l => string.IsNullOrWhiteSpace(l.ContextId) || l.Kind is not ("local" or "ssh"))
            || result.Locations.Select(l => l.ContextId).Distinct().Count() != result.Locations.Length)
            throw new InvalidDataException("Invalid file locations");
        return result;
    }
    public async Task<NativeFileDirectory> DirectoryAsync(string project, string? session, string context, string path, CancellationToken token = default)
    {
        var result = await Call<NativeFileDirectory>("directory", project, session, new() { ["context_id"] = context, ["path"] = path }, token);
        // The host canonicalizes '~' and local paths; the returned path is the next navigation base.
        if (result.ContextId != context || string.IsNullOrWhiteSpace(result.Path) || result.Entries is null)
            throw new InvalidDataException("Invalid file directory response");
        return result;
    }
    public async Task<NativePanelFileContent> ReadAsync(string project, string? session, string context, string path, CancellationToken token = default)
    {
        var result = await Call<NativeFilePreview>("read", project, session, new() { ["context_id"] = context, ["path"] = path,
            ["render_pdf"] = true, ["render_office"] = true }, token);
        if (result.ContextId != context || result.RequestedPath != path || result.Content is null)
            throw new InvalidDataException("File preview identity mismatch");
        return result.Content;
    }
    public async Task<NativeFileTransfer> UploadAsync(string project, string? session, string context, string path, IReadOnlyList<string> sources, CancellationToken token = default)
    {
        if (sources.Count == 0 || sources.Any(string.IsNullOrWhiteSpace) || sources.Distinct().Count() != sources.Count)
            throw new ArgumentException("Choose unique source files", nameof(sources));
        var result = await Call<NativeFileTransfer>("upload", project, session, new() { ["context_id"] = context, ["path"] = path,
            ["source_paths"] = new JsonArray(sources.Select(s => (JsonNode?)JsonValue.Create(s)).ToArray()) }, token);
        if (result.ContextId != context || result.Path != path || result.Items is null || result.Items.Length != sources.Count
            || !result.Items.Select(i => i.SourcePath).Order().SequenceEqual(sources.Order())
            || result.Items.Any(i => i.Status is not ("succeeded" or "failed" or "submitted" or "running" or "queued")))
            throw new InvalidDataException("File transfer result was not confirmed; refresh before trying again");
        return result;
    }
}
