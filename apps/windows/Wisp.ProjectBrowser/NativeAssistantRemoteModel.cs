using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Owns one mounted remote-access dialog. Requests are serialized; mutations and
/// binding polls are never replayed after an ambiguous response or after closing.
public sealed class NativeAssistantRemoteModel(INativeSettingsClient client) : IDisposable
{
    public event Action? Changed;
    public NativeAssistantWeixinStatus? Status { get; private set; }
    public string? Error { get; private set; }
    public string? QrImage { get; private set; }
    public bool Expired { get; private set; }
    public bool Closed { get; private set; }
    public bool Busy => requesting || qrcode != null;
    public bool CanBind => !Closed && !Busy && Status != null;
    public bool CanChange => CanBind && Status!.Bound;
    private string? qrcode;
    private bool requesting;
    private int polls;
    private readonly CancellationTokenSource lifetime = new();

    public async Task RefreshAsync(bool clearError = true)
    {
        if (Closed || Busy) return;
        await Request(async () => { if (clearError) Error = null; await ReadStatus(); });
    }
    public Task SetEnabledAsync(bool enabled) => Change("set_weixin_channel", enabled);
    public Task UnbindAsync() => Change("weixin_unbind", false);
    private async Task Change(string command, bool enabled)
    {
        if (!CanChange) return;
        await Request(async () =>
        {
            Error = null;
            await Invoke(command, new() { ["destination"] = "assistant", ["enabled"] = enabled });
            if (!Closed) await ReadStatus();
        });
    }
    public async Task StartBindingAsync()
    {
        if (!CanBind) return;
        await Request(async () =>
        {
            Error = null; Expired = false;
            var value = await Invoke("weixin_bind_start", new());
            if (Closed) return;
            var code = value?["qrcode"]?.GetValue<string>();
            var image = value?["qr_image"]?.GetValue<string>();
            const string prefix = "data:image/svg+xml;base64,";
            if (string.IsNullOrWhiteSpace(code) || image == null || !image.StartsWith(prefix, StringComparison.Ordinal) || image.Length > 2000000)
                throw new InvalidDataException("Invalid WeChat binding QR image.");
            // Validate data before exposing a flow that can persist a binding.
            _ = Convert.FromBase64String(image[prefix.Length..]);
            qrcode = code; QrImage = image; polls = 0;
        });
    }
    public async Task PollAsync()
    {
        if (Closed || requesting || qrcode == null) return;
        await Request(async () =>
        {
            var value = await Invoke("weixin_bind_poll", new() { ["qrcode"] = qrcode, ["destination"] = "assistant" });
            if (Closed) return;
            var state = value?.GetValue<string>() ?? throw new InvalidDataException("Missing WeChat binding status.");
            if (state == "confirmed") { ClearQr(); await ReadStatus(); }
            else if (state == "expired" || ++polls >= 180) { ClearQr(); Expired = true; }
        });
    }
    public void ImageFailed(string message)
    {
        if (Closed) return;
        ClearQr(); Error = message; Changed?.Invoke();
    }
    private async Task ReadStatus()
    {
        try
        {
            var value = await Invoke("assistant_weixin_status", new());
            if (Closed) return;
            Status = new(value?["enabled"]?.GetValue<bool>() ?? throw new InvalidDataException("Missing WeChat enabled state."),
                value?["bound"]?.GetValue<bool>() ?? throw new InvalidDataException("Missing WeChat binding state."),
                value?["state"]?.GetValue<string>() ?? throw new InvalidDataException("Missing WeChat channel state."),
                value?["detail"]?.GetValue<string>() ?? "");
        }
        catch { if (!Closed) Status = null; throw; }
    }
    private async Task Request(Func<Task> action)
    {
        requesting = true; Changed?.Invoke();
        try { await action(); }
        catch (Exception error) { if (!Closed) { ClearQr(); Error = error.Message; } }
        finally { requesting = false; if (!Closed) Changed?.Invoke(); }
    }
    private Task<JsonNode?> Invoke(string command, JsonObject args) => client.InvokeAsync(command, args, null, lifetime.Token);
    private void ClearQr() { qrcode = null; QrImage = null; }
    public void Dispose()
    {
        if (Closed) return;
        Closed = true; ClearQr(); Status = null; lifetime.Cancel(); lifetime.Dispose();
    }
}
