using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

// Wire shapes from wisp-dto/src/project_chat_search.rs.
public sealed record ProjectChatCursor([property: JsonPropertyName("ts"), JsonRequired] long Ts,
    [property: JsonPropertyName("session_id"), JsonRequired] string SessionId,
    [property: JsonPropertyName("message_seq"), JsonRequired] long MessageSeq);
public sealed record ProjectChatHit(
    [property: JsonPropertyName("session_id"), JsonRequired] string SessionId,
    [property: JsonPropertyName("session_title"), JsonRequired] string SessionTitle,
    [property: JsonPropertyName("message_seq"), JsonRequired] long MessageSeq,
    [property: JsonPropertyName("role"), JsonRequired] string Role,
    [property: JsonPropertyName("ts"), JsonRequired] long Ts,
    [property: JsonPropertyName("before"), JsonRequired] string Before,
    [property: JsonPropertyName("matched"), JsonRequired] string Matched,
    [property: JsonPropertyName("after"), JsonRequired] string After);
public sealed record ProjectChatMatches([property: JsonPropertyName("items"), JsonRequired] ProjectChatHit[] Items,
    [property: JsonPropertyName("next_cursor"), JsonRequired] ProjectChatCursor? NextCursor);
public sealed record ProjectChatLocation([property: JsonPropertyName("user_index"), JsonRequired] int UserIndex,
    [property: JsonPropertyName("before_seq"), JsonRequired] long? BeforeSeq);
public interface IProjectChatHistory
{
    Task<ProjectChatMatches> SearchChatsAsync(string database, string project, string query, ProjectChatCursor? cursor, CancellationToken token);
    Task<ProjectChatLocation?> LocateChatAsync(string database, string project, ProjectChatHit hit, CancellationToken token);
}
