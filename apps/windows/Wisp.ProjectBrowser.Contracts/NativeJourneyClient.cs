using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record JourneyEntry(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("occurred_at")] long OccurredAt,
    [property: JsonPropertyName("kind")] string Kind = "",
    [property: JsonPropertyName("summary")] string Summary = "",
    [property: JsonPropertyName("recorded_at")] long? RecordedAt = null,
    [property: JsonPropertyName("source_id")] string? SourceId = null,
    [property: JsonPropertyName("frame_id")] string? FrameId = null,
    [property: JsonPropertyName("status")] string Status = "",
    [property: JsonPropertyName("version_number")] long? VersionNumber = null,
    [property: JsonPropertyName("source_discarded")] bool SourceDiscarded = false,
    [property: JsonPropertyName("manual")] bool Manual = false,
    [property: JsonPropertyName("run_id")] string? RunId = null);

public sealed record JourneyInput(string Title, string Role, string? VersionId, string Confidence);
public sealed record JourneySource(string? RunId, string RunTitle, string RunStatus, string ContextId, long? GeneratedAt, JourneyInput[] Inputs);
public sealed record JourneyArtifact(string VersionId, string Filename, long VersionNumber, JourneySource Source,
    string? Text, string Mime, string? Base64, bool Truncated, string? ContentError);

public sealed record JourneyPage(
    [property: JsonPropertyName("entries")] IReadOnlyList<JourneyEntry> Entries,
    [property: JsonPropertyName("truncated")] bool Truncated)
{
    [JsonPropertyName("recaps")] public IReadOnlyList<JourneyRecap> Recaps { get; init; } = [];
}

// Projections of the existing wisp_dto::ResearchJournalInput, ResearchRecap and
// ResearchGraph contracts. Writes remain scoped to the explicit project.
public sealed record JourneyNote(string Title, string Body, string Category, long OccurredAt);
public sealed record JourneyRecapItem(string Text, int[] Refs);
public sealed record JourneyRecapSource(string Kind, string Id, string Title);
public sealed record JourneyRecap(string Id, long DayStart, string Status, string Headline,
    JourneyRecapItem[] Done, JourneyRecapItem[] Findings, JourneyRecapItem[] Issues, JourneyRecapItem[] Next,
    JourneyRecapSource[] Sources, string Model, long GeneratedAt);
public sealed record JourneyRecapEdit(string Id, string Status, string Headline,
    JourneyRecapItem[] Done, JourneyRecapItem[] Findings, JourneyRecapItem[] Issues, JourneyRecapItem[] Next);
public sealed record JourneyGraphNode(string Id, string Kind, string Title, string? RefId, string MetadataJson);
public sealed record JourneyGraphEdge(string SourceId, string TargetId, string Relation, string MetadataJson);
public sealed record JourneyGraph(JourneyGraphNode[] Nodes, JourneyGraphEdge[] Edges);

/// <summary>
/// Research journey for one explicit project. A lost read is not retried.
/// </summary>
public interface INativeJourneyClient
{
    Task<JourneyGraph> GraphAsync(string projectId, CancellationToken token = default)
        => throw new NotSupportedException("This host does not expose journey relationships.");
    Task<string> AddAsync(string projectId, JourneyNote input, CancellationToken token = default)
        => throw new NotSupportedException("This host does not expose research entries.");
    Task<JourneyRecap> EditRecapAsync(string projectId, JourneyRecapEdit edit, CancellationToken token = default)
        => throw new NotSupportedException("This host does not expose recap editing.");
    Task<NativeRun> RunAsync(string projectId, string runId, CancellationToken token = default)
        => throw new NotSupportedException("This host does not expose journey runs.");
    Task<JourneyPage> ReadAsync(string projectId, long from, long until, CancellationToken cancellationToken = default);
    Task<JourneyArtifact> ArtifactAsync(string projectId, string versionId, CancellationToken token = default)
        => throw new NotSupportedException("This host does not expose journey artifact versions.");
    Task<JourneyArtifact> ThumbnailAsync(string projectId, string versionId, CancellationToken token = default)
        => ArtifactAsync(projectId, versionId, token);
}

public sealed class NativeJourneyClient(INativeSettingsClient transport) : INativeJourneyClient
{
    public async Task<JourneyGraph> GraphAsync(string projectId, CancellationToken token = default)
        => (await transport.InvokeAsync("native_research_journey_graph", new(), projectId, token))?.Deserialize<JourneyGraph>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing research relationships");
    public async Task<string> AddAsync(string projectId, JourneyNote input, CancellationToken token = default)
        => (await transport.InvokeAsync("native_research_journey_add", new() { ["input"] = JsonSerializer.SerializeToNode(input, ConversationSnapshot.JsonOptions) }, projectId, token))?.GetValue<string>()
            ?? throw new InvalidDataException("Missing research entry identity");
    public async Task<JourneyRecap> EditRecapAsync(string projectId, JourneyRecapEdit edit, CancellationToken token = default)
        => (await transport.InvokeAsync("native_research_journey_recap", new() { ["edit"] = JsonSerializer.SerializeToNode(edit, ConversationSnapshot.JsonOptions) }, projectId, token))?.Deserialize<JourneyRecap>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing research recap");
    public async Task<NativeRun> RunAsync(string projectId, string runId, CancellationToken token = default)
    {
        var node = await transport.InvokeAsync("native_research_journey_run", new() { ["run_id"] = runId }, projectId, token);
        var result = node?.Deserialize<NativeRun>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing journey run");
        if (result.Id != runId) throw new InvalidDataException("Journey run identity mismatch");
        return result;
    }
    public async Task<JourneyArtifact> ArtifactAsync(string projectId, string versionId, CancellationToken token = default)
        => await ArtifactReadAsync(projectId, versionId, false, token);
    public async Task<JourneyArtifact> ThumbnailAsync(string projectId, string versionId, CancellationToken token = default)
        => await ArtifactReadAsync(projectId, versionId, true, token);
    private async Task<JourneyArtifact> ArtifactReadAsync(string projectId, string versionId, bool thumbnail, CancellationToken token)
    {
        var args = new JsonObject { ["version_id"] = versionId }; if (thumbnail) args["thumbnail"] = true;
        var node = await transport.InvokeAsync("native_research_journey_artifact", args, projectId, token);
        var result = node?.Deserialize<JourneyArtifact>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing journey artifact");
        if (result.VersionId != versionId || result.Source?.Inputs == null) throw new InvalidDataException("Journey artifact identity mismatch");
        return result;
    }
    public async Task<JourneyPage> ReadAsync(string projectId, long from, long until, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_research_journey", new JsonObject
        {
            ["from"] = from,
            ["until"] = until,
        }, projectId, cancellationToken).ConfigureAwait(false) ?? throw new InvalidDataException("Missing journey result");
        return node.Deserialize<JourneyPage>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing journey result");
    }
}
