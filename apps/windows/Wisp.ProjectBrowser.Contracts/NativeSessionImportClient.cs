using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record ArchivePreviewLine(string Role, string Text);
public sealed record NativeArchivePreview(string Schema, string ProjectId, string ArchivePath, string Sha256,
    string SourceSessionId, string Title, int MessageCount, string[] Artifacts, ArchivePreviewLine[] Messages,
    string? ExistingSessionId, string State);
public sealed record NativeArchiveImportResult(string Schema, string ProjectId, string SourceSessionId,
    string FrameId, string Status, int MessageCount, int ArtifactCount, string[] MissingArtifacts);

public interface INativeSessionImportClient
{
    Task<NativeArchivePreview> PreviewAsync(string project, string path, CancellationToken cancellationToken = default);
    Task<NativeArchiveImportResult> ImportAsync(NativeArchivePreview reviewed, CancellationToken cancellationToken = default);
}

public sealed class NativeSessionImportClient(INativeSettingsClient transport) : INativeSessionImportClient
{
    public const string Schema = "wisp.native-session-import.v1";
    public static NativeArchivePreview DecodePreview(JsonNode? node, string project, string path)
    {
        var result = node?.Deserialize<NativeArchivePreview>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing archive preview");
        if (result.Schema != Schema || result.ProjectId != project || result.ArchivePath != path
            || string.IsNullOrWhiteSpace(result.SourceSessionId) || result.Sha256?.Length != 64
            || !result.Sha256.All(char.IsAsciiHexDigit) || result.MessageCount <= 0 || result.Title == null
            || result.Artifacts == null || result.Artifacts.Length > 4096 || result.Artifacts.Any(a => a == null)
            || result.Messages == null || result.Messages.Length > 4 || result.Messages.Any(m => m == null || m.Role is not ("user" or "assistant") || m.Text == null)
            || result.State is not ("new" or "updatable" or "imported")
            || (result.State == "new" ? result.ExistingSessionId != null : string.IsNullOrWhiteSpace(result.ExistingSessionId)))
            throw new InvalidDataException("Archive preview identity or content is invalid");
        return result;
    }
    public static NativeArchiveImportResult DecodeResult(JsonNode? node, NativeArchivePreview reviewed)
    {
        var result = node?.Deserialize<NativeArchiveImportResult>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing import result");
        if (result.Schema != Schema || result.ProjectId != reviewed.ProjectId || result.SourceSessionId != reviewed.SourceSessionId
            || string.IsNullOrWhiteSpace(result.FrameId) || result.Status is not ("imported" or "updated" or "skipped")
            || (reviewed.ExistingSessionId != null && result.FrameId != reviewed.ExistingSessionId)
            || result.MessageCount != reviewed.MessageCount || result.ArtifactCount < 0 || result.ArtifactCount > reviewed.Artifacts.Length
            || result.MissingArtifacts == null || result.MissingArtifacts.Any(a => a == null))
            throw new InvalidDataException("Import result does not match the reviewed archive and destination");
        return result;
    }
    public async Task<NativeArchivePreview> PreviewAsync(string project, string path, CancellationToken cancellationToken = default) =>
        DecodePreview(await transport.InvokeAsync("native_session_archive_preview", new() { ["archive_path"] = path }, project, cancellationToken), project, path);
    public async Task<NativeArchiveImportResult> ImportAsync(NativeArchivePreview reviewed, CancellationToken cancellationToken = default) =>
        DecodeResult(await transport.InvokeAsync("native_session_archive_import", new() { ["archive_path"] = reviewed.ArchivePath,
            ["sha256"] = reviewed.Sha256, ["source_session_id"] = reviewed.SourceSessionId }, reviewed.ProjectId, cancellationToken), reviewed);
}
