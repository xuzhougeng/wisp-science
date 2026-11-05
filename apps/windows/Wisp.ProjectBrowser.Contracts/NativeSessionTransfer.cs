namespace Wisp.ProjectBrowser.Contracts;

// Shared shapes: wisp-dto::native_session_transfer (snake_case on the transport).
public sealed record NativeSessionTransferPreview(string Schema, string ProjectId, string SessionId,
    string TargetProjectId, string Mode, string Title, long MessageCount, string Revision,
    SessionArtifactPreview? Artifacts, string? ArtifactError)
{
    public const string SchemaName = "wisp.native-session-transfer.v1";
}
public sealed record NativeSessionTransferResult(string Schema, string ProjectId, string SessionId,
    string TargetProjectId, string Mode, bool IncludeArtifacts, string FrameId);
