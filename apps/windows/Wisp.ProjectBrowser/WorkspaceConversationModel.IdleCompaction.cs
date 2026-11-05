using System.Text.Json.Nodes;
using System.Text.Json;
using System.Globalization;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeIdleCompactionPrompt(string ProjectId, string SessionId, string Epoch, ulong Hours);

public static class NativeIdleCompaction
{
    // Session timestamps below this threshold are ranking stubs, matching WebView.
    public const long MinimumActivityTimestamp = 1_000_000_000_000;
    public static ulong Hours(JsonNode? preferences) => preferences is JsonObject root && root["semantic_compact_idle_hours"] is JsonValue value
        && value.GetValueKind() == JsonValueKind.Number
        && ulong.TryParse(value.ToJsonString(), NumberStyles.None, CultureInfo.InvariantCulture, out var hours) ? hours : 24;
    public static bool ShouldPrompt(ulong hours, long lastActivity, long now, bool hasTranscript) => hours > 0
        && hasTranscript && lastActivity >= MinimumActivityTimestamp && now >= lastActivity
        && (UInt128)((Int128)now - lastActivity) >= (UInt128)hours * 3_600_000;
}

public sealed partial class WorkspaceConversationModel
{
    private readonly HashSet<(string Project, string Session)> idleCompactionDismissed = [];
    private (int Count, string? Text, long? Timestamp) idleCompactionUser;
    private static (int, string?, long?) IdleUser(ConversationSnapshot snapshot)
    {
        var user = snapshot.Items.LastOrDefault(row => row.Role == "user");
        return (snapshot.Items.Count(row => row.Role == "user"), user?.Text, user?.Timestamp);
    }
    public NativeIdleCompactionPrompt? IdleCompactionPrompt { get; private set; }
    public bool CanShowIdleCompaction => IdleCompactionPrompt is { } prompt && active && !AssistantMode
        && projectId == prompt.ProjectId && sessionId == prompt.SessionId && Snapshot?.Epoch == prompt.Epoch
        && CanOpenCompaction && IdleUser(Snapshot) == idleCompactionUser && Compaction?.Visible != true && !Handoff.Visible;

    private async Task ReadIdleCompactionPromptAsync(long lastActivity, int owner, CancellationToken token)
    {
        if (settings == null || AssistantMode || !CanOpenCompaction || Snapshot is not { } source
            || lastActivity < NativeIdleCompaction.MinimumActivityTimestamp
            || idleCompactionDismissed.Contains((source.ProjectId, source.SessionId))
            || !source.Items.Any(row => row.Role == "user" && row.Kind != "checkpoint")) return;
        try
        {
            using var deadline = CancellationTokenSource.CreateLinkedTokenSource(token);
            deadline.CancelAfter(TimeSpan.FromSeconds(10));
            var preferences = await settings.InvokeAsync("get_settings", new(), source.ProjectId, deadline.Token);
            if (owner != generation || token.IsCancellationRequested || !CanOpenCompaction || Snapshot?.Epoch != source.Epoch
                || Compaction?.Visible == true || Handoff.Visible || preferences is not JsonObject) return;
            if (IdleUser(Snapshot) != IdleUser(source)) return;
            var hours = NativeIdleCompaction.Hours(preferences);
            if (NativeIdleCompaction.ShouldPrompt(hours, lastActivity, DateTimeOffset.UtcNow.ToUnixTimeMilliseconds(), true))
            {
                IdleCompactionPrompt = new(source.ProjectId, source.SessionId, source.Epoch, hours);
                idleCompactionUser = IdleUser(source);
                Notify();
            }
        }
        catch (OperationCanceledException) { }
        // Optional reopen advice must not block conversation loading or invent a
        // preference after a failed read. A later activation may try reading again.
        catch (Exception) { }
    }
    public void DismissIdleCompaction()
    {
        if (IdleCompactionPrompt is not { } prompt) return;
        idleCompactionDismissed.Add((prompt.ProjectId, prompt.SessionId));
        IdleCompactionPrompt = null; Notify();
    }
    public bool AcceptIdleCompaction()
    {
        if (!CanShowIdleCompaction) return false;
        DismissIdleCompaction();
        return OpenCompaction(semantic: true);
    }
}
