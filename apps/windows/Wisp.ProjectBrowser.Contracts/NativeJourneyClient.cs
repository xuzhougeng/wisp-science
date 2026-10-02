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
    [property: JsonPropertyName("truncated")] bool Truncated);

/// <summary>
/// Research journey for one explicit project. A lost read is not retried.
/// </summary>
public interface INativeJourneyClient
{
    Task<NativeRun> RunAsync(string projectId, string runId, CancellationToken token = default)
        => throw new NotSupportedException("This host does not expose journey runs.");
    Task<JourneyPage> ReadAsync(string projectId, long from, long until, CancellationToken cancellationToken = default);
    Task<JourneyArtifact> ArtifactAsync(string projectId, string versionId, CancellationToken token = default)
        => throw new NotSupportedException("This host does not expose journey artifact versions.");
}

public sealed class NativeJourneyClient(INativeSettingsClient transport) : INativeJourneyClient
{
    public async Task<NativeRun> RunAsync(string projectId, string runId, CancellationToken token = default)
    {
        var node = await transport.InvokeAsync("native_research_journey_run", new() { ["run_id"] = runId }, projectId, token);
        var result = node?.Deserialize<NativeRun>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing journey run");
        if (result.Id != runId) throw new InvalidDataException("Journey run identity mismatch");
        return result;
    }
    public async Task<JourneyArtifact> ArtifactAsync(string projectId, string versionId, CancellationToken token = default)
    {
        var node = await transport.InvokeAsync("native_research_journey_artifact", new() { ["version_id"] = versionId }, projectId, token);
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
        return node.Deserialize<JourneyPage>() ?? throw new InvalidDataException("Missing journey result");
    }
}
