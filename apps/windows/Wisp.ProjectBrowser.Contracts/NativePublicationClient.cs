using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativePublication(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("description")] string Description);

public sealed record NativePublicationRevision(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("label")] string Label,
    [property: JsonPropertyName("state")] string State);

public sealed record NativePublicationItem(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("ordinal")] long Ordinal);

public sealed record NativePublicationWorkspace(
    [property: JsonPropertyName("publications")] IReadOnlyList<NativePublication> Publications,
    [property: JsonPropertyName("publication")] NativePublication? Publication,
    [property: JsonPropertyName("revision")] NativePublicationRevision? Revision,
    [property: JsonPropertyName("items")] IReadOnlyList<NativePublicationItem> Items);

/// <summary>
/// Publication workspace for one explicit project. A lost create is not retried.
/// </summary>
public interface INativePublicationClient
{
    Task<NativePublicationWorkspace> ReadAsync(string projectId, CancellationToken cancellationToken = default);
    Task<NativePublicationWorkspace> CreateAsync(string projectId, string title, string description, string revisionLabel, CancellationToken cancellationToken = default);
}

public sealed class NativePublicationClient(INativeSettingsClient transport) : INativePublicationClient
{
    public async Task<NativePublicationWorkspace> ReadAsync(string projectId, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_publication_workspace", new JsonObject(), projectId, cancellationToken).ConfigureAwait(false)
            ?? throw new InvalidDataException("Missing publication workspace");
        return node.Deserialize<NativePublicationWorkspace>() ?? throw new InvalidDataException("Missing publication workspace");
    }

    public async Task<NativePublicationWorkspace> CreateAsync(string projectId, string title, string description, string revisionLabel, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_publication_create", new JsonObject
        {
            ["title"] = title,
            ["description"] = description,
            ["revision_label"] = revisionLabel,
        }, projectId, cancellationToken).ConfigureAwait(false) ?? throw new InvalidDataException("Missing publication workspace");
        return node.Deserialize<NativePublicationWorkspace>() ?? throw new InvalidDataException("Missing publication workspace");
    }
}
