using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Live conversation cursor. Reads may reconnect; Send/Create/Approve are never replayed
/// after an ambiguous failure. Snapshot pages replace, they never append.</summary>
public sealed partial class WorkspaceConversationModel(INativeConversationClient client, INativeSettingsClient? settings = null)
{
    public event Action? Changed;
    public event Action<ConversationSnapshot>? SnapshotAccepted;
    public bool AssistantMode { get; init; }
    public bool ContextReady { get; private set; } = true;
    public NativeReferenceOption? ContextReference { get; private set; }
    public void SetAssistantContext(NativeReferenceOption? reference, bool ready)
    {
        if (!AssistantMode) return;
        if (reference != null && (reference.Reference.Kind != "project" || !reference.Reference.Valid))
            throw new ArgumentException("Assistant context must identify a visible project", nameof(reference));
        ContextReference = ready ? reference : null; ContextReady = ready; ModelContext.Bind(Snapshot, ready); Notify();
    }
    private NativeReferenceOption[] SendingReferences => ContextReference is { } context
        ? References.Prepend(context).DistinctBy(value => value.Reference.Key).ToArray() : References;
    public NativeInputPreferences InputPreferences { get; private set; } = new();
    public void ApplyInputPreferences(NativeInputPreferences preferences)
    {
        InputPreferences = preferences;
        Notify();
    }
    public string Draft { get; set; } = "";
    public bool HasComposerContent => !string.IsNullOrWhiteSpace(Draft) || Attachments.Length > 0 || References.Length > 0 || Quotes.Length > 0;
    public bool ShowStopButton => Snapshot?.Running == true && !HasComposerContent;
    private readonly HashSet<(string Project, string Session, string Epoch, string? Request)> pendingStops = [];
    public bool IsStopping => active && !ShowingHistory && Snapshot is { Running: true } turn
        && (turn.Stopping || pendingStops.Contains((turn.ProjectId, turn.SessionId, turn.Epoch, turn.RequestId)));
    public bool CanStop => active && !Loading && !Busy && !ShowingHistory && ConnectionError == null
        && Snapshot is { Running: true, ReadOnly: false } && !IsStopping;
    public ConversationSnapshot? Snapshot { get; private set; }
    public ConversationSnapshot? History { get; private set; }
    public bool ShowingHistory { get; private set; }
    public ConversationItem[] VisibleItems => (ShowingHistory ? History : Snapshot)?.Items ?? [];
    public bool ShowWelcome => !ModelContext.Active && !Loading && !ShowingHistory && VisibleItems.Length == 0
        && Snapshot?.Exploration is not { Summary: not null } && Snapshot?.Exploration?.SourceFrozen != true
        && Snapshot?.Running != true && ConnectionError == null && (OperationError == null || HasInlineAttachmentError) && Snapshot?.Error == null;
    public bool Loading { get; private set; }
    public bool Busy { get; private set; }
    public bool UncertainSend { get; private set; }
    public ComposerAttachment[] Attachments { get; private set; } = [];
    public NativeReferenceOption[] References { get; private set; } = [];
    public NativeComposerQuote[] Quotes { get; private set; } = [];
    private readonly Dictionary<string, NativeComposerQuote[]> stagedQuotes = [];
    private readonly Dictionary<string, NativeComposerQuote[]> submittedQuotes = [];
    public NativeComposerReferencesModel ReferencePicker { get; } = new(settings);
    private readonly Dictionary<string, NativeReferenceOption[]> stagedReferences = [];
    private readonly Dictionary<string, NativeReferenceOption[]> submittedReferences = [];
    public string? QueuedFollowUp { get; private set; }
    private readonly Dictionary<string, ComposerAttachment[]> stagedFiles = [];
    private readonly Dictionary<string, ComposerAttachment[]> submittedFiles = [];
    private readonly Dictionary<string, string> queuedDrafts = [];
    private readonly HashSet<string> uncertainQueues = [];
    public bool CanAttach => ContextReady && Snapshot is { ReadOnly: false } && !Busy && !CompactionBlocksInput && !UncertainSend && !QueueEnqueuePending && !ShowingHistory && ConnectionError == null;
    public bool CanQueue => CanAttach && !Options.Busy && Snapshot?.Running == true && HasComposerContent
        && (Snapshot.Queue != null ? QueuedTurns.Length < 64 : QueuedFollowUp == null) && sessionId != null && !QueueUncertain
        && (SendingReferences.Length == 0 || Snapshot?.ComposerReferences == true);
    public string? ConnectionError { get; private set; }
    public string? OperationError { get; private set; }
    public ConversationModelOption[] Models { get; private set; } = [];
    public ConversationModelOption[] Agents { get; private set; } = [];
    public NativeComposerEffortModel Effort { get; } = new(settings);
    public NativeRuntimeStripModel Runtimes { get; } = new(settings);
    public NativeContextDetailsModel ContextDetails { get; } = new(settings);
    public NativeModelContextModel ModelContext { get; } = new(settings);
    public NativeComposerOptionsModel Options { get; } = new(settings);
    private JsonObject[] profiles = [];
    public ComposerModelMenuChoice[] ModelMenuChoices => NativeComposerModelMenu.Choices(profiles, Agents, ComposerModelId);
    public NativeHighlight[] SavedHighlights { get; private set; } = [];
    public string? RevealedExcerpt { get; private set; }
    public int? ScrollTarget { get; private set; }
    public int ScrollRevision { get; private set; }
    public bool CanSend =>
        HasComposerContent && Snapshot is { Running: false, ReadOnly: false } && !Busy && !SavingFastMode && !Effort.Busy && !Options.Busy && !UncertainSend
        && ContextReady && !CompactionBlocksInput && ConnectionError == null && !ShowingHistory && !QueueUncertain && !QueueEnqueuePending && (SendingReferences.Length == 0 || Snapshot?.ComposerReferences == true);
    private string? projectId, sessionId;
    private int generation;
    private int historyGeneration;
    private bool active;
    private (Guid Id, string Text)? pending;
    private readonly Dictionary<string, string> drafts = [];
    private readonly Dictionary<string, (Guid Id, string Text)> pendingSends = [];
    private readonly Dictionary<string, (Guid Id, string Text)> submittedDrafts = [];
    private ConversationCursor? cursor;
    private readonly HashSet<string> savingSelections = [];

    public async Task OpenAsync(string projectId, string sessionId, CancellationToken cancellationToken = default, long lastActivity = 0)
    {
        Pause();
        profiles = []; Models = []; Agents = []; Effort.Reset();
        this.projectId = projectId; this.sessionId = sessionId;
        Runtimes.Bind(projectId, sessionId);
        Options.Bind(projectId, sessionId);
        ReferencePicker.Bind(projectId, sessionId);
        active = true;
        Draft = drafts.GetValueOrDefault(sessionId, "");
        Attachments = stagedFiles.GetValueOrDefault(sessionId, []);
        References = stagedReferences.GetValueOrDefault(sessionId, []);
        Quotes = stagedQuotes.GetValueOrDefault(sessionId, []);
        QueuedFollowUp = queuedDrafts.GetValueOrDefault(sessionId);
        Snapshot = null; History = null; ShowingHistory = false; RevealedExcerpt = null; ScrollTarget = null;
        SavedHighlights = []; cursor = new(projectId, sessionId);
        pending = pendingSends.TryGetValue(sessionId, out var waiting) ? waiting : null;
        UncertainSend = pending != null || uncertainRecoveryResumes.ContainsKey((projectId, sessionId));
        OperationError = !UncertainSend ? null : "上次发送结果尚未确认。请核对最新消息；不会自动重发。";
        ConnectionError = null; Loading = true; Busy = false;
        var current = ++generation;
        Notify();
        await RefreshAsync(cancellationToken);
        if (current != generation) return;
        Loading = false;
        if (Snapshot != null)
        {
            try { await client.MarkSeenAsync(projectId, sessionId, cancellationToken); }
            catch (Exception ex) when (ex is not OperationCanceledException)
            { if (current == generation) OperationError = "未能标记已查看：" + ex.Message; }
        }
        if (current != generation) return;
        await LoadComposerCatalogAsync(projectId, current, cancellationToken);
        if (current == generation) Notify();
        if (current == generation) await ReadIdleCompactionPromptAsync(lastActivity, current, cancellationToken);
    }

    public void Pause()
    {
        ContextRecovery = null; recoveryCompactionOwner = null; recoveryResumeReady = null;
        IdleCompactionPrompt = null;
        excerptStart = null; excerptReadFailure = null;
        AttachmentUploads = []; attachmentReads.Clear(); attachmentOperationError = null;
        HistoryBusy = false;
        RunReviewPrompt = null;
        if (CurrentProjectDraft is { } projectDraft) { projectDraft.Text = Draft; projectDraft.References = References; projectDraft.Quotes = Quotes; }
        if (sessionId is { } session) { drafts[session] = Draft; stagedFiles[session] = Attachments; stagedReferences[session] = References; stagedQuotes[session] = Quotes; }
        generation++;
        active = false;
        ContextDetails.Bind(null);
        Handoff.Bind(null, false);
        ModelContext.Bind(null);
        Runtimes.Bind(null, null);
        Effort.Reset();
        Options.Reset();
        ReferencePicker.Bind(null, null);
    }

    public void Reset()
    {
        Pause();
        projectId = null; sessionId = null; Snapshot = null; History = null; ShowingHistory = false;
        Attachments = []; References = []; Quotes = []; QueuedFollowUp = null;
        Draft = ""; OperationError = null; UncertainSend = false; pending = null;
        Loading = false; Busy = false; ConnectionError = null; Notify();
    }

    public async Task RefreshAsync(CancellationToken cancellationToken = default)
    {
        if (!active || projectId is not { } project || sessionId is not { } session) return;
        var current = generation;
        try
        {
            var value = await client.SnapshotAsync(project, session, null, cancellationToken);
            if (current != generation) return;
            if (cursor is null || !cursor.TryAccept(value)) return;
            var previous = Snapshot;
            Snapshot = value; ConnectionError = null;
            if (excerptStart is { } start && !ValidExcerptTurn(start)) excerptStart = null;
            ModelContext.Bind(value, ContextReady);
            ReconcileCompaction(value);
            if (value.AcpAgentId != null || value.Items.Length > 0) provisionalAgents.Remove((project, session));
            foreach (var run in value.RunCards ?? []) runReadErrors.Remove((project, session, run.Id));
            SnapshotAccepted?.Invoke(value);
            _ = SyncEffortAsync(cancellationToken);
            if (pending is { } waiting && value.RequestId == waiting.Id.ToString())
            {
                if (Draft == waiting.Text) Draft = "";
                pending = null; pendingSends.Remove(session); UncertainSend = false; OperationError = null;
                Attachments = []; stagedFiles[session] = [];
                References = []; stagedReferences[session] = [];
                Quotes = []; stagedQuotes[session] = [];
            }
            if (!value.Running && submittedDrafts.TryGetValue(session, out var submitted) && value.RequestId == submitted.Id.ToString())
            {
                if (value.Error != null && Draft.Length == 0)
                { Draft = submitted.Text; Attachments = submittedFiles.GetValueOrDefault(session, []); stagedFiles[session] = Attachments;
                    References = submittedReferences.GetValueOrDefault(session, []); stagedReferences[session] = References;
                    Quotes = submittedQuotes.GetValueOrDefault(session, []); stagedQuotes[session] = Quotes; }
                submittedDrafts.Remove(session);
                submittedFiles.Remove(session);
                submittedReferences.Remove(session);
                submittedQuotes.Remove(session);
            }
            if (value.Queue != null) ReconcileQueue(value);
            else if (!value.Running) { QueuedFollowUp = null; queuedDrafts.Remove(session); }
            ObserveRunReviews(value, cancellationToken);
            ObserveContextRecovery(previous, value);
        }
        catch (OperationCanceledException) { }
        catch (Exception ex)
        {
            if (current == generation)
                ConnectionError = "连接暂时中断，正在重新读取会话：" + ex.Message;
        }
        if (current == generation) Notify();
        if (current == generation) await RefreshHistoricalRunsAsync(cancellationToken);
        if (current == generation) await ResumeRecoveredTurnAsync(cancellationToken);
    }

    public async Task<string?> CreateAsync(string projectId, CancellationToken cancellationToken = default, string? agentId = null)
    {
        if (Busy) return null;
        Busy = true; OperationError = null; var current = generation; Notify();
        try
        {
            var id = await client.CreateWithAgentAsync(projectId, agentId, cancellationToken);
            if (string.IsNullOrEmpty(id)) throw new InvalidDataException("Missing new session ID");
            return id;
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (current == generation) OperationError = CreationFailurePrefix + ex.Message;
            return null;
        }
        finally { if (generation == current) { Busy = false; Notify(); } }
    }

    public async Task SendAsync(CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested || UncertainSend || Busy || !CanSend || projectId is not { } project || sessionId is not { } session) return;
        var text = Draft; var id = Guid.NewGuid(); var current = generation;
        var selectedAgent = ProvisionalAgent;
        var files = Attachments; submittedFiles[session] = files;
        var references = SendingReferences; submittedReferences[session] = References;
        var quotes = Quotes; submittedQuotes[session] = quotes;
        Busy = true; OperationError = null; pending = (id, text); pendingSends[session] = pending.Value; submittedDrafts[session] = pending.Value;
        Notify();
        try
        {
            await client.SendSelectedContextAsync(project, session, id, NativeComposerReferencesModel.Message(NativeComposerQuote.Compose(text, quotes), references),
                files.Select(f => f.Path).ToArray(), references.Where(option => option.Reference.Kind != "file-path").Select(option => option.Reference).ToArray(), selectedAgent, cancellationToken);
            pendingSends.Remove(session);
            stagedFiles[session] = [];
            stagedReferences[session] = [];
            stagedQuotes[session] = [];
            if (drafts.GetValueOrDefault(session) == text) drafts[session] = "";
            if (generation != current) return;
            if (Draft == text) Draft = "";
            Attachments = [];
            References = [];
            Quotes = [];
            pending = null;
        }
        catch (Exception ex)
        {
            if (generation != current) return;
            UncertainSend = true;
            OperationError = "发送结果尚未确认。正在读取服务器状态；不会自动重发。" + ex.Message;
        }
        finally { if (generation == current) { Busy = false; Notify(); } }
        if (generation == current && !cancellationToken.IsCancellationRequested) await RefreshAsync(cancellationToken);
    }

    public void AcknowledgeUncertainSend()
    {
        if (projectId != null && sessionId != null) uncertainRecoveryResumes.Remove((projectId, sessionId));
        if (sessionId is { } session) { pendingSends.Remove(session); uncertainQueues.Remove(session); uncertainQueueActions.Remove(session); RetainUnconfirmedQueueDraft(session); }
        UncertainSend = false; pending = null; OperationError = null; Notify();
    }

    public void ReportAttachmentSelectionError(string message)
    {
        OperationError = message; Notify();
    }

    public async Task<int> AttachFilesAsync(IReadOnlyList<string> paths, CancellationToken cancellationToken = default)
    {
        if (!CanAttach) return 0;
        var current = generation; var accepted = 0;
        foreach (var path in paths.Where(path => !string.IsNullOrWhiteSpace(path)).Distinct(StringComparer.OrdinalIgnoreCase))
        {
            // Stop on the first unknown result. Retain accepted files and never
            // replay a copy, or continue the selection into a different session.
            if (current != generation || !CanAttach || !await AttachAsync(path, cancellationToken)) break;
            if (current != generation) break;
            accepted++;
        }
        return accepted;
    }
    public async Task<bool> AttachAsync(string path, CancellationToken cancellationToken = default)
    {
        if (!CanAttach || string.IsNullOrWhiteSpace(path) || projectId is not { } project || sessionId is not { } session) return false;
        var upload = new NativeAttachmentUpload(Guid.NewGuid(), path.Replace('\\', '/').Split('/').Last());
        AttachmentUploads = [.. AttachmentUploads, upload];
        var current = generation; Busy = true; OperationError = null; Notify();
        try
        {
            var file = await client.AttachAsync(project, session, path, cancellationToken);
            if (!NativeAttachmentPresentation.ValidPath(file.Path) || string.IsNullOrWhiteSpace(file.Name)) throw new InvalidDataException("Invalid attachment response");
            if (current != generation) return false;
            AttachmentUploads = AttachmentUploads.Where(item => item.Id != upload.Id).ToArray();
            Attachments = Attachments.Where(f => f.Path != file.Path).Append(file).ToArray(); stagedFiles[session] = Attachments;
            return true;
        }
        catch (Exception ex)
        {
            if (current == generation)
            {
                AttachmentUploads = AttachmentUploads.Select(item => item.Id == upload.Id ? item with { Error = ex.Message } : item).ToArray();
                OperationError = attachmentOperationError = "附件未能确认添加；不会自动重试。\n" + ex.Message;
            }
            return false;
        }
        finally { if (current == generation) { Busy = false; Notify(); } }
    }
    public void RemoveAttachment(string path)
    {
        if (!CanAttach) return;
        Attachments = Attachments.Where(f => f.Path != path).ToArray();
        attachmentReads.Remove(path);
        if (sessionId != null) stagedFiles[sessionId] = Attachments;
        Notify();
    }
    public bool AddReference(NativeReferenceOption option)
    {
        if (!(CanAttach && Snapshot?.ComposerReferences == true || CanPrepareProjectDraft && option.Reference.Kind is "file-path" or "session")
            || !option.Reference.Valid || References.Length >= 64) return false;
        if (!References.Any(existing => existing.Reference.Key == option.Reference.Key)) References = [.. References, option];
        if (sessionId != null) stagedReferences[sessionId] = References;
        else if (CurrentProjectDraft is { } draft) draft.References = References;
        Notify(); return true;
    }
    public bool CanRemoveReference => CanAttach || CanPrepareProjectDraft;
    public void RemoveReference(string key)
    {
        if (!CanRemoveReference) return;
        References = References.Where(option => option.Reference.Key != key).ToArray();
        if (sessionId != null) stagedReferences[sessionId] = References;
        else if (CurrentProjectDraft is { } draft) draft.References = References;
        Notify();
    }
    public async Task QueueAsync(CancellationToken cancellationToken = default)
    {
        if (!CanQueue || projectId is not { } project || sessionId is not { } session) return;
        var current = generation; var text = Draft; var files = Attachments.Select(f => f.Path).ToArray();
        var references = SendingReferences;
        var quotes = Quotes;
        var requestId = Guid.NewGuid();
        var queuedDraft = new QueueDraft(NativeQueueSnapshot.RequestQueueId(requestId), Snapshot!.Epoch, text, Attachments, references, quotes);
        pendingQueueEnqueues[session] = queuedDraft;
        Busy = true; OperationError = null; Notify();
        try
        {
            await client.EnqueueContextAsync(project, session, requestId, NativeComposerReferencesModel.Message(NativeComposerQuote.Compose(text, quotes), references), files,
                references.Where(option => option.Reference.Kind != "file-path").Select(option => option.Reference).ToArray(), cancellationToken);
            if (pendingQueueEnqueues.TryGetValue(session, out var waitingQueue) && waitingQueue.Id == queuedDraft.Id)
            {
                pendingQueueEnqueues.Remove(session); submittedQueueDrafts[(session, queuedDraft.Id)] = queuedDraft;
                ConsumeQueuedDraft(session, queuedDraft);
            }
            queuedDrafts[session] = text;
            if (current != generation) return;
            QueuedFollowUp = text;
        }
        catch (Exception ex)
        {
            if (pendingQueueEnqueues.TryGetValue(session, out var waitingQueue) && waitingQueue.Id == queuedDraft.Id)
            {
                uncertainQueues.Add(session);
                queueErrorDetails[session] = ex.Message;
                if (current == generation) OperationError = "后续未能确认排队；请核对会话，不会自动重试。\n" + ex.Message;
            }
        }
        finally { if (current == generation) { Busy = false; Notify(); } }
        if (current == generation && !cancellationToken.IsCancellationRequested) await RefreshAsync(cancellationToken);
    }
    public bool Prefill(string text, bool append = false)
    {
        if (sessionId == null || Busy || UncertainSend || Snapshot is not { ReadOnly: false }) return false;
        Draft = append && !string.IsNullOrWhiteSpace(Draft) ? Draft + "\n\n" + text : text; Notify(); return true;
    }
    // Explicit Insert only: never creates a conversation or sends a message.
    public bool InsertMemoBody(string body)
    {
        if (!(CanCompose || CanPrepareProjectDraft) || UncertainSend) return false;
        Draft = WorkspaceMemosModel.ComposeInsert(Draft, body);
        if (CurrentProjectDraft is { } draft) draft.Text = Draft;
        Notify(); return true;
    }
    public bool CanQuote(ConversationSnapshot source, int itemIndex, string text) => CanAttach
        && source.ProjectId == projectId && source.SessionId == sessionId && Quotes.Length < 16
        && text.Length <= 32768 && Quotes.Sum(quote => quote.Text.Length) + text.Length <= 65536
        && NativeComposerQuote.From(source, itemIndex, text) != null;

    public bool AddQuote(ConversationSnapshot source, int itemIndex, string text)
    {
        if (!CanQuote(source, itemIndex, text) || NativeComposerQuote.From(source, itemIndex, text) is not { } quote) return false;
        if (!Quotes.Contains(quote)) Quotes = [.. Quotes, quote];
        stagedQuotes[sessionId!] = Quotes;
        // Quoting an older page returns to the live composer without changing the draft.
        Latest(); return true;
    }
    public void RemoveQuote(NativeComposerQuote quote)
    {
        if (!CanAttach && !CanPrepareProjectDraft) return;
        Quotes = Quotes.Where(value => value != quote).ToArray();
        if (sessionId != null) stagedQuotes[sessionId] = Quotes;
        Notify();
    }
    public bool AddRuntimeQuote(string text)
    {
        if (!(CanAttach || CanPrepareProjectDraft) || string.IsNullOrWhiteSpace(text) || text.Length > 32768
            || Quotes.Length >= 16 || Quotes.Sum(q => q.Text.Length) + text.Length > 65536) return false;
        var value = new NativeComposerQuote(sessionId ?? "", -1, "runtime", text.Trim());
        if (!Quotes.Contains(value)) Quotes = [.. Quotes, value];
        if (sessionId != null) stagedQuotes[sessionId] = Quotes;
        if (CurrentProjectDraft is { } draft) draft.Quotes = Quotes;
        Notify(); return true;
    }
    public bool AddDocumentQuote(string project, string session, string path, NativeDocumentSelection selection)
    {
        if (!CanAttach || project != projectId || session != sessionId || !selection.Valid
            || string.IsNullOrWhiteSpace(path) || path.Length > 32768 || path.Any(char.IsControl)
            || Quotes.Length >= 16 || Quotes.Sum(quote => quote.Text.Length) + selection.Text.Length > 65536) return false;
        var quote = new NativeComposerQuote(session, -1, "document", selection.Text.Trim(), new(project, path, selection.Location));
        if (!Quotes.Contains(quote)) Quotes = [.. Quotes, quote];
        stagedQuotes[session] = Quotes;
        Latest(); return true;
    }
    public async Task PrepareIssueReportAsync()
    {
        if (settings == null || projectId is not { } project || sessionId == null || Busy) return;
        var current = generation; var original = Draft;
        try
        {
            var bootstrap = await settings.InvokeAsync("get_bootstrap_status", new(), project);
            if (current != generation || Draft != original) return;
            string Field(string key) => bootstrap?[key]?.GetValue<string>() ?? "未记录";
            var model = Snapshot?.ModelId.StartsWith("acp:", StringComparison.Ordinal) == true
                ? Snapshot.ModelId[4..] : Models.FirstOrDefault(m => m.Id == Snapshot?.ModelId)?.Label ?? Models.FirstOrDefault()?.Label ?? "not configured";
            Prefill($"请帮我向 xuzhougeng/wisp-science 提交一个 GitHub issue。\n\n【已自动采集，请勿向我索要 API key、transcript、项目文件、环境变量、用户名或绝对路径】\n- Wisp 版本：{Field("app_version")}\n- OS / 架构：{Field("os")} / {Field("arch")}\n- 模型配置：{model}\n- 启动耗时：{Field("startup")}\n\n请用中文逐条引导我说明：发生了什么、复现步骤、预期与实际行为，以及我知道的 Run ID 或错误信息。\n若启动很慢或长时间白屏，提醒我在 Windows 正式版可把日志发给维护者：%APPDATA%\\science.wisp-science\\wisp-science\\logs\\wisp.log（上次启动：wisp.previous.log）。\n信息足够后，给出简短 issue 标题和 Markdown 正文，并提供预填链接：https://github.com/xuzhougeng/wisp-science/issues/new?title=...&body=...\n提醒截图需在 GitHub 页面手动附加，Wisp 不会上传截图。");
        }
        catch (Exception ex) { if (current == generation) { OperationError = ex.Message; Notify(); } }
    }

    public async Task StopAsync(CancellationToken cancellationToken = default)
    {
        if (!CanStop || cancellationToken.IsCancellationRequested || projectId is not { } project || sessionId is not { } session) return;
        // Interrupt only the selected turn. The input stays editable, including
        // while a runtime is taking time to acknowledge its cancellation.
        var owner = generation; var key = (project, session, Snapshot!.Epoch, Snapshot.RequestId);
        pendingStops.Add(key); OperationError = null; Notify();
        try
        {
            await client.StopAsync(project, session, cancellationToken);
            if (generation == owner) await RefreshAsync(cancellationToken);
        }
        catch (OperationCanceledException)
        {
            if (generation == owner) ConnectionError = OperationError = "操作等待已取消，结果尚未确认；不会自动重试。请重新读取会话核对。";
        }
        catch (Exception ex) { if (generation == owner) OperationError = ex.Message; }
        finally
        {
            pendingStops.Remove(key);
            if (projectId == project && sessionId == session) Notify();
        }
    }
    public Task RespondAcpPermissionAsync(NativeAcpPermission permission, string? optionId, CancellationToken token = default)
    {
        if (ShowingHistory || ConnectionError != null || Snapshot is not { ReadOnly: false }
            || permission.FrameId != sessionId || Snapshot.Acp?.Permissions.FirstOrDefault(value => value.RequestId == permission.RequestId) is not { } pending
            || optionId != null && !pending.Options.Any(option => option.Id == optionId)) return Task.CompletedTask;
        return ActAsync((project, session, cancellation) => client.AcpPermissionAsync(project, session, permission.RequestId, optionId, cancellation), token);
    }
    public bool CanAnswerAcp(string requestId) => !ShowingHistory && !Busy && ConnectionError == null && Snapshot is { ReadOnly: false }
        && Snapshot.Acp?.QuestionIds.Contains(requestId) == true;
    public bool CanChangeAcpSettings => CanAttach && !Effort.Busy && !Options.Busy
        && Snapshot is { Running: false, AcpState: not null };
    public Task SetAcpModeAsync(string modeId, CancellationToken token = default) =>
        CanChangeAcpSettings && Snapshot!.AcpState!.CurrentMode != modeId
            && Snapshot.AcpState.ModeChoices.Any(choice => choice.Id == modeId)
            ? ActAsync((project, session, cancellation) => client.AcpSettingAsync(project, session,
                new() { ["kind"] = "mode", ["id"] = modeId }, cancellation), token) : Task.CompletedTask;
    public Task SetAcpConfigAsync(string configId, JsonNode value, CancellationToken token = default) =>
        CanChangeAcpSettings && Snapshot!.AcpState!.Allows(configId, value)
            ? ActAsync((project, session, cancellation) => client.AcpSettingAsync(project, session,
                new() { ["kind"] = "config", ["id"] = configId, ["value"] = value.DeepClone() }, cancellation), token) : Task.CompletedTask;
    public Task AnswerAcpAsync(string requestId, string answer, CancellationToken token = default) =>
        CanAnswerAcp(requestId) && !string.IsNullOrWhiteSpace(answer)
            ? ActAsync((project, session, cancellation) => client.AcpAnswerAsync(project, session, requestId, answer, cancellation), token)
            : Task.CompletedTask;
    public Task ApproveAsync(ConversationApproval approval, bool allowed, CancellationToken cancellationToken = default) =>
        ActAsync((project, session, token) => client.ApproveAsync(project, session, approval.ApprovalId, allowed, token), cancellationToken);
    public async Task<bool> SelectModelAsync(string modelId, CancellationToken cancellationToken = default)
    {
        if (!CanAttach || Snapshot is not { Running: false, AcpAgentId: null } || AcpModelLocked)
            return false;
        var owner = generation; var accepted = false; var compact = false;
        await ActAsync(async (project, session, token) =>
        {
            if (settings != null && VisibleItems.Length > 0)
                compact = (await settings.InvokeAsync("get_settings", new(), project, token))?["semantic_compact_on_model_switch"]?.GetValue<bool>() == true;
            if (owner != generation || token.IsCancellationRequested) return;
            await client.SetModelAsync(project, session, modelId, token);
            accepted = true;
        }, cancellationToken);
        // A rejected/uncertain write or late reply must not suppress future confirmation.
        if (accepted && owner == generation && projectId != null)
        {
            provisionalAgents.Remove((projectId, sessionId));
            await SyncEffortAsync(cancellationToken);
            if (compact && owner == generation && !cancellationToken.IsCancellationRequested)
                await StartCompactionAsync(token: cancellationToken);
            if (owner == generation) Notify();
            return owner == generation;
        }
        return false;
    }

    public bool CanChangePlanMode => ComposerAgentId == null && CanAttach && !Effort.Busy && !Options.Busy && Snapshot is { Running: false, PlanMode: not null };
    private const string DraftFastFailurePrefix = "Draft Fast mode was not confirmed: ";
    public string? DraftFastFailureDetail => OperationError?.StartsWith(DraftFastFailurePrefix, StringComparison.Ordinal) == true
        ? OperationError[DraftFastFailurePrefix.Length..] : null;
    private bool? DraftFastDefault => ComposerProfile is { } profile && NativeModelPresentation.SupportsFast(profile)
        ? profile["service_tier"]?.GetValue<string>().Trim() is "priority" or "fast" : null;
    public ConversationFastMode? ComposerFastMode => ComposerAgentId != null ? null
        : CurrentProjectDraft is { } draft && DraftFastDefault is bool inherited
            ? new(draft.FastOverride ?? inherited, draft.FastOverride == null) : Snapshot?.FastMode;
    private int? fastWriteGeneration;
    public bool SavingFastMode => fastWriteGeneration == generation;
    public bool CanChangeFastMode => ComposerFastMode != null && !SavingFastMode
        && (CanPrepareProjectDraft || CanAttach && !Effort.Busy && !Options.Busy && Snapshot is { Running: false });
    public async Task SetFastModeAsync(bool enabled, CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested || !CanChangeFastMode || ComposerFastMode?.Enabled == enabled) return;
        if (CurrentProjectDraft is { } draft && DraftFastDefault is bool inherited)
        {
            draft.FastOverride = enabled == inherited ? null : enabled;
            Notify(); return;
        }
        await WriteSessionFastAsync(enabled, cancellationToken);
    }
    private async Task<bool> WriteSessionFastAsync(bool enabled, CancellationToken token)
    {
        if (token.IsCancellationRequested || !CanChangeFastMode || Snapshot is not { } snapshot) return false;
        var owner = generation; var acknowledged = false;
        fastWriteGeneration = owner;
        try
        {
            await ActAsync(async (project, session, ct) =>
            {
                await client.SetFastModeAsync(project, session, snapshot.ModelId, enabled, ct);
                acknowledged = true;
            }, token);
            return acknowledged && owner == generation && !token.IsCancellationRequested && ConnectionError == null
                && Snapshot?.ModelId == snapshot.ModelId && Snapshot.FastMode?.Enabled == enabled;
        }
        finally { if (fastWriteGeneration == owner) { fastWriteGeneration = null; Notify(); } }
    }
    public Task SetPlanModeAsync(bool enabled, CancellationToken cancellationToken = default) =>
        CanChangePlanMode && Snapshot?.PlanMode != enabled
            ? ActAsync((project, session, token) => client.SetPlanModeAsync(project, session, enabled, token), cancellationToken)
            : Task.CompletedTask;

    private JsonObject? ComposerProfile => ComposerAgentId != null
        ? null : NativeModelPresentation.SessionProfile(profiles, Snapshot?.ModelId);
    public string? ComposerModelId => ComposerAgentId is { } agent ? "acp:" + agent : ComposerProfile?["id"]?.GetValue<string>();
    public bool CanSelectEffort => Effort.Available && !Effort.Busy && !Options.Busy
        && (CanPrepareProjectDraft || CanAttach && Snapshot is { Running: false });
    private Task SyncEffortAsync(CancellationToken token) => projectId != null
        ? Effort.BindAsync(projectId, sessionId, ComposerProfile, token)
        : Task.CompletedTask;

    public async Task<bool> SelectEffortAsync(string effort, CancellationToken token = default)
    {
        if (!CanSelectEffort) return false;
        var current = generation; var id = ComposerModelId;
        var saved = await Effort.SaveAsync(effort, token);
        if (saved && current == generation && profiles.FirstOrDefault(row => row["id"]?.GetValue<string>() == id) is { } profile)
            profile["reasoning_effort"] = Effort.Value;
        return saved && current == generation;
    }

    public async Task OlderAsync(CancellationToken cancellationToken = default)
    {
        RevealedExcerpt = null;
        if (projectId is not { } project || sessionId is not { } session) return;
        var cursorSeq = (ShowingHistory ? History : Snapshot)?.NextBeforeSeq;
        if (cursorSeq is null) return;
        var current = generation;
        var request = ++historyGeneration;
        try
        {
            var page = await client.SnapshotAsync(project, session, cursorSeq, cancellationToken);
            if (current != generation || request != historyGeneration) return;
            History = page;
            ShowingHistory = true;
            await ModelContext.SelectAsync(false, cancellationToken);
            ScrollTarget = 0; ScrollRevision++;
            ObserveRunReviews(page, cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (current == generation && request == historyGeneration) OperationError = ex.Message; }
        if (current == generation && request == historyGeneration) Notify();
    }

    public void Latest() { historyGeneration++; RevealedExcerpt = null; ShowingHistory = false; History = null; ScrollTarget = null; ScrollRevision++; Notify(); }

    public async Task<bool> RevealChatQuestionAsync(ProjectChatLocation target)
    {
        if (Snapshot == null) return false;
        if (WorkspaceOutlineModel.QuestionItemIndex(target.UserIndex, Snapshot.UserOffset, Snapshot.Items) != null)
        {
            var owner = generation;
            var jump = ++historyGeneration;
            await ModelContext.SelectAsync(false);
            if (owner != generation || jump != historyGeneration || Snapshot == null
                || WorkspaceOutlineModel.QuestionItemIndex(target.UserIndex, Snapshot.UserOffset, Snapshot.Items) is not { } index) return false;
            History = null; ShowingHistory = false; RevealedExcerpt = null;
            ScrollTarget = index; ScrollRevision++; Notify(); return true;
        }
        if (Snapshot.Running) return false;
        return await OpenQuestionAsync(new(target.UserIndex, "", target.BeforeSeq, null, null));
    }

    public async Task<bool> OpenQuestionAsync(ConversationOutlineEntry entry, CancellationToken cancellationToken = default)
    {
        RevealedExcerpt = null;
        if (projectId is not { } project || sessionId is not { } session) return false;
        var current = generation;
        var request = ++historyGeneration;
        var opened = false;
        try
        {
            var page = await client.SnapshotAsync(project, session, entry.BeforeSeq, cancellationToken);
            if (current != generation || request != historyGeneration || cancellationToken.IsCancellationRequested) return false;
            var index = WorkspaceOutlineModel.QuestionItemIndex(entry.UserIndex, page.UserOffset, page.Items)
                ?? throw new InvalidOperationException("问题位置已变化，请刷新大纲后重试。");
            History = page; ShowingHistory = true; ScrollTarget = index; ScrollRevision++;
            await ModelContext.SelectAsync(false, cancellationToken);
            opened = true; OperationError = null;
            ObserveRunReviews(page, cancellationToken);
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (current == generation && request == historyGeneration) OperationError = ex.Message; }
        if (current == generation && request == historyGeneration) Notify();
        return opened;
    }

    public void RevealExcerpt(string text)
    {
        var index = Array.FindIndex(VisibleItems, item =>
            NativeSavedExcerpt.Find(RenderedText(item), text) != null
            || (item.Role == "tool" && item.Input is { } input && NativeSavedExcerpt.Find(input, text) != null));
        if (index < 0) { OperationError = "未在当前已加载的消息中找到原文；请打开对应历史记录后重试。"; Notify(); return; }
        OperationError = null; RevealedExcerpt = text; ScrollTarget = index; ScrollRevision++; Notify();
    }

    public void ClearExcerpt(int revision) { if (ScrollRevision == revision) { RevealedExcerpt = null; Notify(); } }

    public async Task SaveSelectionAsync(string text, CancellationToken cancellationToken = default)
    {
        if (projectId is not { } project || sessionId is not { } session || string.IsNullOrWhiteSpace(text) || savingSelections.Contains(text)) return;
        var current = generation; savingSelections.Add(text); OperationError = null;
        try
        {
            var row = await new NativeHighlightClient(settings ?? throw new InvalidOperationException("Missing settings transport"))
                .StarAsync(project, session, text, cancellationToken);
            if (current != generation) return;
            SavedHighlights = SavedHighlights.Where(item => item.Id != row.Id).Append(row).ToArray();
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (current == generation) OperationError = ex.Message; }
        finally { if (current == generation) savingSelections.Remove(text); Notify(); }
    }

    public static string RenderedText(ConversationItem item) => item.Role == "tool" ? item.Text : item.Text;

    private async Task ActAsync(Func<string, string, CancellationToken, Task> action, CancellationToken cancellationToken)
    {
        if (Busy || cancellationToken.IsCancellationRequested || projectId is not { } project || sessionId is not { } session) return;
        var current = generation; Busy = true; OperationError = null; Notify();
        var cancelled = false;
        try { await action(project, session, cancellationToken); }
        catch (OperationCanceledException)
        {
            cancelled = true;
            if (current == generation)
            {
                OperationError = "操作等待已取消，结果尚未确认；不会自动重试。请重新读取会话核对。";
                ConnectionError = OperationError;
            }
        }
        catch (Exception ex)
        { if (current == generation) OperationError = ex.Message; }
        finally { if (current == generation) { Busy = false; Notify(); } }
        if (current == generation && !cancelled) await RefreshAsync(cancellationToken);
    }

    private void Notify() => Changed?.Invoke();
}

public sealed record ConversationModelOption(string Id, string Label);
