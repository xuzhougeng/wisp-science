using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

/// <summary>
/// Boundary used by the WinUI 3 model and bundled wisp-service.exe transport.
/// The UI never opens SQLite directly.
/// </summary>
public interface IProjectBrowserClient
{
    // Launch with --allow-project-writes only for this explicit desired-state command.
    // Success returns the authoritative projects snapshot; never optimistically reorder.
    Task<ProjectListSnapshot> SetProjectStarredAsync(
        string databasePath, string projectId, bool starred, CancellationToken cancellationToken = default);
    Task<IReadOnlyList<BrowserSession>> ListSessionsAsync(
        string databasePath, string? projectId = null, CancellationToken cancellationToken = default);
    Task<TranscriptPage> GetTranscriptAsync(
        string databasePath, string projectId, string sessionId, long? beforeSeq = null,
        CancellationToken cancellationToken = default);
    Task<ProjectListSnapshot> ListProjectsAsync(
        string databasePath,
        CancellationToken cancellationToken = default);
}

public static class ProjectBrowserProtocol
{
    public const string Schema = "wisp.project-browser.v1";
    public const string PersistedOnly = "persisted_only";
}

public sealed record ProjectListSnapshot(
    IReadOnlyList<ProjectSummary> Projects,
    string ActivitySource);

public sealed record ProjectSummary(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("name")] string Name,
    [property: JsonPropertyName("description")] string Description,
    [property: JsonPropertyName("workspace_dir")] string WorkspaceDirectory,
    [property: JsonPropertyName("starred")] bool Starred,
    [property: JsonPropertyName("session_count")] long SessionCount,
    [property: JsonPropertyName("artifact_count")] long ArtifactCount,
    [property: JsonPropertyName("updated_at")] long UpdatedAt,
    [property: JsonPropertyName("running_count")] long RunningCount,
    [property: JsonPropertyName("needs_you_count")] long NeedsYouCount,
    [property: JsonPropertyName("sync_configured")] bool SyncConfigured,
    [property: JsonPropertyName("last_synced_at")] long? LastSyncedAt);

/// <summary>Decode this envelope, validate Schema/Id/Type, then expose a snapshot.</summary>
public sealed record ProjectBrowserResponse(
    [property: JsonPropertyName("schema")] string Schema,
    [property: JsonPropertyName("id")] string? Id,
    [property: JsonPropertyName("type")] string Type,
    [property: JsonPropertyName("projects")] IReadOnlyList<ProjectSummary>? Projects,
    [property: JsonPropertyName("activity_source")] string? ActivitySource,
    [property: JsonPropertyName("code")] string? Code,
    [property: JsonPropertyName("message")] string? Message,
    [property: JsonPropertyName("sessions")] IReadOnlyList<BrowserSession>? Sessions = null,
    [property: JsonPropertyName("messages")] IReadOnlyList<BrowserMessage>? Messages = null,
    [property: JsonPropertyName("next_before_seq")] long? NextBeforeSeq = null);

public sealed record BrowserSession(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("ts")] long Timestamp,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("folder_id")] string? FolderId = null);

public sealed record BrowserMessage(
    [property: JsonPropertyName("seq")] long Sequence,
    [property: JsonPropertyName("role")] string Role,
    [property: JsonPropertyName("text")] string Text,
    [property: JsonPropertyName("tool_name")] string? ToolName);

public sealed record TranscriptPage(IReadOnlyList<BrowserMessage> Messages, long? NextBeforeSeq);

public sealed record SetProjectStarredRequest(
    [property: JsonPropertyName("schema")] string Schema,
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("type")] string Type,
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("starred")] bool Starred);
