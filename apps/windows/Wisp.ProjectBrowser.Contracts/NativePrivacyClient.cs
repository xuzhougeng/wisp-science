using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record PrivacyMode(
    [property: JsonPropertyName("active")] bool Active,
    [property: JsonPropertyName("project_ids")] string[] ProjectIds);

/// <summary>
/// Privacy list written by the WebView. The read carries no project id and is not retried.
/// </summary>
public interface INativePrivacyClient
{
    Task<PrivacyMode> GetAsync(CancellationToken cancellationToken = default);
}

public sealed class NativePrivacyClient(INativeSettingsClient transport) : INativePrivacyClient
{
    public async Task<PrivacyMode> GetAsync(CancellationToken cancellationToken = default)
    {
        var node = await transport.InvokeAsync("get_privacy_mode", new JsonObject(), null, cancellationToken).ConfigureAwait(false)
            ?? throw new InvalidDataException("Missing privacy mode");
        return node.Deserialize<PrivacyMode>(ConversationSnapshot.JsonOptions) ?? throw new InvalidDataException("Missing privacy mode");
    }
}
