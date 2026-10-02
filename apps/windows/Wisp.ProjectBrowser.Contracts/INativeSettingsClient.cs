using System.Diagnostics;
using System.Net;
using System.Net.Http.Headers;
using System.Text;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

/// <summary>
/// UI-independent settings boundary for WinUI 3. A view model passes its project
/// explicitly on every call. Payloads match the shared command catalog, retaining
/// unknown fields when round-tripping configuration. Never persist secrets here.
/// </summary>
public interface INativeSettingsClient
{
    Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null,
        CancellationToken cancellationToken = default);
}

public sealed record NativeSettingsHost(
    [property: JsonPropertyName("schema")] string Schema,
    [property: JsonPropertyName("endpoint")] string Endpoint,
    [property: JsonPropertyName("token")] string Token,
    [property: JsonPropertyName("database")] string Database,
    [property: JsonPropertyName("pid")] uint Pid);

public sealed record NativeSettingsRequest(
    [property: JsonPropertyName("schema")] string Schema,
    [property: JsonPropertyName("id")] string Id,
    [property: JsonPropertyName("project_id")] string? ProjectId,
    [property: JsonPropertyName("command")] string Command,
    [property: JsonPropertyName("args")] JsonObject Arguments);

public static class NativeSettingsProtocol
{
    public const string Schema = "wisp.native-settings.v1";
    public static readonly IReadOnlyList<string> Sections = Array.AsReadOnly(new[] {
        "general", "network", "session", "appearance", "pet", "models", "quick-actions", "workflows",
        "specialists", "memory", "skills", "plugins", "browser", "connections", "channels",
        "credentials", "permissions", "environments", "storage", "usage"
    });

    public static Uri ValidateHost(NativeSettingsHost host, string databasePath)
    {
        if (host.Schema != Schema || !Uri.TryCreate(host.Endpoint, UriKind.Absolute, out var uri)
            || uri.Scheme != "http" || uri.Host != "127.0.0.1" || uri.Port <= 0
            || uri.AbsolutePath != "/invoke" || uri.UserInfo.Length != 0
            || uri.Query.Length != 0 || uri.Fragment.Length != 0
            || host.Token.Length != 64 || !host.Token.All(Uri.IsHexDigit)
            || !string.Equals(Path.GetFullPath(host.Database), Path.GetFullPath(databasePath),
                OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal))
            throw new InvalidDataException("Invalid native settings host or database mismatch.");
        return uri;
    }

    public static JsonNode? DecodeResponse(string json, string requestId)
    {
        var root = JsonNode.Parse(json)?.AsObject() ?? throw new InvalidDataException("Missing response.");
        if (root["schema"]?.GetValue<string>() != Schema || root["id"]?.GetValue<string>() != requestId)
            throw new InvalidDataException("Native settings response correlation mismatch.");
        if (root["error"] is JsonValue error)
            throw new InvalidOperationException(error.GetValue<string>());
        if (!root.ContainsKey("result")) throw new InvalidDataException("Missing command result.");
        return root["result"]?.DeepClone(); // null is a successful void command
    }
}

/// <summary>
/// Authenticated local transport, reusable from WinUI without dispatcher or XAML
/// dependencies. Owns no retry policy: a failed save may already have committed.
/// Hosts can be supplied directly for tests or discovered beside wisp.sqlite.
/// </summary>
public sealed class NativeSettingsClient : INativeSettingsClient, IDisposable
{
    private readonly HttpClient http;
    private readonly Uri endpoint;
    private readonly string token;

    public NativeSettingsClient(NativeSettingsHost host, string databasePath, HttpMessageHandler? handler = null)
    {
        endpoint = NativeSettingsProtocol.ValidateHost(host, databasePath);
        token = host.Token;
        // Never forward the bearer token via a proxy or an HTTP redirect.
        http = new HttpClient(handler ?? new HttpClientHandler { AllowAutoRedirect = false, UseProxy = false });
        http.Timeout = TimeSpan.FromSeconds(665);
    }

    public async Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null,
        CancellationToken cancellationToken = default)
    {
        var id = Guid.NewGuid().ToString();
        using var request = new HttpRequestMessage(HttpMethod.Post, endpoint);
        request.Headers.Authorization = new AuthenticationHeaderValue("Bearer", token);
        request.Content = new StringContent(JsonSerializer.Serialize(new NativeSettingsRequest(
            NativeSettingsProtocol.Schema, id, projectId, command, arguments)), Encoding.UTF8, "application/json");
        using var response = await http.SendAsync(request, cancellationToken).ConfigureAwait(false);
        if (response.StatusCode != HttpStatusCode.OK)
            throw new HttpRequestException("Settings host rejected the request; reconnect before retrying.", null, response.StatusCode);
        return NativeSettingsProtocol.DecodeResponse(await response.Content.ReadAsStringAsync(cancellationToken).ConfigureAwait(false), id);
    }

    /// <summary>
    /// Launch the full desktop backend on demand. Windows package layout supplies
    /// wisp-tauri.exe and its existing runtime resources (including WebView2).
    /// Cancellation stops discovery; it never kills a shared desktop host.
    /// </summary>
    public static async Task<NativeSettingsClient> ConnectAsync(string databasePath, string? hostExecutable = null,
        CancellationToken cancellationToken = default)
    {
        var descriptorPath = Path.Combine(Path.GetDirectoryName(Path.GetFullPath(databasePath))!, "native-settings.json");
        async Task<NativeSettingsClient?> Discover()
        {
            NativeSettingsClient? client = null;
            try {
                var descriptor = JsonSerializer.Deserialize<NativeSettingsHost>(await File.ReadAllTextAsync(descriptorPath, cancellationToken).ConfigureAwait(false));
                if (descriptor is null) return null;
                client = new NativeSettingsClient(descriptor, databasePath);
                using var timeout = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken);
                timeout.CancelAfter(TimeSpan.FromSeconds(2));
                await client.InvokeAsync("native_settings_capabilities", new JsonObject(), cancellationToken: timeout.Token).ConfigureAwait(false);
                return client;
            } catch (Exception ex) when (ex is IOException or JsonException or HttpRequestException or InvalidOperationException or OperationCanceledException) {
                client?.Dispose(); cancellationToken.ThrowIfCancellationRequested(); return null;
            }
        }
        var connected = await Discover().ConfigureAwait(false);
        if (connected is not null) return connected;
        if (hostExecutable is null || !File.Exists(hostExecutable))
            throw new FileNotFoundException("The full desktop settings host is not bundled.", hostExecutable);
        using var process = Process.Start(new ProcessStartInfo(Path.GetFullPath(hostExecutable)) {
            UseShellExecute = false, CreateNoWindow = true, ArgumentList = { "--native-settings-host" }
        }) ?? throw new InvalidOperationException("Unable to launch native settings host.");
        var exitedChecks = 0;
        for (var attempt = 0; attempt < 60; attempt++) {
            cancellationToken.ThrowIfCancellationRequested();
            connected = await Discover().ConfigureAwait(false);
            if (connected is not null) return connected;
            if (process.HasExited && process.ExitCode != 0)
                throw new InvalidOperationException($"桌面服务启动失败（退出码 {process.ExitCode}）。请重新构建或安装完整的原生版本后重试。");
            // A compatible running desktop may take a moment to publish its descriptor.
            // Older desktops intercept the launch but never start the broker.
            if (process.HasExited && ++exitedChecks >= 6)
                throw new InvalidOperationException("设置宿主已退出，未提供可用接口。若旧版 Wisp 正在运行，请先完成工作并退出旧版，再点击重试；无需关闭此设置窗口。");
            await Task.Delay(500, cancellationToken).ConfigureAwait(false);
        }
        throw new TimeoutException("桌面服务未能就绪。请检查版本与所选数据库后重试。");
    }

    public void Dispose() => http.Dispose();
}
