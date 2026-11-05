namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    public NativeHandoffModel Handoff { get; } = new(settings);
    public bool CanOpenHandoff => CanOpenCompaction && !AssistantMode;

    public async Task<string> PrepareHandoffSessionAsync(NativeHandoffLaunch launch)
    {
        if (!CanOpenHandoff || Snapshot?.Epoch != launch.Epoch || projectId != launch.ProjectId
            || sessionId != launch.SessionId || string.IsNullOrWhiteSpace(launch.Note))
            throw new InvalidOperationException("The source conversation changed. Open its handoff again.");
        return await CreatePromptSessionAsync(launch.ProjectId, launch.Note);
    }

    public bool CanStartPromptSession(string project) => CurrentProjectId == project && !Busy && Snapshot?.Running != true;

    public async Task<string> CreatePromptSessionAsync(string project, string prompt)
    {
        if (!CanStartPromptSession(project) || string.IsNullOrWhiteSpace(prompt))
            throw new InvalidOperationException("A new conversation is unavailable.");
        var id = await CreateAsync(project);
        if (id == null) throw new IOException(OperationError ?? "New conversation creation was not confirmed.");
        // Only the new session gets the prompt. Preserve source drafts and attachments.
        drafts[id] = prompt.Trim();
        return id;
    }
}
