using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

// Existing wisp_dto::native_conversations::ContextView and SessionContextState.
public sealed record NativeContextEpoch(ulong Epoch, ulong ParentEpoch, string Strategy, string Kind,
    ulong BeforeTokens, ulong AfterTokens, long InitialHeadSeq, long? FirstKeptSeq, long? CheckpointSeq,
    long? UiEventSeq, bool HasNewTurns);
public sealed record NativeContextCompaction(ulong Epoch, ulong Before, ulong After, string Strategy,
    string? Checkpoint, int? KeptFromUserIndex, bool CanUndo, string? UndoReason);
public sealed record NativeContextState(ulong HeadEpoch, NativeContextEpoch[] ContextEpochs,
    int? InContextFromUserIndex, NativeContextCompaction[] Compactions, ulong[] UndoneEpochs)
{
    public static NativeContextState Decode(JsonNode? node)
    {
        var state = node?.Deserialize<NativeContextState>(ConversationSnapshot.JsonOptions);
        if (node is not JsonObject root || !root.ContainsKey("head_epoch") || state == null
            || state.ContextEpochs == null || state.Compactions == null || state.UndoneEpochs == null
            || state.InContextFromUserIndex < 0 || state.ContextEpochs.Any(e => e == null || e.Epoch == 0 || e.Epoch == e.ParentEpoch)
            || state.ContextEpochs.Select(e => e.Epoch).Distinct().Count() != state.ContextEpochs.Length
            || state.Compactions.Any(c => c == null || c.Epoch == 0 || c.KeptFromUserIndex < 0))
            throw new InvalidDataException("Invalid model context state");
        return state;
    }
}
public sealed record NativeModelContext(string ProjectId, string SessionId, ConversationItem[] Items, NativeContextState State)
{
    public static NativeModelContext Decode(JsonNode? node, string project, string session)
    {
        if (node is not JsonObject root || root["project_id"]?.GetValue<string>() != project
            || root["session_id"]?.GetValue<string>() != session || root["items"] is not JsonArray rows)
            throw new InvalidDataException("Model context scope mismatch");
        var items = rows.Deserialize<ConversationItem[]>(ConversationSnapshot.JsonOptions)!;
        if (items.Any(item => item == null || string.IsNullOrEmpty(item.Role) || item.Text == null))
            throw new InvalidDataException("Invalid model context rows");
        return new(project, session, items.Select(item => item.Role == "user" && item.Kind == "checkpoint"
            ? item with { Role = "checkpoint" } : item).ToArray(), NativeContextState.Decode(root["state"]));
    }
}
