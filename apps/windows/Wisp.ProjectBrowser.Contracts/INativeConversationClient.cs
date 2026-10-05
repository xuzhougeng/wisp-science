using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

/// <summary>Platform-independent conversation boundary for WinUI view models.
/// Reads may reconnect; never automatically replay Send/Create/Approve after an ambiguous failure.</summary>
public interface INativeConversationClient
{
    Task<NativeInboxEntry[]> InboxAsync(string projectId, CancellationToken cancellationToken = default);
    Task MarkSeenAsync(string projectId, string sessionId, CancellationToken cancellationToken = default);
    Task<NativeTrajectory> TrajectoryAsync(string projectId, string sessionId, CancellationToken cancellationToken = default);
    Task<string> TrajectoryHtmlAsync(string projectId, string sessionId, CancellationToken cancellationToken = default);
    Task<ConversationOutlineEntry[]> OutlineAsync(string projectId, string sessionId, CancellationToken cancellationToken = default);
    Task<string> CreateAsync(string projectId, CancellationToken cancellationToken = default);
    Task<string> CreateWithAgentAsync(string projectId, string? agentId, CancellationToken cancellationToken = default)
        => agentId == null ? CreateAsync(projectId, cancellationToken) : throw new NotSupportedException("ACP creation is not supported by this client");
    Task AcpPermissionAsync(string projectId, string sessionId, string requestId, string? optionId, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("ACP permissions are not supported by this client");
    Task AcpAnswerAsync(string projectId, string sessionId, string requestId, string answer, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("ACP questions are not supported by this client");
    Task AcpSettingAsync(string projectId, string sessionId, JsonObject change, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("ACP settings are not supported by this client");
    Task<ConversationSnapshot> SnapshotAsync(string projectId, string sessionId, long? beforeSeq = null, CancellationToken cancellationToken = default);
    Task SendAsync(string projectId, string sessionId, Guid requestId, string message, CancellationToken cancellationToken = default);
    Task SendFilesAsync(string projectId, string sessionId, Guid requestId, string message, string[] attachments, CancellationToken cancellationToken = default)
        => attachments.Length == 0 ? SendAsync(projectId, sessionId, requestId, message, cancellationToken) : throw new NotSupportedException("Attachments are not supported by this client");
    Task SendContextAsync(string projectId, string sessionId, Guid requestId, string message, string[] attachments,
        NativeComposerReference[] references, CancellationToken cancellationToken = default)
        => references.Length == 0 ? SendFilesAsync(projectId, sessionId, requestId, message, attachments, cancellationToken)
            : throw new NotSupportedException("Composer references are not supported by this client");
    Task<ComposerAttachment> AttachAsync(string projectId, string sessionId, string path, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("Attachments are not supported by this client");
    Task EnqueueAsync(string projectId, string sessionId, Guid requestId, string message, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("Follow-ups are not supported by this client");
    Task EnqueueFilesAsync(string projectId, string sessionId, Guid requestId, string message, string[] attachments, CancellationToken cancellationToken = default)
        => attachments.Length == 0 ? EnqueueAsync(projectId, sessionId, requestId, message, cancellationToken) : throw new NotSupportedException("Attachments are not supported by this client");
    Task EnqueueContextAsync(string projectId, string sessionId, Guid requestId, string message, string[] attachments,
        NativeComposerReference[] references, CancellationToken cancellationToken = default)
        => references.Length == 0 ? EnqueueFilesAsync(projectId, sessionId, requestId, message, attachments, cancellationToken)
            : throw new NotSupportedException("Composer references are not supported by this client");
    Task StopAsync(string projectId, string sessionId, CancellationToken cancellationToken = default);
    Task ApproveAsync(string projectId, string sessionId, string approvalId, bool approved, CancellationToken cancellationToken = default);
    Task SetModelAsync(string projectId, string sessionId, string modelId, CancellationToken cancellationToken = default);
    Task SetPlanModeAsync(string projectId, string sessionId, bool enabled, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("Plan mode is not supported by this client");
    Task SetFastModeAsync(string projectId, string sessionId, string modelId, bool enabled, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("Fast mode is not supported by this client");
}
public sealed record NativeInboxEntry(string Id, string ProjectId, string ProjectName, string Title, long Ts, long ActivityAt, string Status);
public sealed record ConversationOutlineEntry(int UserIndex, string Text, long? BeforeSeq, long? SentAt, long? ResponseAt);
public sealed record ConversationItem(string Role, string Text, string? ToolName, string? Input, bool? Ok, string? Status, string[]? Attachments = null,
    ulong? DurationMs = null, string? ModelName = null, long? Timestamp = null, ConversationRun? Run = null,
    string? CallId = null, string? Kind = null, string? Locations = null, NativePlanStep[]? PlanSteps = null, NativePlanProposal? Proposal = null);
public sealed record NativePlanStep(string Status, string Content);
public sealed record NativePlanEntry(string Content, string Status, string Priority);
public sealed record NativePlanProposal(NativePlanEntry[] Entries, string Source)
{
    [System.Text.Json.Serialization.JsonIgnore]
    public bool Valid => Source is "native" or "acp" && Entries is { Length: > 0 }
        && Entries.All(row => row != null && !string.IsNullOrWhiteSpace(row.Content)
            && row.Status is "pending" or "in_progress" or "completed" && row.Priority is "low" or "medium" or "high");
}
public sealed record ConversationRun(string Id, string Status, int? OwnerIndex, bool NeedsReview);
public sealed record ComposerAttachment(string Path, string Name);
public sealed record ConversationApproval(string ApprovalId, string FrameId, string Message, string Tool, string Preview);
public sealed record ConversationFastMode(bool Enabled, bool Inherited);
public sealed record NativeAcpPermissionOption(string Id, string Name, string Kind);
public sealed record NativeAcpPermission(string RequestId, string FrameId, string Title, string Preview, NativeAcpPermissionOption[] Options);
public sealed record NativeAcpInteractions(NativeAcpPermission[] Permissions, string[] QuestionIds);
public sealed record ConversationSnapshot(string Schema, string Epoch, ulong Sequence, string ProjectId, string SessionId,
    ConversationItem[] Items, long? NextBeforeSeq, bool Running, bool Stopping, bool ReadOnly, string ModelId,
    string? RequestId, string? Error, ConversationApproval[] Approvals, int? UserOffset = null, bool? PlanMode = null, ConversationFastMode? FastMode = null,
    bool? ComposerReferences = null, string[]? FollowUps = null, NativeAcpInteractions? Acp = null, string? AcpAgentId = null,
    NativeAcpSessionState? AcpState = null, NativeHistoryState? HistoryState = null, NativeQueueSnapshot? Queue = null,
    NativeRun[]? RunCards = null, bool? RunReviewSupported = null, string? ActivityStatus = null)
{
    public const string SchemaId = "wisp.native-conversations.v1";
    public static readonly JsonSerializerOptions JsonOptions = new() { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower };
    public static ConversationSnapshot Decode(JsonNode? node, string projectId, string sessionId)
    {
        var value = node?.Deserialize<ConversationSnapshot>(JsonOptions) ?? throw new InvalidDataException("Missing conversation snapshot");
        if (value.Schema != SchemaId || string.IsNullOrEmpty(value.Epoch) || value.Sequence == 0
            || value.ProjectId != projectId || value.SessionId != sessionId || value.Items is null || value.Approvals is null
            || value.Approvals.Any(a => a.FrameId != sessionId)
            || value.AcpState != null && value.AcpState.FrameId != sessionId
            || value.Queue != null && !value.Queue.Valid
            || value.RunCards != null && (value.RunCards.Select(run => run.Id).Distinct().Count() != value.RunCards.Length
                || value.RunCards.Any(run => run.FrameId != sessionId || string.IsNullOrEmpty(run.Id)))
            || value.HistoryState != null && (string.IsNullOrEmpty(value.HistoryState.Revision) || value.HistoryState.Turns == null
                || value.HistoryState.Turns.Where((turn, index) => turn.UserIndex != index || turn.UserSeq < 0 || string.IsNullOrEmpty(turn.Digest)).Any())
            || value.Acp != null && (value.Acp.Permissions == null || value.Acp.QuestionIds == null
                || value.Acp.Permissions.Any(permission => permission.FrameId != sessionId || string.IsNullOrWhiteSpace(permission.RequestId)
                    || permission.Options == null || permission.Options.Any(option => string.IsNullOrEmpty(option.Id)))))
            throw new InvalidDataException("Conversation snapshot identity mismatch");
        return value;
    }
}
/// <summary>One cursor per selected session. Replace the transcript when accepted;
/// never append snapshot text. Historical pages use their own view state.</summary>
public sealed class ConversationCursor(string projectId, string sessionId)
{
    private string? epoch;
    private ulong sequence;
    private readonly HashSet<string> retiredEpochs = [];
    public bool TryAccept(ConversationSnapshot value)
    {
        if (value.ProjectId != projectId || value.SessionId != sessionId || value.Schema != ConversationSnapshot.SchemaId
            || string.IsNullOrEmpty(value.Epoch) || value.Sequence == 0 || retiredEpochs.Contains(value.Epoch)) return false;
        if (value.Epoch == epoch && value.Sequence <= sequence) return false;
        if (epoch is not null && epoch != value.Epoch) retiredEpochs.Add(epoch);
        epoch = value.Epoch; sequence = value.Sequence; return true;
    }
}
public sealed class NativeConversationClient(INativeSettingsClient transport) : INativeConversationClient
{
    public async Task AcpSettingAsync(string projectId, string sessionId, JsonObject change, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_acp_setting", new() { ["session_id"] = sessionId, ["change"] = change.DeepClone() },
            projectId, cancellationToken).ConfigureAwait(false);
    public async Task<string> CreateWithAgentAsync(string projectId, string? agentId, CancellationToken cancellationToken = default) =>
        (await transport.InvokeAsync("native_conversation_create", new() { ["acp_agent_id"] = agentId }, projectId, cancellationToken).ConfigureAwait(false))?.GetValue<string>()
            ?? throw new InvalidDataException("Missing new session ID");
    public async Task AcpPermissionAsync(string projectId, string sessionId, string requestId, string? optionId, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_acp_permission", new() { ["session_id"] = sessionId, ["request_id"] = requestId,
            ["option_id"] = optionId }, projectId, cancellationToken).ConfigureAwait(false);
    public async Task AcpAnswerAsync(string projectId, string sessionId, string requestId, string answer, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_acp_answer", new() { ["session_id"] = sessionId, ["request_id"] = requestId,
            ["answer"] = answer }, projectId, cancellationToken).ConfigureAwait(false);
    public async Task<NativeInboxEntry[]> InboxAsync(string projectId, CancellationToken cancellationToken = default) =>
        (await transport.InvokeAsync("native_conversation_inbox", new(), projectId, cancellationToken).ConfigureAwait(false))?.Deserialize<NativeInboxEntry[]>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing inbox");
    public async Task MarkSeenAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_seen", new() { ["session_id"] = sessionId }, projectId, cancellationToken).ConfigureAwait(false);
    public async Task<NativeTrajectory> TrajectoryAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
        NativeTrajectory.Decode(await transport.InvokeAsync("native_conversation_trajectory", new() { ["session_id"] = sessionId }, projectId, cancellationToken).ConfigureAwait(false), sessionId);
    public async Task<string> TrajectoryHtmlAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
        (await transport.InvokeAsync("native_conversation_trajectory_html", new() { ["session_id"] = sessionId }, projectId, cancellationToken).ConfigureAwait(false))?.GetValue<string>()
            ?? throw new InvalidDataException("Missing trajectory export");
    public async Task<ConversationOutlineEntry[]> OutlineAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
        (await transport.InvokeAsync("native_conversation_outline", new() { ["session_id"] = sessionId }, projectId, cancellationToken).ConfigureAwait(false))
            ?.Deserialize<ConversationOutlineEntry[]>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing conversation outline");
    public async Task<string> CreateAsync(string projectId, CancellationToken cancellationToken = default) =>
        (await transport.InvokeAsync("native_conversation_create", new(), projectId, cancellationToken).ConfigureAwait(false))?.GetValue<string>()
        ?? throw new InvalidDataException("Missing new session ID");
    public async Task<ConversationSnapshot> SnapshotAsync(string projectId, string sessionId, long? beforeSeq = null, CancellationToken cancellationToken = default) =>
        ConversationSnapshot.Decode(await transport.InvokeAsync("native_conversation_snapshot",
            new() { ["session_id"] = sessionId, ["before_seq"] = beforeSeq }, projectId, cancellationToken).ConfigureAwait(false), projectId, sessionId);
    public async Task SendAsync(string projectId, string sessionId, Guid requestId, string message, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_send", new() { ["session_id"] = sessionId, ["request_id"] = requestId.ToString(), ["message"] = message }, projectId, cancellationToken).ConfigureAwait(false);
    public async Task SendFilesAsync(string projectId, string sessionId, Guid requestId, string message, string[] attachments, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_send", ComposerArgs(sessionId, requestId, message, attachments), projectId, cancellationToken).ConfigureAwait(false);
    public async Task SendContextAsync(string projectId, string sessionId, Guid requestId, string message, string[] attachments,
        NativeComposerReference[] references, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_send", ReferenceArgs(sessionId, requestId, message, attachments, references), projectId, cancellationToken).ConfigureAwait(false);
    private static JsonObject ReferenceArgs(string sessionId, Guid requestId, string message, string[] attachments, NativeComposerReference[] references)
    {
        var args = ComposerArgs(sessionId, requestId, message, attachments);
        if (references.Length > 0) args["references"] = new JsonArray(references.Select(reference => (JsonNode)reference.ToJson()).ToArray());
        return args;
    }
    public async Task EnqueueContextAsync(string projectId, string sessionId, Guid requestId, string message, string[] attachments,
        NativeComposerReference[] references, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_conversation_enqueue", ReferenceArgs(sessionId, requestId, message, attachments, references), projectId, cancellationToken).ConfigureAwait(false);
        if (node?["queued"]?.GetValue<bool>() != true) throw new InvalidDataException("Follow-up was not queued");
    }
    private static JsonObject ComposerArgs(string sessionId, Guid requestId, string message, string[] attachments) => new()
    {
        ["session_id"] = sessionId, ["request_id"] = requestId.ToString(), ["message"] = message,
        ["attachments"] = JsonSerializer.SerializeToNode(attachments)
    };
    public async Task EnqueueFilesAsync(string projectId, string sessionId, Guid requestId, string message, string[] attachments, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_conversation_enqueue", ComposerArgs(sessionId, requestId, message, attachments), projectId, cancellationToken).ConfigureAwait(false);
        if (node?["queued"]?.GetValue<bool>() != true) throw new InvalidDataException("Follow-up was not queued");
    }
    public async Task<ComposerAttachment> AttachAsync(string projectId, string sessionId, string path, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_conversation_attach", new() { ["session_id"] = sessionId, ["path"] = path }, projectId, cancellationToken).ConfigureAwait(false)
            ?? throw new InvalidDataException("Missing attachment");
        return node.Deserialize<ComposerAttachment>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing attachment");
    }
    public async Task EnqueueAsync(string projectId, string sessionId, Guid requestId, string message, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_conversation_enqueue", new() { ["session_id"] = sessionId, ["request_id"] = requestId.ToString(), ["message"] = message }, projectId, cancellationToken).ConfigureAwait(false)
            ?? throw new InvalidDataException("Missing queued follow-up");
        if (node["queued"]?.GetValue<bool>() != true) throw new InvalidDataException("Follow-up was not queued");
    }
    public async Task StopAsync(string projectId, string sessionId, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_stop", new() { ["session_id"] = sessionId }, projectId, cancellationToken).ConfigureAwait(false);
    public async Task ApproveAsync(string projectId, string sessionId, string approvalId, bool approved, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_approve", new() { ["session_id"] = sessionId, ["approval_id"] = approvalId, ["approved"] = approved }, projectId, cancellationToken).ConfigureAwait(false);
    public async Task SetModelAsync(string projectId, string sessionId, string modelId, CancellationToken cancellationToken = default) =>
        await transport.InvokeAsync("native_conversation_model", new() { ["session_id"] = sessionId, ["model_id"] = modelId }, projectId, cancellationToken).ConfigureAwait(false);
    public async Task SetPlanModeAsync(string projectId, string sessionId, bool enabled, CancellationToken cancellationToken = default)
    {
        var result = await transport.InvokeAsync("native_conversation_plan", new() { ["session_id"] = sessionId, ["enabled"] = enabled }, projectId, cancellationToken).ConfigureAwait(false);
        if (result?.GetValue<bool>() != enabled) throw new InvalidDataException("Plan mode change was not confirmed; refresh the conversation before trying again");
    }
    public async Task SetFastModeAsync(string projectId, string sessionId, string modelId, bool enabled, CancellationToken cancellationToken = default)
    {
        var result = await transport.InvokeAsync("native_conversation_fast", new() { ["session_id"] = sessionId, ["model_id"] = modelId, ["enabled"] = enabled }, projectId, cancellationToken).ConfigureAwait(false);
        if (result?["enabled"]?.GetValue<bool>() != enabled || result?["inherited"] is not JsonValue inherited || !inherited.TryGetValue<bool>(out _))
            throw new InvalidDataException("Fast mode change was not confirmed; refresh the conversation before trying again");
    }
}
