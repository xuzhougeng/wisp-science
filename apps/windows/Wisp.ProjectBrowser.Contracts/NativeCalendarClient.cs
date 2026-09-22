using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record CalendarEntry(
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("kind")] string Kind,
    [property: JsonPropertyName("title")] string Title,
    [property: JsonPropertyName("occurred_at")] long OccurredAt,
    [property: JsonPropertyName("status")] string Status,
    [property: JsonPropertyName("manual")] bool Manual);

public sealed record CalendarHistory(
    [property: JsonPropertyName("entries")] IReadOnlyList<CalendarEntry> Entries,
    [property: JsonPropertyName("truncated")] bool Truncated);

public sealed record CalendarProject(
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("history")] CalendarHistory History,
    [property: JsonPropertyName("error")] string? Error);

/// <summary>
/// Research calendar read. The project list is in the body. The call carries
/// no project id and is not retried.
/// </summary>
public interface INativeCalendarClient
{
    Task<IReadOnlyList<CalendarProject>> ReadAsync(IReadOnlyList<string> projectIds, long from, long until, CancellationToken cancellationToken = default);
}

public sealed class NativeCalendarClient(INativeSettingsClient transport) : INativeCalendarClient
{
    public async Task<IReadOnlyList<CalendarProject>> ReadAsync(IReadOnlyList<string> projectIds, long from, long until, CancellationToken cancellationToken = default)
    {
        var ids = new JsonArray();
        foreach (var id in projectIds) ids.Add(id);
        var node = await transport.InvokeAsync("native_research_calendar", new JsonObject
        {
            ["project_ids"] = ids,
            ["from"] = from,
            ["until"] = until,
        }, null, cancellationToken).ConfigureAwait(false) ?? throw new InvalidDataException("Missing calendar result");
        return node.Deserialize<List<CalendarProject>>() ?? throw new InvalidDataException("Missing calendar result");
    }
}
