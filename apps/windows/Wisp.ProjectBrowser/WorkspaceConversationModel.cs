using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Live conversation cursor. Reads may reconnect; Send/Create/Approve are never replayed
/// after an ambiguous failure. Snapshot pages replace, they never append.</summary>
public sealed partial class WorkspaceConversationModel(INativeConversationClient client, INativeSettingsClient? settings = null)
{
    public event Action? Changed;
    public event Action<ConversationSnapshot>? SnapshotAccepted;
    public NativeInputPreferences InputPreferences { get; private set; } = new();
    public void ApplyInputPreferences(NativeInputPreferences preferences)
    {
        InputPreferences = preferences;
        Notify();
    }
    public string Draft { get; set; } = "";
    public ConversationSnapshot? Snapshot { get; private set; }
    public ConversationSnapshot? History { get; private set; }
    public bool ShowingHistory { get; private set; }
    public ConversationItem[] VisibleItems => (ShowingHistory ? History : Snapshot)?.Items ?? [];
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
    public bool CanAttach => Snapshot is { ReadOnly: false } && !Busy && !UncertainSend && !QueueEnqueuePending && !ShowingHistory && ConnectionError == null;
    public bool CanQueue => CanAttach && !Options.Busy && Snapshot?.Running == true && (Draft.Trim().Length > 0 || Attachments.Length > 0 || References.Length > 0 || Quotes.Length > 0)
        && (Snapshot.Queue != null ? QueuedTurns.Length < 64 : QueuedFollowUp == null) && sessionId != null && !QueueUncertain
        && (References.Length == 0 || Snapshot?.ComposerReferences == true);
    public string? ConnectionError { get; private set; }
    public string? OperationError { get; private set; }
    public ConversationModelOption[] Models { get; private set; } = [];
    public ConversationModelOption[] Agents { get; private set; } = [];
    public NativeComposerEffortModel Effort { get; } = new(settings);
    public NativeComposerOptionsModel Options { get; } = new(settings);
    private JsonObject[] profiles = [];
    public NativeHighlight[] SavedHighlights { get; private set; } = [];
    public string? RevealedExcerpt { get; private set; }
    public int? ScrollTarget { get; private set; }
    public int ScrollRevision { get; private set; }
    public bool CanSend =>
        (Draft.Trim().Length > 0 || Attachments.Length > 0 || References.Length > 0 || Quotes.Length > 0) && Snapshot is { Running: false, ReadOnly: false } && !Busy && !Effort.Busy && !Options.Busy && !UncertainSend
        && ConnectionError == null && !ShowingHistory && !QueueUncertain && !QueueEnqueuePending && (References.Length == 0 || Snapshot?.ComposerReferences == true);
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

    public async Task OpenAsync(string projectId, string sessionId, CancellationToken cancellationToken = default)
    {
        Pause();
        profiles = []; Models = []; Agents = []; Effort.Reset();
        this.projectId = projectId; this.sessionId = sessionId;
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
        UncertainSend = pending != null;
        OperationError = pending == null ? null : "上次发送结果尚未确认。请核对最新消息；不会自动重发。";
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
        if (settings is not null)
        {
            try
            {
                var rows = await settings.InvokeAsync("list_models", new JsonObject(), projectId, cancellationToken) as JsonArray ?? [];
                if (current == generation)
                {
                    profiles = rows.OfType<JsonObject>().Select(row => (JsonObject)row.DeepClone()).ToArray();
                    Models = rows.OfType<JsonObject>()
                        .Where(row => row["use_for_image_generation"]?.GetValue<bool>() != true && row["use_for_video_generation"]?.GetValue<bool>() != true)
                        .Select(row => new ConversationModelOption(row["id"]?.GetValue<string>() ?? "",
                            string.IsNullOrEmpty(row["label"]?.GetValue<string>()) ? row["model"]?.GetValue<string>() ?? "" : row["label"]!.GetValue<string>()))
                        .Where(row => row.Id.Length > 0).ToArray();
                    await SyncEffortAsync(cancellationToken);
                }
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            { if (current == generation) OperationError = ex.Message; }
        }
        if (current != generation) return;
        if (settings != null)
        {
            try
            {
                var agents = await settings.InvokeAsync("list_acp_agents", new(), projectId, cancellationToken) as JsonArray ?? [];
                if (current != generation) return;
                Agents = agents.OfType<JsonObject>().Where(row => row["id"] is JsonValue).Select(row => new ConversationModelOption(
                    row["id"]!.GetValue<string>(), row["label"]?.GetValue<string>() ?? row["id"]!.GetValue<string>())).ToArray();
                if (Snapshot?.ModelId is { } currentModel && currentModel.StartsWith("acp:", StringComparison.Ordinal))
                    Models = [new(currentModel, "ACP · " + (Agents.FirstOrDefault(agent => agent.Id == currentModel[4..])?.Label ?? currentModel[4..]))];
            }
            catch (Exception ex) when (ex is not OperationCanceledException) { if (current == generation) OperationError = "未能读取 ACP Agent：" + ex.Message; }
        }
        if (current == generation) Notify();
    }

    public void Pause()
    {
        HistoryBusy = false;
        RunReviewPrompt = null;
        if (sessionId is { } session) { drafts[session] = Draft; stagedFiles[session] = Attachments; stagedReferences[session] = References; stagedQuotes[session] = Quotes; }
        generation++;
        active = false;
        Effort.Reset();
        Options.Reset();
        ReferencePicker.Bind(null, null);
    }

    public void Reset()
    {
        Pause();
        projectId = null; sessionId = null; Snapshot = null; History = null; ShowingHistory = false;
        Attachments = []; References = []; Quotes = []; QueuedFollowUp = null;
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
            Snapshot = value; ConnectionError = null;
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
        }
        catch (OperationCanceledException) { }
        catch (Exception ex)
        {
            if (current == generation)
                ConnectionError = "连接暂时中断，正在重新读取会话：" + ex.Message;
        }
        if (current == generation) Notify();
        if (current == generation) await RefreshHistoricalRunsAsync(cancellationToken);
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
            if (current == generation) OperationError = "新建会话未确认成功，请刷新列表后检查：" + ex.Message;
            return null;
        }
        finally { if (generation == current) { Busy = false; Notify(); } }
    }

    public async Task SendAsync(CancellationToken cancellationToken = default)
    {
        if (cancellationToken.IsCancellationRequested || UncertainSend || Busy || !CanSend || projectId is not { } project || sessionId is not { } session) return;
        var text = Draft; var id = Guid.NewGuid(); var current = generation;
        var files = Attachments; submittedFiles[session] = files;
        var references = References; submittedReferences[session] = references;
        var quotes = Quotes; submittedQuotes[session] = quotes;
        Busy = true; OperationError = null; pending = (id, text); pendingSends[session] = pending.Value; submittedDrafts[session] = pending.Value;
        Notify();
        try
        {
            await client.SendContextAsync(project, session, id, NativeComposerReferencesModel.Message(NativeComposerQuote.Compose(text, quotes), references),
                files.Select(f => f.Path).ToArray(), references.Select(option => option.Reference).ToArray(), cancellationToken);
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
        if (sessionId is { } session) { pendingSends.Remove(session); uncertainQueues.Remove(session); uncertainQueueActions.Remove(session); RetainUnconfirmedQueueDraft(session); }
        UncertainSend = false; pending = null; OperationError = null; Notify();
    }

    public async Task<bool> AttachAsync(string path, CancellationToken cancellationToken = default)
    {
        if (!CanAttach || string.IsNullOrWhiteSpace(path) || projectId is not { } project || sessionId is not { } session) return false;
        var current = generation; Busy = true; OperationError = null; Notify();
        try
        {
            var file = await client.AttachAsync(project, session, path, cancellationToken);
            if (!file.Path.StartsWith("uploads/", StringComparison.Ordinal) || string.IsNullOrWhiteSpace(file.Name)) throw new InvalidDataException("Invalid attachment response");
            if (current != generation) return false;
            Attachments = Attachments.Where(f => f.Path != file.Path).Append(file).ToArray(); stagedFiles[session] = Attachments;
            return true;
        }
        catch (Exception ex) { if (current == generation) OperationError = "附件未能确认添加；不会自动重试。\n" + ex.Message; return false; }
        finally { if (current == generation) { Busy = false; Notify(); } }
    }
    public void RemoveAttachment(string path)
    {
        if (!CanAttach) return;
        Attachments = Attachments.Where(f => f.Path != path).ToArray();
        if (sessionId != null) stagedFiles[sessionId] = Attachments;
        Notify();
    }
    public bool AddReference(NativeReferenceOption option)
    {
        if (!CanAttach || Snapshot?.ComposerReferences != true || !option.Reference.Valid || References.Length >= 64) return false;
        if (!References.Any(existing => existing.Reference.Key == option.Reference.Key)) References = [.. References, option];
        if (sessionId != null) stagedReferences[sessionId] = References;
        Notify(); return true;
    }
    public void RemoveReference(string key)
    {
        if (!CanAttach) return;
        References = References.Where(option => option.Reference.Key != key).ToArray();
        if (sessionId != null) stagedReferences[sessionId] = References;
        Notify();
    }
    public async Task QueueAsync(CancellationToken cancellationToken = default)
    {
        if (!CanQueue || projectId is not { } project || sessionId is not { } session) return;
        var current = generation; var text = Draft; var files = Attachments.Select(f => f.Path).ToArray();
        var references = References;
        var quotes = Quotes;
        var requestId = Guid.NewGuid();
        var queuedDraft = new QueueDraft(NativeQueueSnapshot.RequestQueueId(requestId), Snapshot!.Epoch, text, Attachments, references, quotes);
        pendingQueueEnqueues[session] = queuedDraft;
        Busy = true; OperationError = null; Notify();
        try
        {
            await client.EnqueueContextAsync(project, session, requestId, NativeComposerReferencesModel.Message(NativeComposerQuote.Compose(text, quotes), references), files,
                references.Select(option => option.Reference).ToArray(), cancellationToken);
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
    public bool AddQuote(ConversationSnapshot source, int itemIndex, string text)
    {
        if (!CanAttach
            || source.ProjectId != projectId || source.SessionId != sessionId || Quotes.Length >= 16
            || text.Length > 32768 || Quotes.Sum(quote => quote.Text.Length) + text.Length > 65536
            || NativeComposerQuote.From(source, itemIndex, text) is not { } quote) return false;
        if (!Quotes.Contains(quote)) Quotes = [.. Quotes, quote];
        stagedQuotes[sessionId!] = Quotes;
        // Quoting an older page returns to the live composer without changing the draft.
        Latest(); return true;
    }
    public void RemoveQuote(NativeComposerQuote quote)
    {
        if (!CanAttach) return;
        Quotes = Quotes.Where(value => value != quote).ToArray();
        if (sessionId != null) stagedQuotes[sessionId] = Quotes;
        Notify();
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

    public Task StopAsync(CancellationToken cancellationToken = default) => ActAsync((project, session, token) => client.StopAsync(project, session, token), cancellationToken);
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
    public Task SelectModelAsync(string modelId, CancellationToken cancellationToken = default) =>
        ActAsync((project, session, token) => client.SetModelAsync(project, session, modelId, token), cancellationToken);

    public bool CanChangePlanMode => CanAttach && !Effort.Busy && !Options.Busy && Snapshot is { Running: false, PlanMode: not null };
    public bool CanChangeFastMode => CanAttach && !Effort.Busy && !Options.Busy && Snapshot is { Running: false, FastMode: not null };
    public Task SetFastModeAsync(bool enabled, CancellationToken cancellationToken = default)
    {
        if (!CanChangeFastMode || Snapshot is not { } snapshot || snapshot.FastMode?.Enabled == enabled) return Task.CompletedTask;
        return ActAsync((project, session, token) => client.SetFastModeAsync(project, session, snapshot.ModelId, enabled, token), cancellationToken);
    }
    public Task SetPlanModeAsync(bool enabled, CancellationToken cancellationToken = default) =>
        CanChangePlanMode && Snapshot?.PlanMode != enabled
            ? ActAsync((project, session, token) => client.SetPlanModeAsync(project, session, enabled, token), cancellationToken)
            : Task.CompletedTask;

    private Task SyncEffortAsync(CancellationToken token) => projectId != null && sessionId != null
        ? Effort.BindAsync(projectId, sessionId, profiles.FirstOrDefault(row => row["id"]?.GetValue<string>() == Snapshot?.ModelId), token)
        : Task.CompletedTask;

    public async Task<bool> SelectEffortAsync(string effort, CancellationToken token = default)
    {
        if (!CanAttach || Snapshot is not { Running: false }) return false;
        var current = generation; var id = Snapshot.ModelId;
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
            ScrollTarget = 0; ScrollRevision++;
            ObserveRunReviews(page, cancellationToken);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (current == generation && request == historyGeneration) OperationError = ex.Message; }
        if (current == generation && request == historyGeneration) Notify();
    }

    public void Latest() { historyGeneration++; RevealedExcerpt = null; ShowingHistory = false; History = null; ScrollTarget = null; ScrollRevision++; Notify(); }

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
