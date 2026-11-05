using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Read-only head projection, independent of usage-panel lifetime and full-history paging.
public sealed class NativeModelContextModel(INativeSettingsClient? transport)
{
    private ConversationSnapshot? snapshot;
    private int generation;
    private string? revision, refreshKey;
    private int? liveFrom;
    public event Action? Changed;
    public bool Available => transport != null && snapshot != null;
    public bool Active { get; private set; }
    public bool Busy { get; private set; }
    public NativeModelContext? View { get; private set; }
    public string? Error { get; private set; }
    public bool ChangedDuringRead { get; private set; }

    public void Bind(ConversationSnapshot? next, bool available = true)
    {
        if (!available || next?.AcpAgentId != null || next?.ModelId.StartsWith("acp:", StringComparison.Ordinal) == true) next = null;
        var sameScope = snapshot?.ProjectId == next?.ProjectId && snapshot?.SessionId == next?.SessionId;
        var sameHost = snapshot?.Epoch == next?.Epoch && snapshot?.ModelId == next?.ModelId;
        var nextRevision = next == null ? null : Revision(next);
        var nextRefresh = next == null ? null : next.Epoch + "/" + next.ModelId + "/"
            + next.Items.LastOrDefault(row => row.Role is "usage" or "compaction")?.Text;
        var shouldRefresh = refreshKey != nextRefresh || next?.Running == false && (revision != nextRevision || snapshot?.Running == true);
        snapshot = next; revision = nextRevision; refreshKey = nextRefresh;
        if (!sameScope || next == null) { Clear(); Active = false; Changed?.Invoke(); return; }
        if (!sameHost) Clear();
        if (Active && !Busy && Error == null && (View == null || shouldRefresh)) _ = LoadAsync();
    }
    private static string Revision(ConversationSnapshot value) => value.Epoch + "/" + value.ModelId + "/" + value.UserOffset + "/"
        + JsonSerializer.Serialize(value.Items);
    private void Clear()
    {
        generation++; Busy = false; View = null; liveFrom = null; Error = null; ChangedDuringRead = false;
    }
    public async Task SelectAsync(bool model, CancellationToken token = default)
    {
        if (model == Active || model && !Available) return;
        Clear(); Active = model; Changed?.Invoke();
        if (Active) await LoadAsync(token);
    }
    public async Task RetryAsync(CancellationToken token = default)
    {
        if (!Active || Busy) return;
        Clear(); await LoadAsync(token);
    }
    private async Task LoadAsync(CancellationToken token = default)
    {
        if (!Available || !Active || Busy || Error != null) return;
        var current = generation; Busy = true; Changed?.Invoke();
        try
        {
            for (var attempt = 0; attempt < 3; attempt++)
            {
                var source = snapshot!; var requestedRevision = revision;
                var nextUser = (source.UserOffset ?? 0) + source.Items.Count(row => row.Role == "user" && row.Kind != "checkpoint");
                var response = await transport!.InvokeAsync("native_conversation_context", new() { ["session_id"] = source.SessionId }, source.ProjectId, token);
                if (current != generation || token.IsCancellationRequested) return;
                var view = NativeModelContext.Decode(response, source.ProjectId, source.SessionId);
                if (requestedRevision != revision) continue;
                View = view;
                // A view opened during a running turn must keep replacing that
                // turn from the live snapshot, not freeze the partial persisted answer.
                liveFrom = source.Running && nextUser > (source.UserOffset ?? 0) ? nextUser - 1
                    : Math.Max(nextUser, (view.State.InContextFromUserIndex ?? 0) + view.Items.Count(row => row.Role == "user"));
                return;
            }
            View = null; liveFrom = null; ChangedDuringRead = true; Error = "The context changed while loading.";
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (current == generation) { View = null; liveFrom = null; Error = ex.Message; } }
        finally { if (current == generation) { Busy = false; Changed?.Invoke(); } }
    }
    public ConversationSnapshot? DisplaySnapshot => !Active || snapshot == null ? null : snapshot with
    {
        Items = WithLiveTurns(View?.Items ?? [], snapshot.Items, snapshot.UserOffset ?? 0, liveFrom, View?.State.InContextFromUserIndex ?? 0),
        UserOffset = View?.State.InContextFromUserIndex ?? 0, NextBeforeSeq = null,
        HistoryState = null, Branches = null, RunCards = null
    };
    public static ConversationItem[] WithLiveTurns(ConversationItem[] persisted, ConversationItem[] live, int userOffset, int? from, int persistedUserOffset = 0)
    {
        if (from == null) return persisted;
        var user = userOffset;
        var start = Array.FindIndex(live, row => row.Role == "user" && row.Kind != "checkpoint" && user++ >= from);
        if (start < 0) return persisted;
        var persistedUser = persistedUserOffset;
        var end = Array.FindIndex(persisted, row => row.Role == "user" && row.Kind != "checkpoint" && persistedUser++ >= from);
        return persisted.Take(end < 0 ? persisted.Length : end).Concat(live.Skip(start).Where(row => row.Role is "assistant" or "tool" or "reasoning" or "acp_tool"
            || row.Role == "user" && row.Kind != "checkpoint")).ToArray();
    }
}
