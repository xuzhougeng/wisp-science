namespace Wisp.ProjectBrowser.Contracts;

// Shapes shared with wisp-dto::native_projects and session_artifacts.
public sealed record SessionChildrenPreview(int Subagents, int Branches, int Explorations, string[] Nested)
{
    public bool IsEmpty => Subagents + Branches + Explorations == 0;
}
public sealed record RetainedSessionArtifact(string Name, string Reason);
public sealed record SessionArtifactPreview(string Fingerprint, string[] Artifacts, string[] Files, RetainedSessionArtifact[] Retained);
public sealed record SessionChildArtifactPreview(string Id, SessionArtifactPreview Preview);
public sealed record SessionFamilyArtifactPreview(string Id, SessionArtifactPreview Preview, SessionChildArtifactPreview[] Children);
public sealed record SessionDeletePreview(string ProjectId, string[] SessionIds, SessionChildrenPreview Children,
    bool? IncludeChildren, SessionFamilyArtifactPreview[] Artifacts, string? ArtifactError);
