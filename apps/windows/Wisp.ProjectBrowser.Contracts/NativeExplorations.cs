using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeExploration(
    string Id, string CheckpointId, string FrameId, string Name, string Status,
    string WorkspaceDir, string WorkspaceBackend, long ScopeGeneration,
    string WarningsJson, long CreatedAt, long UpdatedAt);

public sealed record NativeExplorationSummary(NativeExploration Exploration, string SourceFrameId,
    [property: JsonRequired] int CheckpointUserIndex, string IsolationSummaryJson)
{
    [JsonIgnore] public bool Valid => Exploration != null && !string.IsNullOrWhiteSpace(Exploration.Id)
        && !string.IsNullOrWhiteSpace(Exploration.FrameId) && !string.IsNullOrWhiteSpace(SourceFrameId)
        && SourceFrameId != Exploration.FrameId && CheckpointUserIndex >= 0
        && Exploration.Name != null && IsolationSummaryJson != null && Exploration.WarningsJson != null
        && Exploration.Status is "creating" or "active" or "promoting" or "failed";
    // Match the shared WebView helper, including its fallback for old malformed metadata.
    [JsonIgnore] public bool FullyIsolated
    {
        get
        {
            try { if (JsonNode.Parse(IsolationSummaryJson) is JsonObject data && data["partial"] is JsonValue flag
                    && flag.TryGetValue<bool>(out var partial) && partial) return false; } catch (JsonException) { }
            try { if (JsonNode.Parse(Exploration.WarningsJson) is JsonArray { Count: > 0 }) return false; } catch (JsonException) { }
            return true;
        }
    }
    [JsonIgnore] public string StatusKey => "exploration.status_" + Exploration.Status;
    [JsonIgnore] public string IsolationKey => FullyIsolated ? "exploration.isolation_full" : "exploration.isolation_partial";
    public BrowserSession Session(string projectId) => new(Exploration.FrameId, projectId, Exploration.Name, Exploration.UpdatedAt, "complete");
}

public sealed record NativeExplorations(string ProjectId, NativeExplorationSummary[] Explorations)
{
    public static NativeExplorations Decode(JsonNode? data, string projectId)
    {
        var result = data?.Deserialize<NativeExplorations>(ConversationSnapshot.JsonOptions);
        if (result?.ProjectId != projectId || result.Explorations == null
            || result.Explorations.Any(row => row?.Valid != true)
            || result.Explorations.Select(row => row.Exploration.Id).Distinct().Count() != result.Explorations.Length
            || result.Explorations.Select(row => row.Exploration.FrameId).Distinct().Count() != result.Explorations.Length)
            throw new InvalidDataException("Exploration list did not match the project");
        return result;
    }
}

public sealed record NativeExplorationOpen(string ProjectId, NativeExplorationSummary Summary)
{
    public static NativeExplorationSummary Decode(JsonNode? data, string projectId, string explorationId, string sessionId)
    {
        var result = data?.Deserialize<NativeExplorationOpen>(ConversationSnapshot.JsonOptions);
        if (result?.ProjectId != projectId || result.Summary?.Valid != true
            || result.Summary.Exploration.Id != explorationId || result.Summary.Exploration.FrameId != sessionId)
            throw new InvalidDataException("Exploration did not match the selected conversation");
        return result.Summary;
    }
}

public sealed record NativeExplorationState(NativeExplorationSummary? Summary,
    [property: JsonRequired] int ActiveCount, [property: JsonRequired] bool SourceFrozen,
    string? RoundSourceFrameId = null, int? RoundUserIndex = null, int CandidateCount = 0)
{
    public bool ValidFor(string sessionId) => ActiveCount >= 0 && CandidateCount >= 0
        && (RoundSourceFrameId == null && RoundUserIndex == null || !string.IsNullOrWhiteSpace(RoundSourceFrameId) && RoundUserIndex >= 0)
        && (Summary == null
        ? SourceFrozen || ActiveCount == 0
        : Summary.Valid && Summary.Exploration.FrameId == sessionId && !SourceFrozen && ActiveCount == 0);
}

public sealed record NativeExplorationStarted(string ProjectId, string SourceSessionId,
    NativeTurnIdentity Target, NativeExplorationSummary Summary)
{
    public static NativeExplorationSummary Decode(JsonNode? data, NativeHistoryTarget target)
    {
        var reply = data?.Deserialize<NativeExplorationStarted>(ConversationSnapshot.JsonOptions);
        if (reply?.ProjectId != target.ProjectId || reply.SourceSessionId != target.SessionId || reply.Target != target.Turn
            || reply.Summary?.Valid != true || reply.Summary.SourceFrameId != target.SessionId
            || reply.Summary.CheckpointUserIndex != target.Turn.UserIndex || reply.Summary.Exploration.Status != "active")
            throw new InvalidDataException("Created exploration did not match the selected response");
        return reply.Summary;
    }
}
