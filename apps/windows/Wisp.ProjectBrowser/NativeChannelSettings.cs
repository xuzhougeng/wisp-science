using System.Globalization;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

// Channel status uses snake_case, except the nested Device Bridge status.
public static class NativeChannelSettings
{
    private static string Text(JsonNode? value, string key) => value?[key]?.GetValue<string>() ?? "";
    private static bool Flag(JsonNode? value, string key) => value?[key]?.GetValue<bool>() == true;

    public static bool Supports(JsonNode? capabilities, string command) =>
        (capabilities?["commands"] as JsonArray)?.OfType<JsonValue>()
            .Any(value => value.TryGetValue<string>(out var name) && name == command) == true;
    public static JsonObject RemoteDraft(JsonNode? remote) => new()
    {
        ["enabled"] = Flag(remote, "enabled"), ["relay_url"] = Text(remote, "relay_url"), ["relay_token"] = ""
    };

    public static JsonObject FeishuDraft(JsonNode status) => new()
    {
        ["enabled"] = Flag(status, "feishu_enabled"), ["international"] = Flag(status, "feishu_international"),
        ["app_id"] = Text(status, "feishu_app_id"), ["app_secret"] = ""
    };
    public static JsonObject DeviceDraft(JsonNode? device) => new()
    {
        ["enabled"] = Flag(device, "enabled"), ["mode"] = "lan",
        ["bind_ipv4"] = Text(device, "bindIpv4"), ["port"] = device?["port"]?.DeepClone() ?? JsonValue.Create(18766)
    };
    public static string? DeviceSaveBlockReason(JsonObject draft)
    {
        if (Text(draft, "mode") != "lan") return "中继服务器模式尚未开放。";
        var address = Text(draft, "bind_ipv4").Trim();
        var octets = address.Split('.');
        if (octets.Length != 4 || octets.Any(part => part.Length == 0 || part.Length > 3
            || part.Length > 1 && part[0] == '0'
            || !byte.TryParse(part, NumberStyles.None, CultureInfo.InvariantCulture, out _)))
            return "请输入有效且具体的 IPv4 地址。";
        if (address == "0.0.0.0") return "不允许使用 0.0.0.0，请选择一个具体 IPv4 地址。";
        if (!int.TryParse(draft["port"]?.ToString(), NumberStyles.None, CultureInfo.InvariantCulture, out var port) || port is < 1 or > 65535)
            return "端口必须在 1 到 65535 之间。";
        return null;
    }
    public static JsonObject DeviceSaveDraft(JsonObject draft)
    {
        if (DeviceSaveBlockReason(draft) is { } error) throw new InvalidDataException(error);
        var saved = (JsonObject)draft.DeepClone();
        saved["bind_ipv4"] = Text(draft, "bind_ipv4").Trim();
        saved["port"] = int.Parse(draft["port"]!.ToString(), CultureInfo.InvariantCulture);
        return saved;
    }
    public static bool CanEnable(JsonNode status, string channel) => channel switch
    {
        "feishu" => Flag(status, "feishu_bound"),
        "weixin" => Flag(status, "weixin_bound"),
        "device" => Text(status["device"], "bindIpv4").Length > 0,
        "remote" => Text(status["remote"], "relay_url").Length > 0 && Flag(status["remote"], "has_token"),
        _ => false
    };

    public readonly record struct RowStatus(string State, string Label, string Hint);

    // Overview badges on WebView are gated by binding (bots) or enablement
    // (bridges); the StickS3 detail still reports its underlying listener state.
    public static RowStatus Overview(JsonNode status, string channel, NativeLocalization locale)
    {
        string T(string key) => locale.Text(key);
        var bound = channel is "feishu" or "weixin" && Flag(status, channel + "_bound");
        var device = status["device"];
        var remote = status["remote"];
        var state = channel switch
        {
            "feishu" or "weixin" when bound => Text(status, channel + "_state"),
            "device" when Flag(device, "enabled") => Text(device, "state"),
            "remote" when Flag(remote, "enabled") => Text(remote, "state"),
            _ => "stopped"
        };
        var label = channel switch
        {
            "lab" => "",
            "feishu" or "weixin" when !bound => T($"channels.{channel}.not_bound"),
            _ => StateLabel(state, locale)
        };
        var hint = channel switch
        {
            "feishu" when Text(status, "feishu_pending_owner_open_id").Length > 0 => T("channels.feishu.pending_title"),
            "feishu" when bound && Text(status, "feishu_owner_open_id").Length == 0 => T("channels.feishu.owner_missing"),
            "feishu" when bound => Text(status, "feishu_app_id") + " · " + T(Flag(status, "feishu_international") ? "channels.feishu.region_lark" : "channels.feishu.region_cn"),
            "weixin" when bound && Text(status, "weixin_detail") is { Length: > 0 } detail => detail,
            "device" when device?["url"] is JsonValue url => url.GetValue<string>(),
            "device" when Text(device, "bindIpv4") is { Length: > 0 } ip => ip + ":" + (device?["port"]?.ToString() ?? "18766"),
            "remote" when Flag(remote, "enabled") && Text(remote, "relay_url") is { Length: > 0 } relay => relay,
            _ => T($"channels.{channel}.subtitle")
        };
        return new(state, label, hint);
    }
    public static string StateColor(string state) => state switch
    {
        "running" or "listening" => "ok", "connecting" => "warn", "error" => "err", _ => "text-muted"
    };
    public static string StateLabel(string state, NativeLocalization? locale = null) => locale?.Text(state switch
    {
        "listening" => "channels.device.state.listening", "running" => "channels.state.running",
        "connecting" => "channels.state.connecting", "error" => "channels.state.error", _ => "channels.state.stopped"
    }) ?? state switch
    {
        "running" => "运行中", "listening" => "正在监听", "connecting" => "连接中", "error" => "出错", _ => "已停止"
    };
}
