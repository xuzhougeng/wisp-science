using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Live conversation cursor. Reads may reconnect; Send/Create/Approve are never replayed
/// after an ambiguous failure. Snapshot pages replace, they never append.</summary>
public sealed class WorkspaceConversationModel(INativeConversationClient client, INativeSettingsClient? settings = null)
{
    public event Action? Changed;
    public string Draft { get; set; } = "";
    public ConversationSnapshot? Snapshot { get; private set; }
    public ConversationSnapshot? History { get; private set; }
    public bool ShowingHistory { get; private set; }
    public ConversationItem[] VisibleItems => (ShowingHistory ? History : Snapshot)?.Items ?? [];
    public bool Loading { get; private set; }
    public bool Busy { get; private set; }
    public bool UncertainSend { get; private set; }
    public ComposerAttachment[] Attachments { get; private set; } = [];
    public string? QueuedFollowUp { get; private set; }
    private readonly Dictionary<string, ComposerAttachment[]> stagedFiles = [];
    private readonly Dictionary<string, ComposerAttachment[]> submittedFiles = [];
    private readonly Dictionary<string, string> queuedDrafts = [];
    private readonly HashSet<string> uncertainQueues = [];
    public bool CanAttach => Snapshot is { ReadOnly: false } && !Busy && !UncertainSend && !ShowingHistory && ConnectionError == null;
    public bool CanQueue => CanAttach && Snapshot?.Running == true && (Draft.Trim().Length > 0 || Attachments.Length > 0)
        && QueuedFollowUp == null && sessionId != null && !uncertainQueues.Contains(sessionId);
    public string? ConnectionError { get; private set; }
    public string? OperationError { get; private set; }
    public ConversationModelOption[] Models { get; private set; } = [];
    public NativeHighlight[] SavedHighlights { get; private set; } = [];
    public string? RevealedExcerpt { get; private set; }
    public int? ScrollTarget { get; private set; }
    public int ScrollRevision { get; private set; }
    public bool CanSend =>
        (Draft.Trim().Length > 0 || Attachments.Length > 0) && Snapshot is { Running: false, ReadOnly: false } && !Busy && !UncertainSend
        && ConnectionError == null && !ShowingHistory;
    private string? projectId, sessionId;
    private int generation;
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
        this.projectId = projectId; this.sessionId = sessionId;
        active = true;
        Draft = drafts.GetValueOrDefault(sessionId, "");
        Attachments = stagedFiles.GetValueOrDefault(sessionId, []);
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
                    Models = rows.OfType<JsonObject>()
                        .Where(row => row["use_for_image_generation"]?.GetValue<bool>() != true && row["use_for_video_generation"]?.GetValue<bool>() != true)
                        .Select(row => new ConversationModelOption(row["id"]?.GetValue<string>() ?? "",
                            string.IsNullOrEmpty(row["label"]?.GetValue<string>()) ? row["model"]?.GetValue<string>() ?? "" : row["label"]!.GetValue<string>()))
                        .Where(row => row.Id.Length > 0).ToArray();
            }
            catch (Exception ex) when (ex is not OperationCanceledException)
            { if (current == generation) OperationError = ex.Message; }
        }
        if (current != generation) return;
        Notify();
    }

    public void Pause()
    {
        if (sessionId is { } session) { drafts[session] = Draft; stagedFiles[session] = Attachments; }
        generation++;
        active = false;
    }

    public void Reset()
    {
        Pause();
        projectId = null; sessionId = null; Snapshot = null; History = null; ShowingHistory = false;
        Attachments = []; QueuedFollowUp = null;
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
            if (pending is { } waiting && value.RequestId == waiting.Id.ToString())
            {
                if (Draft == waiting.Text) Draft = "";
                pending = null; pendingSends.Remove(session); UncertainSend = false; OperationError = null;
                Attachments = []; stagedFiles[session] = [];
            }
            if (!value.Running && submittedDrafts.TryGetValue(session, out var submitted) && value.RequestId == submitted.Id.ToString())
            {
                if (value.Error != null && Draft.Length == 0)
                { Draft = submitted.Text; Attachments = submittedFiles.GetValueOrDefault(session, []); stagedFiles[session] = Attachments; }
                submittedDrafts.Remove(session);
                submittedFiles.Remove(session);
            }
            if (!value.Running) { QueuedFollowUp = null; queuedDrafts.Remove(session); }
        }
        catch (OperationCanceledException) { }
        catch (Exception ex)
        {
            if (current == generation)
                ConnectionError = "连接暂时中断，正在重新读取会话：" + ex.Message;
        }
        if (current == generation) Notify();
    }

    public async Task<string?> CreateAsync(string projectId, CancellationToken cancellationToken = default)
    {
        if (Busy) return null;
        Busy = true; OperationError = null; var current = generation; Notify();
        try
        {
            var id = await client.CreateAsync(projectId, cancellationToken);
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
        if (UncertainSend || Busy || !CanSend || projectId is not { } project || sessionId is not { } session) return;
        var text = Draft; var id = Guid.NewGuid(); var current = generation;
        var files = Attachments; submittedFiles[session] = files;
        Busy = true; OperationError = null; pending = (id, text); pendingSends[session] = pending.Value; submittedDrafts[session] = pending.Value;
        Notify();
        try
        {
            await client.SendFilesAsync(project, session, id, text, files.Select(f => f.Path).ToArray(), cancellationToken);
            pendingSends.Remove(session);
            stagedFiles[session] = [];
            if (drafts.GetValueOrDefault(session) == text) drafts[session] = "";
            if (generation != current) return;
            if (Draft == text) Draft = "";
            Attachments = [];
            pending = null;
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            if (generation != current) return;
            UncertainSend = true;
            OperationError = "发送结果尚未确认。正在读取服务器状态；不会自动重发。" + ex.Message;
        }
        if (generation == current) { Busy = false; await RefreshAsync(cancellationToken); }
    }

    public void AcknowledgeUncertainSend()
    {
        if (sessionId is { } session) { pendingSends.Remove(session); uncertainQueues.Remove(session); }
        UncertainSend = false; pending = null; OperationError = null; Notify();
    }

    public async Task AttachAsync(string path, CancellationToken cancellationToken = default)
    {
        if (!CanAttach || string.IsNullOrWhiteSpace(path) || projectId is not { } project || sessionId is not { } session) return;
        var current = generation; Busy = true; OperationError = null; Notify();
        try
        {
            var file = await client.AttachAsync(project, session, path, cancellationToken);
            if (!file.Path.StartsWith("uploads/", StringComparison.Ordinal) || string.IsNullOrWhiteSpace(file.Name)) throw new InvalidDataException("Invalid attachment response");
            if (current != generation) return;
            Attachments = Attachments.Where(f => f.Path != file.Path).Append(file).ToArray(); stagedFiles[session] = Attachments;
        }
        catch (Exception ex) { if (current == generation) OperationError = "附件未能确认添加；不会自动重试。\n" + ex.Message; }
        finally { if (current == generation) { Busy = false; Notify(); } }
    }
    public void RemoveAttachment(string path)
    {
        if (Busy || UncertainSend) return;
        Attachments = Attachments.Where(f => f.Path != path).ToArray();
        if (sessionId != null) stagedFiles[sessionId] = Attachments;
        Notify();
    }
    public async Task QueueAsync(CancellationToken cancellationToken = default)
    {
        if (!CanQueue || projectId is not { } project || sessionId is not { } session) return;
        var current = generation; var text = Draft; var files = Attachments.Select(f => f.Path).ToArray();
        Busy = true; OperationError = null; Notify();
        try
        {
            await client.EnqueueFilesAsync(project, session, Guid.NewGuid(), text, files, cancellationToken);
            queuedDrafts[session] = text; stagedFiles[session] = [];
            if (drafts.GetValueOrDefault(session) == text) drafts[session] = "";
            if (current != generation) return;
            QueuedFollowUp = text; if (Draft == text) Draft = ""; Attachments = [];
        }
        catch (Exception ex)
        {
            uncertainQueues.Add(session);
            if (current == generation) OperationError = "后续未能确认排队；请核对会话，不会自动重试。\n" + ex.Message;
        }
        finally { if (current == generation) { Busy = false; Notify(); } }
    }
    public bool Prefill(string text, bool append = false)
    {
        if (sessionId == null || Busy || UncertainSend || Snapshot is not { ReadOnly: false }) return false;
        Draft = append && !string.IsNullOrWhiteSpace(Draft) ? Draft + "\n\n" + text : text; Notify(); return true;
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
    public Task ApproveAsync(ConversationApproval approval, bool allowed, CancellationToken cancellationToken = default) =>
        ActAsync((project, session, token) => client.ApproveAsync(project, session, approval.ApprovalId, allowed, token), cancellationToken);
    public Task SelectModelAsync(string modelId, CancellationToken cancellationToken = default) =>
        ActAsync((project, session, token) => client.SetModelAsync(project, session, modelId, token), cancellationToken);

    public async Task OlderAsync(CancellationToken cancellationToken = default)
    {
        RevealedExcerpt = null;
        if (projectId is not { } project || sessionId is not { } session) return;
        var cursorSeq = (ShowingHistory ? History : Snapshot)?.NextBeforeSeq;
        if (cursorSeq is null) return;
        var current = generation;
        try
        {
            History = await client.SnapshotAsync(project, session, cursorSeq, cancellationToken);
            if (current == generation) ShowingHistory = true;
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (current == generation) OperationError = ex.Message; }
        if (current == generation) Notify();
    }

    public void Latest() { RevealedExcerpt = null; ShowingHistory = false; History = null; Notify(); }

    public async Task OpenQuestionAsync(ConversationOutlineEntry entry, CancellationToken cancellationToken = default)
    {
        RevealedExcerpt = null;
        if (projectId is not { } project || sessionId is not { } session) return;
        var current = generation;
        try
        {
            var page = await client.SnapshotAsync(project, session, entry.BeforeSeq, cancellationToken);
            if (current != generation) return;
            var index = WorkspaceOutlineModel.QuestionItemIndex(entry.UserIndex, page.UserOffset, page.Items)
                ?? throw new InvalidOperationException("问题位置已变化，请刷新大纲后重试。");
            History = page; ShowingHistory = true; ScrollTarget = index; ScrollRevision++;
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (current == generation) OperationError = ex.Message; }
        if (current == generation) Notify();
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
        if (Busy || projectId is not { } project || sessionId is not { } session) return;
        var current = generation; Busy = true; OperationError = null; Notify();
        try { await action(project, session, cancellationToken); }
        catch (Exception ex) when (ex is not OperationCanceledException)
        { if (current == generation) OperationError = ex.Message; }
        if (current == generation) { Busy = false; await RefreshAsync(cancellationToken); }
    }

    private void Notify() => Changed?.Invoke();
}

public sealed record ConversationModelOption(string Id, string Label);
