using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record JourneyEntry(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("occurred_at")] long OccurredAt);

public sealed record JourneyPage(
    [property: JsonPropertyName("entries")] IReadOnlyList<JourneyEntry> Entries,
    [property: JsonPropertyName("truncated")] bool Truncated);

/// <summary>
/// Research journey for one explicit project. A lost read is not retried.
/// </summary>
public interface INativeJourneyClient
{
    Task<JourneyPage> ReadAsync(string projectId, long from, long until, CancellationToken cancellationToken = default);
}

public sealed class NativeJourneyClient(INativeSettingsClient transport) : INativeJourneyClient
{
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
