using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed partial class WorkspaceConversationModel
{
    private sealed class ProjectDraft
    {
        public string Text = "";
        public NativeReferenceOption[] References = [];
        public NativeComposerQuote[] Quotes = [];
        public string? CreatedSession;
        public bool? FastOverride;
        public string? Error;
        public bool Creating, Uncertain;
    }
    private readonly Dictionary<string, ProjectDraft> projectDrafts = [];
    private int modelCatalogRevision;
    private ProjectDraft? CurrentProjectDraft => active && projectId != null && sessionId == null
        ? projectDrafts.GetValueOrDefault(projectId) : null;
    public string? CurrentProjectId => projectId;
    public string? CurrentSessionId => sessionId;
    public bool HasProjectDraft => CurrentProjectDraft != null;
    public bool DraftCreationUncertain => CurrentProjectDraft?.Uncertain == true;
    public bool CanCompose => ContextReady && !Loading && !ShowingHistory && (Snapshot is { ReadOnly: false }
        || CurrentProjectDraft is { Creating: false }) && !Busy;
    public bool CanPrepareProjectDraft => CurrentProjectDraft is { Creating: false, Uncertain: false }
        && !Busy && !Loading && !Effort.Busy && !Options.Busy && ConnectionError == null;
    public bool CanSubmit => !Loading && (CanSend || CanQueue || CanPrepareProjectDraft && HasComposerContent);

    /// Opening or typing in a project does not create a persisted conversation.
    public async Task OpenProjectDraftAsync(string project, CancellationToken token = default)
    {
        Reset();
        projectId = project; active = true;
        Runtimes.Bind(project, null);
        Options.Bind(project, null);
        if (!projectDrafts.TryGetValue(project, out var draft)) projectDrafts[project] = draft = new();
        Draft = draft.Text; References = draft.References; Quotes = draft.Quotes; OperationError = draft.Error; Busy = draft.Creating;
        profiles = []; Models = []; Agents = [];
        var current = generation;
        Notify();
        await LoadComposerCatalogAsync(project, current, token);
    }

    /// Called only for an explicit send or a control that needs a session.
    /// Remember acknowledged creation even if navigation wins the race, so a later
    /// attempt can open that session instead of issuing a second create.
    public async Task<string?> EnsureDraftSessionAsync(CancellationToken token = default)
    {
        if (token.IsCancellationRequested || !CanPrepareProjectDraft || projectId is not { } project
            || CurrentProjectDraft is not { } draft) return null;
        var current = generation;
        draft.Text = Draft;
        if (draft.CreatedSession == null)
        {
            draft.Creating = true; Busy = true; draft.Error = OperationError = null; Notify();
            try
            {
                var id = await client.CreateAsync(project, token);
                if (string.IsNullOrWhiteSpace(id)) throw new InvalidDataException("Missing new session ID");
                draft.CreatedSession = id;
            }
            catch (Exception ex)
            {
                draft.Uncertain = true;
                draft.Error = "Conversation creation was not confirmed. Check the session list before retrying. " + ex.Message;
            }
            finally
            {
                draft.Creating = false;
                if (CurrentProjectDraft == draft) { Busy = false; OperationError = draft.Error; Notify(); }
            }
        }
        if (current != generation || token.IsCancellationRequested || draft.CreatedSession is not { } session) return null;
        drafts[session] = Draft;
        stagedReferences[session] = References;
        stagedQuotes[session] = Quotes;
        if (ProvisionalAgent is { } agent) provisionalAgents[(project, session)] = agent;
        var fast = draft.FastOverride;
        var expectedModel = ComposerModelId;
        // OpenAsync saves this project's draft before adopting the confirmed session.
        var opening = OpenAsync(project, session, token);
        var openingGeneration = generation;
        await opening;
        if (openingGeneration != generation || token.IsCancellationRequested || Snapshot is not { ReadOnly: false }
            || Snapshot.SessionId != session || Snapshot.ProjectId != project || ConnectionError != null) return null;
        projectDrafts.Remove(project);
        provisionalAgents.Remove((project, null));
        // Keep this already-created frame and its contents if applying the option fails.
        if (fast is bool enabled && (Snapshot.ModelId != expectedModel || ComposerFastMode == null
            || (ComposerFastMode.Enabled != enabled && !await WriteSessionFastAsync(enabled, token))))
        {
            if (openingGeneration == generation)
            {
                OperationError = DraftFastFailurePrefix + (OperationError ?? ConnectionError ?? "");
                Notify();
            }
            return null;
        }
        return session;
    }

    public void AcknowledgeDraftCreation()
    {
        if (CurrentProjectDraft is not { Creating: false, Uncertain: true } draft) return;
        draft.Uncertain = false; draft.Error = OperationError = null; Notify();
    }

    public Task ReloadComposerCatalogAsync(CancellationToken token = default) => !Busy && active && projectId is { } project
        ? LoadComposerCatalogAsync(project, generation, token) : Task.CompletedTask;

    private void ApplyComposerProfiles(JsonArray rows)
    {
        profiles = rows.OfType<JsonObject>().Select(row => (JsonObject)row.DeepClone()).ToArray();
        Models = profiles.Where(NativeModelPresentation.IsChat)
            .Select(row => new ConversationModelOption(row["id"]?.GetValue<string>() ?? "", NativeModelPresentation.Label(row)))
            .Where(row => row.Id.Length > 0).ToArray();
        if (CurrentProjectDraft is { FastOverride: bool selected } draft
            && (DraftFastDefault is not bool inherited || selected == inherited)) draft.FastOverride = null;
    }

    private async Task LoadComposerCatalogAsync(string project, int current, CancellationToken token)
    {
        if (settings == null) return;
        var revision = ++modelCatalogRevision;
        bool Current() => current == generation && revision == modelCatalogRevision;
        try
        {
            var rows = await settings.InvokeAsync("list_models", new(), project, token) as JsonArray ?? [];
            if (!Current()) return;
            ApplyComposerProfiles(rows);
            if (DraftModelFailureDetail != null)
            { OperationError = null; if (CurrentProjectDraft is { } draft) draft.Error = null; }
            await SyncEffortAsync(token);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (Current()) OperationError = ex.Message; }
        if (!Current()) return;
        try
        {
            var agents = await settings.InvokeAsync("list_acp_agents", new(), project, token) as JsonArray ?? [];
            if (!Current()) return;
            Agents = agents.OfType<JsonObject>().Where(row => row["id"] is JsonValue).Select(row => new ConversationModelOption(
                row["id"]!.GetValue<string>(), row["label"]?.GetValue<string>() ?? row["id"]!.GetValue<string>())).ToArray();
            if (Snapshot?.ModelId is { } selected && selected.StartsWith("acp:", StringComparison.Ordinal))
                Models = [new(selected, "ACP · " + (Agents.FirstOrDefault(agent => agent.Id == selected[4..])?.Label ?? selected[4..]))];
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (Current()) OperationError = "未能读取 ACP Agent：" + ex.Message; }
        if (Current()) Notify();
    }
}
