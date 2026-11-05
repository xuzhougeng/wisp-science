using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed record NativeComposeEntry(string Id, string Icon, string LabelKey, string DetailKey);

public static class NativeComposeMenu
{
    public static IReadOnlyList<NativeComposeEntry> Entries { get; } = Array.AsReadOnly(new[] {
        new NativeComposeEntry("attach", "attach", "composer.attach_files", "composer.attach_files_sub"),
        new NativeComposeEntry("files", "folder", "composer.your_files", "composer.your_files_sub"),
        new NativeComposeEntry("review", "review", "composer.request_review", "composer.request_review_sub"),
        new NativeComposeEntry("share", "share", "share.title", "composer.share_sub"),
        new NativeComposeEntry("save-skill", "skill", "composer.save_skill", "composer.save_skill_sub"),
        new NativeComposeEntry("skills", "gear", "skills.manage", "skills.manage_sub")
    });
}

public sealed partial class WorkspaceConversationModel
{
    public bool ComposeSessionReady => active && ContextReady && !Loading && !ShowingHistory
        && Snapshot != null && ConnectionError == null;
    public bool CanShareConversation => ComposeSessionReady
        && Snapshot!.Items.Any(item => (item.Role is "user" or "assistant") && !string.IsNullOrWhiteSpace(item.Text));
    public bool CanReviewConversation => settings != null && ComposeSessionReady && CanAttach
        && !HistoryBusy && !HistoryUncertain && Snapshot is { Running: false, Stopping: false }
        && Snapshot.HistoryState?.Reviewing != true;

    public void DraftSkill(string prompt)
    {
        if (!CanCompose) return;
        Draft = prompt; Notify();
    }

    public async Task ReviewConversationAsync(CancellationToken token = default)
    {
        if (token.IsCancellationRequested || !CanReviewConversation || projectId is not { } project || sessionId is not { } session) return;
        var current = generation; Busy = HistoryBusy = true; OperationError = null; Notify();
        try
        {
            await settings!.InvokeAsync("native_conversation_review", new JsonObject { ["session_id"] = session }, project, token);
        }
        catch (Exception ex)
        {
            uncertainHistory.Add(session);
            if (current == generation) OperationError = ex.Message;
        }
        finally { if (current == generation) { Busy = HistoryBusy = false; Notify(); } }
        if (current == generation && !token.IsCancellationRequested) await RefreshAsync(token);
    }
}
