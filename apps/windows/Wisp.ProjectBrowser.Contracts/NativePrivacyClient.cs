using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record PrivacyMode(
    [property: JsonPropertyName("active"), JsonRequired] bool Active,
    [property: JsonPropertyName("project_ids"), JsonRequired] string[] ProjectIds);

/// <summary>
/// Privacy list written by the WebView. The read carries no project id and is not retried.
/// </summary>
public interface INativePrivacyClient
{
    Task<PrivacyMode> GetAsync(CancellationToken cancellationToken = default);
}

public sealed class NativePrivacyClient(INativeSettingsClient transport) : INativePrivacyClient
{
    public async Task<PrivacyMode> SaveAsync(PrivacyMode mode, CancellationToken token = default)
    {
        var args = JsonSerializer.SerializeToNode(mode)!.AsObject();
        var result = await transport.InvokeAsync("set_privacy_mode", args, null, token).ConfigureAwait(false);
        return result?.Deserialize<PrivacyMode>() is { ProjectIds: not null } saved
            ? saved : throw new InvalidDataException("Missing saved privacy mode");
    }
    public async Task<PrivacyMode> GetAsync(CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("get_privacy_mode", new JsonObject(), null, cancellationToken).ConfigureAwait(false)
            ?? throw new InvalidDataException("Missing privacy mode");
        return node.Deserialize<PrivacyMode>(ConversationSnapshot.JsonOptions) is { ProjectIds: not null } saved
            ? saved : throw new InvalidDataException("Missing privacy mode");
    }
}
