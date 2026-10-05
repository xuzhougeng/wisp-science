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
    [property: JsonPropertyName("state")] string State,
    [property: JsonPropertyName("publication_id")] string PublicationId = "",
    [property: JsonPropertyName("revision_number")] long RevisionNumber = 0,
    [property: JsonPropertyName("capability_level")] string CapabilityLevel = "",
    [property: JsonPropertyName("manifest_sha256")] string? ManifestSha256 = null,
    [property: JsonPropertyName("parent_revision_id")] string? ParentRevisionId = null);

public sealed record NativePublicationItem(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("ordinal")] long Ordinal,
    [property: JsonPropertyName("revision_id")] string RevisionId = "",
    [property: JsonPropertyName("parent_item_id")] string? ParentItemId = null,
    [property: JsonPropertyName("content")] string Content = "");

public sealed record NativePublicationWorkspace(
    [property: JsonPropertyName("publications")] IReadOnlyList<NativePublication> Publications,
    [property: JsonPropertyName("publication")] NativePublication? Publication,
    [property: JsonPropertyName("revision")] NativePublicationRevision? Revision,
    [property: JsonPropertyName("items")] IReadOnlyList<NativePublicationItem> Items,
    [property: JsonPropertyName("revisions")] NativePublicationRevision[]? Revisions = null,
    [property: JsonPropertyName("bindings")] NativePublicationBinding[]? Bindings = null,
    [property: JsonPropertyName("readiness")] JsonObject? Readiness = null,
    [property: JsonPropertyName("lineage")] JsonObject[]? Lineage = null,
    [property: JsonPropertyName("drift")] JsonObject[]? Drift = null,
    [property: JsonPropertyName("reviews")] JsonObject[]? Reviews = null,
    [property: JsonPropertyName("waivers")] JsonObject[]? Waivers = null,
    [property: JsonPropertyName("capsule_builds")] JsonObject[]? CapsuleBuilds = null,
    [property: JsonPropertyName("reproduction_runs")] JsonObject[]? ReproductionRuns = null,
    [property: JsonPropertyName("reproduction_results")] JsonObject[]? ReproductionResults = null,
    [property: JsonPropertyName("effective_capability_level")] string? EffectiveCapabilityLevel = null,
    [property: JsonPropertyName("supersessions")] JsonObject[]? Supersessions = null,
    [property: JsonPropertyName("item_links")] JsonObject[]? ItemLinks = null);

public sealed record NativePublicationBinding(string Id, string RevisionId, string? ItemId, string SourceKind, string SourceId,
    string Purpose, string? SupportedClaimItemId, string SelectionState, string ReviewState, string ReproductionState, string Visibility, string SourceSnapshotJson);
public sealed record NativePublicationSource(string Kind, string Id, string Title, string Detail, string? Text, string? TextSha256, string? FrameId, long? MessageSeq);
public sealed record NativePublicationSources(NativePublicationSource[] Sources, bool HasMore);
public sealed record NativePublicationMutation(NativePublicationWorkspace Workspace, JsonObject? Readiness);

/// <summary>
/// Publication workspace for one explicit project. A lost create is not retried.
/// </summary>
public interface INativePublicationClient
{
    Task<NativePublicationWorkspace> ReadAsync(string projectId, CancellationToken cancellationToken = default);
    Task<NativePublicationWorkspace> SelectAsync(string projectId, string? publicationId, string? revisionId, CancellationToken cancellationToken = default);
    Task<NativePublicationSources> SourcesAsync(string projectId, string kind, string query, uint offset, CancellationToken cancellationToken = default);
    Task<NativePublicationMutation> MutateAsync(string projectId, string revisionId, JsonObject operation, CancellationToken cancellationToken = default);
    Task<NativePublicationWorkspace> CreateAsync(string projectId, string title, string description, string revisionLabel, CancellationToken cancellationToken = default);
}

public sealed class NativePublicationClient(INativeSettingsClient transport) : INativePublicationClient
{
    private static readonly JsonSerializerOptions Json = new() { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower };
    public Task<NativePublicationWorkspace> ReadAsync(string projectId, CancellationToken cancellationToken = default) => SelectAsync(projectId, null, null, cancellationToken);
    public async Task<NativePublicationWorkspace> SelectAsync(string projectId, string? publicationId, string? revisionId, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_publication_workspace", new JsonObject { ["publication_id"] = publicationId, ["revision_id"] = revisionId }, projectId, cancellationToken).ConfigureAwait(false);
        return Validate(node?.Deserialize<NativePublicationWorkspace>(Json), projectId, publicationId, revisionId);
    }
    public async Task<NativePublicationSources> SourcesAsync(string projectId, string kind, string query, uint offset, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_publication_sources", new() { ["kind"] = kind, ["query"] = query, ["offset"] = offset }, projectId, cancellationToken).ConfigureAwait(false);
        var page = node?.Deserialize<NativePublicationSources>(Json) ?? throw new InvalidDataException("Missing publication sources");
        if (page.Sources == null || page.Sources.Any(s => string.IsNullOrEmpty(s.Id) || s.Kind is not ("artifact_version" or "run" or "message_span")))
            throw new InvalidDataException("Invalid publication sources");
        return page;
    }
    public async Task<NativePublicationMutation> MutateAsync(string projectId, string revisionId, JsonObject operation, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_publication_mutate", new() { ["revision_id"] = revisionId, ["operation"] = operation.DeepClone() }, projectId, cancellationToken).ConfigureAwait(false);
        var value = node?.Deserialize<NativePublicationMutation>(Json) ?? throw new InvalidDataException("Missing publication mutation result");
        Validate(value.Workspace, projectId, null, operation["action"]?.GetValue<string>() == "clone_revision" ? null : revisionId);
        if (operation["action"]?.GetValue<string>() == "clone_revision"
            && (value.Workspace.Revision is not { } cloned || cloned.Id == revisionId || cloned.ParentRevisionId != revisionId))
            throw new InvalidDataException("Publication clone does not descend from the selected revision");
        if (value.Readiness != null && value.Readiness["revision_id"]?.GetValue<string>() != value.Workspace.Revision?.Id)
            throw new InvalidDataException("Publication readiness revision mismatch");
        return value;
    }
    private static NativePublicationWorkspace Validate(NativePublicationWorkspace? page, string project, string? publication = null, string? revision = null)
    {
        if (page?.Publications == null || page.Items == null || page.Publications.Any(p => p.ProjectId != project)
            || page.Publication is { } selected && selected.ProjectId != project
            || publication != null && page.Publication?.Id != publication
            || revision != null && page.Revision?.Id != revision
            || page.Revision is { PublicationId.Length: > 0 } r && r.PublicationId != page.Publication?.Id
            || page.Items.Any(i => i.RevisionId.Length > 0 && i.RevisionId != page.Revision?.Id)
            || (page.Bindings ?? []).Any(b => b.RevisionId != page.Revision?.Id)
            || (page.Revisions ?? []).Any(r => r.PublicationId.Length > 0 && r.PublicationId != page.Publication?.Id))
            throw new InvalidDataException("Publication workspace identity mismatch");
        return page;
    }

    public async Task<NativePublicationWorkspace> CreateAsync(string projectId, string title, string description, string revisionLabel, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_publication_create", new JsonObject
        {
            ["title"] = title,
            ["description"] = description,
            ["revision_label"] = revisionLabel,
        }, projectId, cancellationToken).ConfigureAwait(false) ?? throw new InvalidDataException("Missing publication workspace");
        return Validate(node.Deserialize<NativePublicationWorkspace>(Json), projectId);
    }
}
