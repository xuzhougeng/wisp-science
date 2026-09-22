using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record ScratchSession(
    [property: JsonPropertyName("project_id")] string ProjectId,
    [property: JsonPropertyName("session_id")] string SessionId);

/// <summary>
/// Hidden scratch chat. Open carries no project id. Close names the scratch
/// project and is not retried. Neither call restores another project.
/// </summary>
public interface INativeScratchClient
{
    Task<ScratchSession> OpenAsync(CancellationToken cancellationToken = default);
    Task<bool> CloseAsync(string projectId, CancellationToken cancellationToken = default);
}

public sealed class NativeScratchClient(INativeSettingsClient transport) : INativeScratchClient
{
    public async Task<ScratchSession> OpenAsync(CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_scratch_open", new JsonObject(), null, cancellationToken).ConfigureAwait(false)
            ?? throw new InvalidDataException("Missing scratch session");
        return node.Deserialize<ScratchSession>() ?? throw new InvalidDataException("Missing scratch session");
    }

    public async Task<bool> CloseAsync(string projectId, CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("native_scratch_close", new JsonObject(), projectId, cancellationToken).ConfigureAwait(false)
            ?? throw new InvalidDataException("Missing scratch close result");
        return node.GetValue<bool>();
    }
}
