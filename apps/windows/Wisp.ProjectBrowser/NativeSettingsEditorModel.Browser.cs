using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed partial class NativeSettingsEditorModel
{
    public Task<bool> SetBrowserBehaviorAsync(string read, bool enabled)
    {
        if (Busy || Closed || Draft != null || read is not ("get_browser_auto_launch" or "get_browser_auto_close_tabs")
            || Values[read] is not JsonValue current || !current.TryGetValue<bool>(out _)) return Task.FromResult(false);
        return InvokeAsync("set_" + read[4..], new() { ["enabled"] = enabled }, value =>
        {
            if (value is not JsonValue reply || !reply.TryGetValue<bool>(out var confirmed))
                throw new InvalidDataException("浏览器设置的保存结果无效，请刷新后确认。");
            Values[read] = confirmed;
        });
    }

    public Task<bool> AddBrowserRuleAsync(string kind, string host, string reason)
    {
        host = host.Trim();
        if (host.Length == 0 || BrowserRuleDraft(kind) is not { } next) return Task.FromResult(false);
        var rows = next[kind]!.AsArray();
        foreach (var row in rows.OfType<JsonObject>().Where(row => string.Equals(row["host"]?.GetValue<string>(), host, StringComparison.OrdinalIgnoreCase)).ToArray()) rows.Remove(row);
        rows.Add(new JsonObject { ["host"] = host, ["reason"] = reason.Trim() });
        return SaveBrowserRulesAsync(next);
    }

    public Task<bool> RemoveBrowserRuleAsync(string kind, string host)
    {
        if (BrowserRuleDraft(kind) is not { } next) return Task.FromResult(false);
        var rows = next[kind]!.AsArray();
        var removed = rows.OfType<JsonObject>().Where(row => row["host"]?.GetValue<string>() == host).ToArray();
        if (removed.Length == 0) return Task.FromResult(false);
        foreach (var row in removed) rows.Remove(row);
        return SaveBrowserRulesAsync(next);
    }

    private JsonObject? BrowserRuleDraft(string kind) => !Busy && !Closed && Draft == null && kind is "block" or "prefer"
        && ValidBrowserRules(Values["get_browser_url_filters"]) ? (JsonObject)Values["get_browser_url_filters"]!.DeepClone() : null;

    private static bool ValidBrowserRules(JsonNode? value) => value is JsonObject filters
        && new[] { "block", "prefer" }.All(kind => filters[kind] is JsonArray rows && rows.All(row => row is JsonObject obj
            && obj["host"] is JsonValue host && host.TryGetValue<string>(out var text) && !string.IsNullOrWhiteSpace(text)
            && obj["reason"] is JsonValue reason && reason.TryGetValue<string>(out _)));

    private Task<bool> SaveBrowserRulesAsync(JsonObject next) => InvokeAsync("set_browser_url_filters", new() { ["filters"] = next }, value =>
    {
        // The host normalizes domains, subdomains and duplicate rules. Display its acknowledgement.
        if (!ValidBrowserRules(value)) throw new InvalidDataException("网址过滤的保存结果无效，请刷新后确认。");
        Values["get_browser_url_filters"] = value!.DeepClone();
    });
}
