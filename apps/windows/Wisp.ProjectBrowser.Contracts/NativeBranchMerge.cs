using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeBranchReply<T>(
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("result")] T Result);
public sealed record NativeBranchDelta(
    [property: JsonPropertyName("seq")] long Seq,
    [property: JsonPropertyName("role")] string Role,
    [property: JsonPropertyName("text")] string Text);
public sealed record NativeBranchPreview(
    [property: JsonPropertyName("main_session_id")] string MainSessionId,
    [property: JsonPropertyName("branch_session_id")] string BranchSessionId,
    [property: JsonPropertyName("branch_title")] string BranchTitle,
    [property: JsonPropertyName("checkpoint_user_index")] int CheckpointUserIndex,
    [property: JsonPropertyName("checkpoint_kind")] string CheckpointKind,
    [property: JsonPropertyName("guard_hash")] string GuardHash,
    [property: JsonPropertyName("new_message_count")] int NewMessageCount,
    [property: JsonPropertyName("messages")] NativeBranchDelta[] Messages);
public sealed record NativeBranchMergeResult(
    [property: JsonPropertyName("main_session_id")] string MainSessionId,
    [property: JsonPropertyName("branch_session_id")] string BranchSessionId,
    [property: JsonPropertyName("summary_message_seq"), JsonRequired] long SummaryMessageSeq);
