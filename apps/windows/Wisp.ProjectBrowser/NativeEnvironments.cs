using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public static class NativeEnvironments
{
    public static string Text(JsonNode? row, string key) => row?[key]?.ToString() ?? "";
    public static JsonObject[] Rows(JsonNode? value) => (value as JsonArray)?.OfType<JsonObject>().ToArray() ?? [];
    public static JsonObject Config(JsonObject context) => string.IsNullOrWhiteSpace(Text(context, "config_json")) ? new() : JsonNode.Parse(Text(context, "config_json")) as JsonObject ?? new();
    public static string Label(JsonObject context) => string.IsNullOrWhiteSpace(Text(context, "label")) ? Text(context, "id") : Text(context, "label");
    public static (string Id, string Label)[] RemoteOptions(IEnumerable<JsonObject> contexts) => contexts
        .Where(row => Text(row, "kind") is "ssh" or "wsl").Select(row => (Text(row, "id"), Label(row))).ToArray();
    public static string Capabilities(JsonObject context)
    {
        JsonObject? value = null;
        try { value = JsonNode.Parse(Text(context, "capabilities_json")) as JsonObject; } catch (System.Text.Json.JsonException) { }
        var parts = new List<string>();
        if (value != null)
        {
            var platform = new[] { Text(value, "os"), Text(value, "arch") }.Where(v => v.Length > 0);
            var os = string.Join("/", platform); if (os.Length > 0) parts.Add(os);
            foreach (var key in new[] { "gpu_summary", "scheduler", "python", "r_version" }) if (Text(value, key) is { Length: > 0 } part) parts.Add(part);
            if (value["probe_skill"] != null && value["gpu_summary"] == null) parts.Add("No GPU");
            if (Text(value, "privilege") is { Length: > 0 } privilege) parts.Add(privilege);
        }
        return parts.Count > 0 ? string.Join(" · ", parts) : context["last_probe_status"]?.ToString() ?? "not probed";
    }
    public static string Connection(JsonObject context, IEnumerable<JsonObject> hosts)
    {
        var id = Text(context, "id"); var alias = id.StartsWith("ssh:", StringComparison.Ordinal) ? id[4..] : null;
        var host = hosts.FirstOrDefault(row => Text(row, "alias") == alias);
        return host == null ? id : (Text(host, "user") is { Length: > 0 } user ? user + "@" : "") + alias
            + (host["port"] != null ? ":" + Text(host, "port") : "");
    }
    public static bool ValidPort(JsonObject draft) => string.IsNullOrWhiteSpace(Text(draft, "port")) || ushort.TryParse(Text(draft, "port"), out var port) && port > 0;
    public static bool CanSaveHost(JsonObject draft) => !string.IsNullOrWhiteSpace(Text(draft, "alias")) && ValidPort(draft)
        && (Text(draft, "auth_method") != "password" || !string.IsNullOrWhiteSpace(Text(draft, "password")) || draft["has_password"]?.GetValue<bool>() == true);
    public static JsonObject HostPayload(JsonObject draft)
    {
        string? Optional(string key) => string.IsNullOrWhiteSpace(Text(draft, key)) ? null : Text(draft, key).Trim();
        var password = Text(draft, "auth_method") == "password";
        return new()
        {
            ["alias"] = Text(draft, "alias").Trim(), ["host_name"] = Optional("host_name"), ["user"] = Optional("user"),
            ["port"] = ushort.TryParse(Text(draft, "port"), out var port) ? (int)port : null,
            ["auth_method"] = password ? "password" : "key", ["identity_file"] = password ? null : Optional("identity_file"),
            ["password"] = password ? Optional("password") : null, ["has_password"] = false, ["notes"] = Optional("notes")
        };
    }
    public static bool NeedsDisposalConfirmation(JsonObject report) => new[] { "external_references", "staged_files", "active_runs" }
        .Any(key => long.TryParse(Text(report, key), out var count) && count > 0);
    public static string FailureKind(string detail)
    {
        var value = detail.ToLowerInvariant();
        bool Has(params string[] needles) => needles.Any(value.Contains);
        if (Has("local openssh is too old", "openssh client was not found", "could not parse local openssh")) return "client";
        if (Has("ssh password authentication failed")) return "password";
        if (Has("ssh key authentication failed")) return "key";
        if (Has("ssh connection succeeded", "ssh authentication succeeded", "probe command returned no output")) return "probe_output";
        if (Has("identity file", "not accessible", "no such identity")) return "identity";
        if (Has("permission denied", "publickey", "too many authentication failures", "authentication failed")) return "auth";
        if (Has("timed out", "timeout", "connection refused", "no route to host", "network is unreachable")) return "timeout";
        if (Has("could not resolve", "name or service not known", "nodename nor servname")) return "resolve";
        if (Has("host key verification failed", "remote host identification has changed")) return "hostkey";
        return "other";
    }
    public static string[] FailureCauses(string kind) => Enumerable.Range(1, kind is "identity" or "resolve" or "hostkey" ? 2 : kind == "auth" ? 4 : 3)
        .Select(number => $"ssh_check.cause.{kind}.{number}").ToArray();
}
