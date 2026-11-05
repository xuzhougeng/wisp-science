using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// Shared API access with editable model rows, matching the WebView provider form.
public static class NativeProviderDrafts
{
    private static readonly string[] RoleKeys = ["supports_vision", "use_for_vision", "use_for_expert", "use_for_image_generation", "use_for_video_generation"];
    public static string Text(JsonObject value, string key) => value[key]?.GetValue<string>() ?? "";
    private static bool Flag(JsonObject value, string key) => value[key]?.GetValue<bool>() == true;
    public static IEnumerable<JsonObject> Entries(JsonObject draft) => (draft["entries"] as JsonArray)?.OfType<JsonObject>() ?? [];
    public static JsonObject Entry(string provider = "openai", string model = "")
    {
        var entry = new JsonObject { ["row_id"] = Guid.NewGuid().ToString("N"), ["provider"] = provider,
            ["model"] = model, ["label"] = "", ["endpoint_suffix"] = "" };
        foreach (var key in RoleKeys) entry[key] = false;
        SelectModel(entry, model); return entry;
    }
    public static JsonObject Create(string url = "https://api.deepseek.com") => new()
    {
        ["api_url"] = url, ["key"] = "", ["entries"] = Suggested(url), ["send_user_agent"] = true,
        ["user_agent"] = "", ["send_session_id"] = null, ["session_header_name"] = ""
    };
    public static JsonArray Suggested(string url)
    {
        var normalized = NormalizeEndpoint(url).ToLowerInvariant();
        var host = Uri.TryCreate(url.Trim(), UriKind.Absolute, out var uri) ? uri.Host : "";
        JsonObject[] entries = host switch
        {
            "api.anthropic.com" => [Entry("anthropic", "claude-sonnet-5")],
            "api.openai.com" => [Entry("openai_responses", "gpt-5.5"), Entry("openai_responses", "gpt-image-2")],
            "api.x.ai" => [Entry("openai", "grok-4.6"), Entry("openai", "grok-imagine-image-2.0")],
            "api.deepseek.com" or "deepseek.com" => [Entry(model: "deepseek-v4-flash"), Entry(model: "deepseek-v4-pro")],
            "api.kimi.com" when normalized.Contains("/coding") => [Entry(model: "kimi-coding")],
            "api.kimi.com" or "api.moonshot.cn" or "api.moonshot.ai" => [Entry(model: "kimi-k3")],
            "open.bigmodel.cn" when normalized.Contains("coding") => [Entry(model: "glm-5.2")],
            "open.bigmodel.cn" => [Entry(model: "glm-5")],
            _ when normalized.Length == 0 => [Entry(model: "deepseek-v4-flash"), Entry(model: "deepseek-v4-pro")],
            _ => [Entry()]
        };
        return new JsonArray(entries.Select(entry => (JsonNode?)entry).ToArray());
    }
    public static bool Pristine(JsonObject draft)
    {
        static string Signature(JsonObject entry) => string.Join('\u001f', new[] { NativeModelDrafts.ProviderValue(Text(entry, "provider")),
            Text(entry, "model").Trim(), Text(entry, "label").Trim(), Text(entry, "endpoint_suffix").Trim() }.Concat(RoleKeys.Select(key => Flag(entry, key).ToString())));
        return Entries(draft).Select(Signature).SequenceEqual(Suggested(Text(draft, "api_url")).OfType<JsonObject>().Select(Signature));
    }
    public static bool ChangeUrl(JsonObject draft, string url)
    {
        var replace = Pristine(draft); draft["api_url"] = url;
        if (replace) draft["entries"] = Suggested(url);
        return replace;
    }
    public static void SelectModel(JsonObject entry, string model)
    {
        entry["model"] = model;
        if (NativeModelPresentation.IsImageId(entry) || NativeModelPresentation.IsVideoId(entry))
        {
            entry["supports_vision"] = false; entry["use_for_vision"] = false;
            entry["use_for_image_generation"] = NativeModelPresentation.IsImageId(entry);
            entry["use_for_video_generation"] = NativeModelPresentation.IsVideoId(entry);
        }
    }
    public static void SetRole(JsonObject entry, string key, bool enabled)
    {
        entry[key] = enabled;
        if (key == "supports_vision" && !enabled) entry["use_for_vision"] = false;
        if (key == "use_for_vision" && enabled) entry["supports_vision"] = true;
        if (enabled && key is "use_for_image_generation" or "use_for_video_generation")
        {
            entry["supports_vision"] = false; entry["use_for_vision"] = false;
            entry[key == "use_for_image_generation" ? "use_for_video_generation" : "use_for_image_generation"] = false;
        }
    }
    public static bool Everyday(JsonObject entry) => !NativeModelPresentation.IsImage(entry) && !NativeModelPresentation.IsVideo(entry) && !Flag(entry, "use_for_expert");
    public static JsonObject[] SaveOrder(JsonObject draft)
    {
        var entries = Entries(draft).Where(entry => Text(entry, "model").Trim().Length > 0).ToArray();
        var first = entries.FirstOrDefault(Everyday);
        if (first == null) return entries;
        return entries.Where(entry => !ReferenceEquals(entry, first)).OrderBy(entry => NativeModelPresentation.IsImage(entry) || NativeModelPresentation.IsVideo(entry) ? 0 : 1)
            .Append(first).ToArray();
    }
    public static string NormalizeEndpoint(string url)
    {
        var endpoint = url.Trim();
        foreach (var scheme in new[] { "https://", "http://" })
            if (endpoint.StartsWith(scheme, StringComparison.Ordinal))
            {
                var rest = endpoint[scheme.Length..]; var slash = rest.IndexOf('/');
                endpoint = scheme + (slash < 0 ? rest.ToLowerInvariant() : rest[..slash].ToLowerInvariant() + rest[slash..]); break;
            }
        while (true)
        {
            endpoint = endpoint.TrimEnd('/');
            var suffix = new[] { "/v1/messages", "/v1/chat/completions", "/chat/completions", "/responses", "/v1" }.FirstOrDefault(s => endpoint.EndsWith(s, StringComparison.Ordinal));
            if (suffix == null) return endpoint;
            endpoint = endpoint[..^suffix.Length];
        }
    }
    public static JsonObject? StoredKeyProfile(JsonObject draft, IEnumerable<JsonObject> existing)
    {
        var endpoint = NormalizeEndpoint(Text(draft, "api_url"));
        return endpoint.Length == 0 ? null : existing.FirstOrDefault(profile => Flag(profile, "has_api_key")
            && NativeModelDrafts.ProviderValue(Text(profile, "provider")) is not ("openai_codex" or "openai_chatgpt" or "xai_oauth")
            && NormalizeEndpoint(Text(profile, "api_url")) == endpoint);
    }
    public static bool NeedsKey(JsonObject draft, IEnumerable<JsonObject> existing) =>
        string.IsNullOrWhiteSpace(Text(draft, "key")) && StoredKeyProfile(draft, existing) == null;

    public static string? ErrorKey(JsonObject draft, IEnumerable<JsonObject> existing, bool save = true)
    {
        if (NativeProxyRoutes.ErrorKey(draft) is { } routeError) return routeError;
        if (Text(draft, "api_url").Trim().Length == 0) return "err.api_url_required";
        var entries = SaveOrder(draft); if (entries.Length == 0) return "err.model_required";
        if (save && !entries.Any(Everyday) && !existing.Any(Everyday)) return "models.need_chat";
        if (NeedsKey(draft, existing)) return "err.api_key_required";
        return null;
    }
    public static JsonObject Profile(JsonObject draft, JsonObject entry, JsonObject? catalog = null)
    {
        var image = NativeModelPresentation.IsImage(entry); var video = !image && NativeModelPresentation.IsVideo(entry); var media = image || video;
        var profile = new JsonObject
        {
            // Stable per-draft IDs make an explicit retry update the same profile
            // after a lost acknowledgement instead of creating a duplicate.
            ["id"] = "native-api-" + Text(entry, "row_id"), ["provider"] = NativeModelDrafts.ProviderValue(Text(entry, "provider")),
            ["api_url"] = Text(draft, "api_url").Trim(), ["model"] = Text(entry, "model").Trim(), ["label"] = Text(entry, "label").Trim(),
            ["endpoint_suffix"] = Text(entry, "endpoint_suffix").Trim(), ["max_tokens"] = catalog?["max_tokens"]?.DeepClone() ?? JsonValue.Create(8192),
            ["context_window"] = catalog?["context_window"]?.DeepClone() ?? JsonValue.Create(128000), ["reasoning_effort"] = "", ["service_tier"] = "",
            ["send_user_agent"] = Flag(draft, "send_user_agent"), ["user_agent"] = Text(draft, "user_agent"), ["send_session_id"] = draft["send_session_id"]?.DeepClone(),
            ["session_header_name"] = Text(draft, "session_header_name"), ["supports_vision"] = Flag(entry, "supports_vision") && !media,
            ["use_for_vision"] = Flag(entry, "use_for_vision") && !media, ["use_for_expert"] = Flag(entry, "use_for_expert") && !media,
            ["use_for_image_generation"] = image, ["use_for_video_generation"] = video,
            ["image_size"] = "", ["image_quality"] = "", ["image_aspect_ratio"] = "", ["image_resolution"] = "",
            ["video_duration_secs"] = null, ["video_aspect_ratio"] = null, ["video_resolution"] = null
        };
        if (draft["_proxy_edited"]?.GetValue<bool>() == true)
            NativeProxyRoutes.Set(profile, Text(draft, "proxy_url"), NativeProxyRoutes.Custom(draft));
        return profile;
    }
}
