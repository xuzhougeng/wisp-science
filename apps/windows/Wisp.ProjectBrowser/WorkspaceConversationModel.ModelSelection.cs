using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    // Like the WebView's provisional choice: selecting a profile does not bind
    // or launch an ACP process. The first explicit send supplies the identity.
    private readonly Dictionary<(string Project, string? Session), string> provisionalAgents = [];
    private string? ProvisionalAgent => active && projectId != null ? provisionalAgents.GetValueOrDefault((projectId, sessionId)) : null;
    public string? ComposerAgentId => ProvisionalAgent ?? Snapshot?.AcpAgentId
        ?? (Snapshot?.ModelId.StartsWith("acp:", StringComparison.Ordinal) == true ? Snapshot.ModelId[4..] : null);
    public bool AcpModelLocked => Snapshot?.AcpAgentId != null || Snapshot?.ModelId.StartsWith("acp:", StringComparison.Ordinal) == true;
    private const string CreationFailurePrefix = "新建会话未确认成功，请刷新列表后检查：";
    private const string DraftModelFailurePrefix = "Default model change was not confirmed. Reload to check: ";
    public int AcpDraftRevision { get; private set; }
    public string? CreationFailureDetail => OperationError?.StartsWith(CreationFailurePrefix, StringComparison.Ordinal) == true
        ? OperationError[CreationFailurePrefix.Length..] : null;
    public string? DraftModelFailureDetail => OperationError?.StartsWith(DraftModelFailurePrefix, StringComparison.Ordinal) == true
        ? OperationError[DraftModelFailurePrefix.Length..] : null;
    public bool CanSelectAcpInPlace => !AssistantMode && !AcpModelLocked && !Effort.Busy && !Options.Busy
        && (CanPrepareProjectDraft || CanAttach && !Loading && Snapshot is { Running: false } && VisibleItems.Length == 0);

    public bool SelectProvisionalAgent(string agentId)
    {
        if (!CanSelectAcpInPlace || projectId == null || !Agents.Any(agent => agent.Id == agentId)) return false;
        provisionalAgents[(projectId, sessionId)] = agentId;
        if (CurrentProjectDraft is { } draft) draft.FastOverride = null;
        Effort.Reset(); Notify(); return true;
    }

    public bool RestoreProvisionalHttpModel(string id)
    {
        if (ProvisionalAgent == null || !CanSelectAcpInPlace || projectId == null
            || NativeModelPresentation.SessionProfile(profiles, Snapshot?.ModelId)?["id"]?.GetValue<string>() != id) return false;
        provisionalAgents.Remove((projectId, sessionId));
        _ = SyncEffortAsync(CancellationToken.None); Notify(); return true;
    }

    // WebView set_active_model without a session changes the app-wide default.
    // Choosing a model in a project draft must not allocate a conversation.
    public async Task<bool> SelectDraftModelAsync(string id, CancellationToken token = default)
    {
        if (settings == null || token.IsCancellationRequested || !CanPrepareProjectDraft || projectId is not { } project
            || CurrentProjectDraft is not { } draft || !profiles.Any(p => p["id"]?.GetValue<string>() == id && NativeModelPresentation.IsChat(p))) return false;
        var current = generation; ++modelCatalogRevision;
        Busy = true; draft.Error = OperationError = null; Notify();
        try
        {
            var reply = await settings.InvokeAsync("set_active_model", new() { ["id"] = id }, project, token) as JsonArray;
            if (current != generation || token.IsCancellationRequested) return false;
            if (reply == null || !reply.OfType<JsonObject>().Any(p => p["id"]?.GetValue<string>() == id
                && p["active"]?.GetValue<bool>() == true && NativeModelPresentation.IsChat(p)))
                throw new InvalidDataException("The response did not confirm the selected default model.");
            ApplyComposerProfiles(reply);
            provisionalAgents.Remove((project, null));
            await SyncEffortAsync(token);
            return current == generation && !token.IsCancellationRequested;
        }
        catch (Exception ex)
        {
            if (current == generation) draft.Error = OperationError = DraftModelFailurePrefix + ex.Message;
            return false;
        }
        finally { if (current == generation) { Busy = false; Notify(); } }
    }

    public async Task<string?> CreateAcpDraftAsync(string agentId, CancellationToken token = default)
    {
        if (AssistantMode || !CanAttach || Loading || Effort.Busy || Options.Busy || Snapshot is not { Running: false } || projectId is not { } project
            || !Agents.Any(agent => agent.Id == agentId) || ComposerAgentId == agentId || token.IsCancellationRequested) return null;
        var owner = generation; var source = sessionId;
        // Create a normal empty frame, leaving the choice provisional until send.
        var id = await CreateAsync(project, token);
        if (id == null) return null;
        provisionalAgents[(project, id)] = agentId;
        if (owner != generation || projectId != project || sessionId != source || token.IsCancellationRequested) return null;
        drafts[id] = Draft; stagedFiles[id] = Attachments; stagedReferences[id] = References; stagedQuotes[id] = Quotes;
        // OpenAsync's Pause stashes the now-empty source, so returning to it does
        // not show a second copy of the moved draft.
        Draft = ""; Attachments = []; References = []; Quotes = [];
        await OpenAsync(project, id, token);
        if (projectId != project || sessionId != id || token.IsCancellationRequested) return null;
        AcpDraftRevision++;
        return id;
    }
}
