namespace Wisp.ProjectBrowser.Contracts;

/// <summary>Shared SessionBranchLink payload from the WebView transcript.</summary>
public sealed record NativeConversationBranch(string Id, string Title, string SourceSessionId,
    int CheckpointUserIndex, string CheckpointKind, bool Merged = false, string? MergeSummary = null);
