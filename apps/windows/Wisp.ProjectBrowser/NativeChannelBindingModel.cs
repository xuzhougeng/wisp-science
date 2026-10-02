using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// A single explicit scan flow. Polling can finalize credentials, so never replay it.
public sealed class NativeChannelBindingModel(INativeSettingsClient client, string? projectId) : WorkspaceActionModel
{
    public string? Kind { get; private set; }
    public JsonObject? Binding { get; private set; }
    public string Status { get; private set; } = "";
    public bool Active => Binding != null;
    private DateTimeOffset nextPoll;
    public Task<bool> StartAsync(string kind, bool international = false)
    {
        if (Active || kind is not ("feishu" or "weixin")) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            var result = await client.InvokeAsync(kind + "_bind_start", kind == "feishu" ? new() { ["international"] = international } : new(), projectId) as JsonObject
                ?? throw new InvalidDataException("绑定服务未返回扫码信息。");
            var idKey = kind == "feishu" ? "flow_id" : "qrcode";
            if (string.IsNullOrWhiteSpace(result[idKey]?.GetValue<string>())) throw new InvalidDataException("绑定服务未返回有效流程 ID。");
            if (Closed && kind == "feishu") await CancelFlow(result);
            return result;
        }, value => { Kind = kind; Binding = (JsonObject)value.DeepClone(); Status = "等待扫码确认"; nextPoll = DateTimeOffset.MinValue; });
    }
    public Task<bool> PollAsync()
    {
        if (!Active || DateTimeOffset.UtcNow < nextPoll) return Task.FromResult(false);
        var kind = Kind; var binding = Binding!;
        var args = kind == "feishu" ? new JsonObject { ["flowId"] = binding["flow_id"]?.DeepClone() } : new JsonObject { ["qrcode"] = binding["qrcode"]?.DeepClone() };
        return RunAsync(() => client.InvokeAsync(kind + "_bind_poll", args, projectId), value =>
        {
            var state = kind == "feishu" ? value?["state"]?.GetValue<string>() : value?.GetValue<string>();
            Status = state switch { "confirmed" => "绑定已确认", "denied" => "绑定已拒绝", "expired" => "二维码已过期", "scaned" or "scanned" => "已扫码，请在手机上确认", "pending" or "wait" => "等待扫码确认", _ => state ?? "未返回绑定状态" };
            if (state is "confirmed" or "denied" or "expired") { Binding = null; Kind = null; }
            else if (kind == "feishu")
            {
                long.TryParse(value?["retry_after_ms"]?.ToString(), out var delay);
                nextPoll = DateTimeOffset.UtcNow.AddMilliseconds(Math.Clamp(delay, 0, 600000));
            }
        });
    }
    public Task<bool> CancelAsync()
    {
        if (!Active) return Task.FromResult(!Busy && !Closed);
        var kind = Kind; var binding = Binding!;
        return RunAsync(async () => { if (kind == "feishu") await CancelFlow(binding); return true; }, _ =>
        { Binding = null; Kind = null; Status = "已取消绑定"; });
    }
    private async Task CancelFlow(JsonObject binding)
        => await client.InvokeAsync("feishu_bind_cancel", new() { ["flowId"] = binding["flow_id"]?.DeepClone() }, projectId);
    public override void Dispose()
    {
        if (Closed) return;
        var cleanup = Kind == "feishu" ? Binding : null;
        base.Dispose(); Binding = null; Kind = null;
        if (cleanup != null) _ = CleanupAsync(cleanup);
    }
    private async Task CleanupAsync(JsonObject binding)
    {
        try { await CancelFlow(binding); } catch { /* Host expires orphaned registrations. Never replay on transport failure. */ }
    }
}
