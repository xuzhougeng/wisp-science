using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeSearchItem(string Kind, string Id, string ProjectId, string ProjectName,
    string Title, string Detail, string? SessionId)
{
    public bool Valid => !string.IsNullOrWhiteSpace(Id) && !string.IsNullOrWhiteSpace(ProjectId)
        && Title != null && Detail != null && ProjectName != null && (Kind switch
        {
            "project" => Id == ProjectId && SessionId == null,
            "session" => Id == SessionId,
            "artifact" => !string.IsNullOrWhiteSpace(SessionId),
            _ => false,
        });
}
public sealed record NativeSearchResponse(string Schema, string Query, string? PreferredProjectId, NativeSearchItem[] Items);
public interface INativeSearchClient
{
    Task<NativeSearchResponse> SearchAsync(string query, string? preferredProject, CancellationToken cancellationToken = default);
}
public sealed class NativeSearchClient(INativeSettingsClient transport) : INativeSearchClient
{
    public const string Schema = "wisp.native-search.v1";
    public static NativeSearchResponse Decode(JsonNode? node, string query, string? project)
    {
        var result = node?.Deserialize<NativeSearchResponse>(ConversationSnapshot.JsonOptions)
            ?? throw new InvalidDataException("Missing search results");
        if (result.Schema != Schema || result.Query != query || result.PreferredProjectId != project
            || result.Items == null || result.Items.Length > 44 || result.Items.Any(item => item == null || !item.Valid)
            || result.Items.DistinctBy(item => (item.Kind, item.Id)).Count() != result.Items.Length)
            throw new InvalidDataException("Search result ownership or correlation mismatch");
        return result;
    }
    public async Task<NativeSearchResponse> SearchAsync(string query, string? preferredProject, CancellationToken cancellationToken = default) =>
        Decode(await transport.InvokeAsync("native_workspace_search", new JsonObject { ["query"] = query }, preferredProject, cancellationToken).ConfigureAwait(false), query, preferredProject);
}
