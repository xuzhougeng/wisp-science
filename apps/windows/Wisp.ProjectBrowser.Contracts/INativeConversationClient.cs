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
    Task<ConversationSnapshot> SnapshotAsync(string projectId, string sessionId, long? beforeSeq = null, CancellationToken cancellationToken = default);
    Task SendAsync(string projectId, string sessionId, Guid requestId, string message, CancellationToken cancellationToken = default);
    Task<ComposerAttachment> AttachAsync(string projectId, string sessionId, string path, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("对话附件 stays disconnected in WinUI");
    Task EnqueueAsync(string projectId, string sessionId, Guid requestId, string message, CancellationToken cancellationToken = default)
        => throw new NotSupportedException("排队后续 stays disconnected in WinUI");
    Task StopAsync(string projectId, string sessionId, CancellationToken cancellationToken = default);
    Task ApproveAsync(string projectId, string sessionId, string approvalId, bool approved, CancellationToken cancellationToken = default);
    Task SetModelAsync(string projectId, string sessionId, string modelId, CancellationToken cancellationToken = default);
}
public sealed record NativeInboxEntry(string Id, string ProjectId, string ProjectName, string Title, long Ts, long ActivityAt, string Status);
public sealed record ConversationOutlineEntry(int UserIndex, string Text, long? BeforeSeq, long? SentAt, long? ResponseAt);
public sealed record ConversationItem(string Role, string Text, string? ToolName, string? Input, bool? Ok, string? Status, string[]? Attachments = null);
public sealed record ComposerAttachment(string Path, string Name);
public sealed record ConversationApproval(string ApprovalId, string FrameId, string Message, string Tool, string Preview);
public sealed record ConversationSnapshot(string Schema, string Epoch, ulong Sequence, string ProjectId, string SessionId,
    ConversationItem[] Items, long? NextBeforeSeq, bool Running, bool Stopping, bool ReadOnly, string ModelId,
    string? RequestId, string? Error, ConversationApproval[] Approvals, int? UserOffset = null)
{
    public const string SchemaId = "wisp.native-conversations.v1";
    public static readonly JsonSerializerOptions JsonOptions = new() { PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower };
    public static ConversationSnapshot Decode(JsonNode? node, string projectId, string sessionId)
    {
        var value = node?.Deserialize<ConversationSnapshot>(JsonOptions) ?? throw new InvalidDataException("Missing conversation snapshot");
        if (value.Schema != SchemaId || string.IsNullOrEmpty(value.Epoch) || value.Sequence == 0
            || value.ProjectId != projectId || value.SessionId != sessionId || value.Items is null || value.Approvals is null
            || value.Approvals.Any(a => a.FrameId != sessionId))
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
}
