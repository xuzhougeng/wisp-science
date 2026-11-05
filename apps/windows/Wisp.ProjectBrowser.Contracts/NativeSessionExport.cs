namespace Wisp.ProjectBrowser.Contracts;

// Shared shapes: wisp-dto::native_session_export.
public sealed record NativeSessionExportArtifact(string Path, string Mime, ulong Bytes);
public sealed record NativeSessionExportMissingArtifact(string Path, string Error);
public sealed record NativeSessionExportPreview(string Schema, string ProjectId, string SessionId, string Title,
    string Revision, bool IncludeArtifacts, long HeadEpoch, long MessageCount, long ToolCallCount,
    long TerminalEventCount, NativeSessionExportArtifact[] Artifacts, NativeSessionExportMissingArtifact[] MissingArtifacts,
    ulong ArtifactBytes, string DefaultFilename)
{
    public const string SchemaName = "wisp.native-session-export.v1";
}
public sealed record NativeSessionExportResult(string Schema, string ProjectId, string SessionId, string Revision,
    bool IncludeArtifacts, string DestinationPath, ulong Bytes, string Checksum);

public sealed record NativeDebugRequestFile(string Schema, string ProjectId, string SessionId, string Filename, string Contents)
{
    public const string SchemaName = "wisp.native-debug-request.v1";
}
