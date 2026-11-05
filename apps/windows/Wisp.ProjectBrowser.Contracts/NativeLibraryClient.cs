using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record LibraryItemSummary(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("language")] string? Language,
    [property: JsonPropertyName("code_preview")] string CodePreview,
    [property: JsonPropertyName("source_project_id")] string SourceProjectId,
    [property: JsonPropertyName("source_project_name")] string SourceProjectName,
    [property: JsonPropertyName("source_session_id")] string SourceSessionId,
    [property: JsonPropertyName("source_session_title")] string SourceSessionTitle,
    [property: JsonPropertyName("source_path")] string? SourcePath,
    [property: JsonPropertyName("created_at")] long CreatedAt);

// Projections of wisp_dto::LibraryItemDetail and LibraryItemVersion. Detail is
// flattened on the wire, including the complete snapshot rather than a preview.
public sealed record LibraryItemDetail(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("language")] string? Language,
    [property: JsonPropertyName("code")] string Code,
    [property: JsonPropertyName("content_type")] string? ContentType,
    [property: JsonPropertyName("source_project_id")] string SourceProjectId,
    [property: JsonPropertyName("source_project_name")] string SourceProjectName,
    [property: JsonPropertyName("source_session_id")] string SourceSessionId,
    [property: JsonPropertyName("source_session_title")] string SourceSessionTitle,
    [property: JsonPropertyName("source_path")] string? SourcePath,
    [property: JsonPropertyName("created_at")] long CreatedAt,
    [property: JsonPropertyName("base64")] string? Base64)
{
    public LibraryItemSummary Summary => new(Id, Kind, Title, Language, "", SourceProjectId,
        SourceProjectName, SourceSessionId, SourceSessionTitle, SourcePath, CreatedAt);
}

public sealed record LibraryItemVersion(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("item_id")] string ItemId,
    [property: JsonPropertyName("version_number")] long VersionNumber,
    [property: JsonPropertyName("parent_version_id")] string? ParentVersionId,
    [property: JsonPropertyName("language")] string? Language,
    [property: JsonPropertyName("code")] string Code,
    [property: JsonPropertyName("origin")] string Origin,
    [property: JsonPropertyName("created_at")] long CreatedAt);

/// <summary>
/// App-global library reads and immutable version edits. Calls carry no project
/// id and are never replayed: a lost mutation may already have succeeded.
/// </summary>
public interface INativeLibraryClient
{
    Task<IReadOnlyList<LibraryItemSummary>> SearchAsync(string query, string? kind, CancellationToken cancellationToken = default);
    Task<bool> DeleteAsync(string id, CancellationToken cancellationToken = default);
    Task<LibraryItemDetail> GetAsync(string id, CancellationToken cancellationToken = default);
    Task<IReadOnlyList<LibraryItemVersion>> VersionsAsync(string id, CancellationToken cancellationToken = default);
    Task<LibraryItemVersion> SaveCodeAsync(string id, string code, CancellationToken cancellationToken = default);
}

public sealed class NativeLibraryClient(INativeSettingsClient transport) : INativeLibraryClient
{
    private async Task<T> Read<T>(string command, JsonObject args, CancellationToken cancellationToken)
    {
        var node = await transport.InvokeAsync(command, args, null, cancellationToken).ConfigureAwait(false);
        return node == null ? throw new InvalidDataException("Missing library response")
            : node.Deserialize<T>() ?? throw new InvalidDataException("Missing library response");
    }
    public Task<LibraryItemDetail> GetAsync(string id, CancellationToken cancellationToken = default)
        => Read<LibraryItemDetail>("native_library_get", new() { ["id"] = id }, cancellationToken);
    public async Task<IReadOnlyList<LibraryItemVersion>> VersionsAsync(string id, CancellationToken cancellationToken = default)
        => await Read<List<LibraryItemVersion>>("native_library_versions", new() { ["id"] = id }, cancellationToken).ConfigureAwait(false);
    public Task<LibraryItemVersion> SaveCodeAsync(string id, string code, CancellationToken cancellationToken = default)
        => Read<LibraryItemVersion>("native_library_update_code", new() { ["id"] = id, ["code"] = code }, cancellationToken);
    public async Task<IReadOnlyList<LibraryItemSummary>> SearchAsync(string query, string? kind, CancellationToken cancellationToken = default)
    {
        var args = new JsonObject { ["query"] = query };
        if (!string.IsNullOrEmpty(kind)) args["kind"] = kind;
        var node = await transport.InvokeAsync("native_library_search", args, null, cancellationToken).ConfigureAwait(false)
            ?? throw new InvalidDataException("Missing library search result");
        return node.Deserialize<List<LibraryItemSummary>>() ?? throw new InvalidDataException("Missing library search result");
    }

    public async Task<bool> DeleteAsync(string id, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_library_delete", new JsonObject { ["id"] = id }, null, cancellationToken).ConfigureAwait(false)
            ?? throw new InvalidDataException("Missing library delete result");
        return node.GetValue<bool>();
    }
}
