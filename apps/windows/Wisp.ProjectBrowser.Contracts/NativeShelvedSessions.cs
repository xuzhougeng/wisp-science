using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

// Mirrors crates/wisp-dto/src/native_projects.rs; shared fixture under contracts/native-projects/v1.
public sealed record ShelvedSessionCursor(
    [property: JsonPropertyName("ts")] long Timestamp,
    [property: JsonPropertyName("id")] string Id);
public sealed record ShelvedSession(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("ts")] long Timestamp);
public sealed record ShelvedSessionPage(
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("items")] ShelvedSession[] Items,
    [property: JsonPropertyName("next_cursor")] ShelvedSessionCursor? NextCursor);
public sealed record SessionShelveResult(
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("session_id")] string SessionId,
    [property: JsonPropertyName("shelved")] bool Shelved);
