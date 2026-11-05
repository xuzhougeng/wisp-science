using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class WorkspaceSessionRename(INativeSettingsClient client, string projectId, string sessionId, string title) : WorkspaceActionModel
{
    public string Title { get; set; } = title;
    public bool MutationUnconfirmed { get; private set; }
    public bool Saved { get; private set; }
    public bool CanSave => !Closed && !Busy && !MutationUnconfirmed && !Saved && !string.IsNullOrWhiteSpace(Title);
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;
    public Task<bool> SaveAsync()
    {
        if (!CanSave) return Task.FromResult(false);
        var value = Title.Trim();
        return RunAsync(async () =>
        {
            try { await client.InvokeAsync("native_conversation_rename", new() { ["session_id"] = sessionId, ["title"] = value }, projectId); }
            catch { MutationUnconfirmed = true; throw; }
            return value;
        }, saved => { Title = saved; Saved = true; });
    }
}
