using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Resolves the restored desktop workspace once; each page owns the returned ID.
public static class NativeSettingsWorkspace
{
    public static async Task<string> ResolveAsync(INativeSettingsClient client, string? projectId)
    {
        if (projectId == null)
        {
            var info = await client.InvokeAsync("get_project_info", new());
            if (info is not JsonObject || info["id"] is not JsonValue id || !id.TryGetValue<string>(out projectId))
                throw new InvalidDataException("The host did not return a workspace identity.");
        }
        if (string.IsNullOrWhiteSpace(projectId)) throw new InvalidDataException("The workspace identity is empty.");
        return projectId;
    }
}
