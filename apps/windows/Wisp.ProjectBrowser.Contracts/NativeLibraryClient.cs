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

/// <summary>
/// App-global library search and delete. Calls carry no project id and are not
/// retried: a lost delete may already have removed the row.
/// </summary>
public interface INativeLibraryClient
{
    Task<IReadOnlyList<LibraryItemSummary>> SearchAsync(string query, string? kind, CancellationToken cancellationToken = default);
    Task<bool> DeleteAsync(string id, CancellationToken cancellationToken = default);
}

public sealed class NativeLibraryClient(INativeSettingsClient transport) : INativeLibraryClient
{
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
