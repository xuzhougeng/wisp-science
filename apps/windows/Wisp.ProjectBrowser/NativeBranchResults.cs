using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public static class NativeBranchResults
{
    /// <summary>Match the WebView's before-user and after-response anchors,
    /// including the absolute user offset of a historical page.</summary>
    public static NativeConversationBranch[] At(ConversationSnapshot page, int row)
    {
        if (row < 0 || row >= page.Items.Length || page.Items[row].Role is not ("user" or "assistant")) return [];
        var turns = page.Items.Take(row + 1).Count(item => item.Role is "user" or "queued_user");
        if (turns == 0) return [];
        var index = (page.UserOffset ?? 0) + turns - 1;
        var kind = page.Items[row].Role == "user" ? "before_user" : "after_response";
        return (page.Branches ?? []).Where(branch => branch.SourceSessionId == page.SessionId
            && branch.Id != page.SessionId && !string.IsNullOrWhiteSpace(branch.Id)
            && branch.CheckpointUserIndex == index && branch.CheckpointKind == kind).ToArray();
    }

    // Legacy BranchMerge rows encode branch title in tool_name and id in input.
    public static bool IsStandalone(ConversationItem item) => item.Role == "branch_merge";
    public static string Title(string? title, string fallback) => string.IsNullOrWhiteSpace(title) ? fallback : title;
}
