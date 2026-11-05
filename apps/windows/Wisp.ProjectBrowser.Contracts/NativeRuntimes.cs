using System.Text.Json;
using System.Text.Json.Nodes;
namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeRuntimeSnapshot(string ProjectId, string? SessionId, NativePanelContext[] Contexts,
    string[] EnabledIds, string? DefaultContextId, NativeRuntimeInfo[] Runtimes, bool ReadOnly);

public sealed class NativeRuntimesClient(INativeSettingsClient transport)
{
    public async Task<NativeRuntimeSnapshot> ReadAsync(string project, string? session, CancellationToken token = default) =>
        (await CallAsync("read", project, session, new(), token).ConfigureAwait(false))?.Deserialize<NativeRuntimeSnapshot>(ConversationSnapshot.JsonOptions)
        ?? throw new InvalidDataException("Missing runtime snapshot");
    public Task<JsonNode?> CallAsync(string action, string project, string? session, JsonObject args, CancellationToken token = default)
    {
        args["session_id"] = session;
        return transport.InvokeAsync("native_runtimes_" + action, args, project, token);
    }
}
