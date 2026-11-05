using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeFolderDeleteResult(
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("folder_id")] string FolderId,
    [property: JsonPropertyName("deleted")] bool Deleted);

public sealed record NativeFolderOrderResult(
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("ids")] string[] Ids);
