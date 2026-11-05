using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public static class NativeModelDrafts
{
    public static string ProviderValue(string provider) => provider.Trim() switch
    {
        "anthropic" => "anthropic", "openai_responses" or "openai-responses" or "responses" => "openai_responses",
        "openai_codex" or "openai-codex" or "codex" => "openai_codex",
        "openai_chatgpt" or "openai-chatgpt" or "chatgpt" => "openai_chatgpt", "xai_oauth" or "xai-oauth" or "grok_oauth" => "xai_oauth", _ => "openai"
    };
    public static JsonObject EditDraft(JsonObject profile)
    {
        var draft = (JsonObject)profile.DeepClone();
        if (!long.TryParse(draft["max_tokens"]?.ToString(), out var output) || output < 16) draft["max_tokens"] = 8192;
        if (!long.TryParse(draft["context_window"]?.ToString(), out var context) || context < 4096) draft["context_window"] = 128000;
        draft["restore_chat_model"] = false;
        return draft;
    }
    public static JsonObject Create(string url = "https://api.openai.com/v1", string name = "") => new()
    {
        ["id"] = "", ["provider"] = "openai", ["api_url"] = url, ["model"] = name, ["label"] = name,
        ["context_window"] = 128000, ["max_tokens"] = 8192, ["send_user_agent"] = true
    };
    public static JsonArray AcpArguments(string text) => new(text.Replace("\r\n", "\n").Replace('\r', '\n')
        .Split('\n').Select(line => (JsonNode?)JsonValue.Create(line)).ToArray());
    public static bool AcpValid(JsonObject draft) => !string.IsNullOrWhiteSpace(draft["label"]?.GetValue<string>())
        && !string.IsNullOrWhiteSpace(draft["command"]?.GetValue<string>());
    public static JsonObject AcpSaveDraft(JsonObject draft)
    {
        var result = (JsonObject)draft.DeepClone();
        result["label"] = result["label"]?.GetValue<string>().Trim();
        result["command"] = result["command"]?.GetValue<string>().Trim();
        return result;
    }
    public static JsonArray? ConfirmedAcpList(JsonNode? reply, JsonObject submitted)
    {
        if (reply is not JsonArray rows) return null;
        static string? Str(JsonNode? node) => node is JsonValue v && v.TryGetValue<string>(out var s) ? s : null;
        var ids = new HashSet<string>(StringComparer.Ordinal);
        var found = false;
        foreach (var node in rows)
        {
            if (node is not JsonObject p || Str(p["id"]) is not { Length: > 0 } id || !ids.Add(id)
                || Str(p["label"]) is not { } label || Str(p["command"]) is not { } command
                || p.ContainsKey("args") && (p["args"] is not JsonArray args || args.Any(a => Str(a) == null))) return null;
            if (id == Str(submitted["id"])) found = label == Str(submitted["label"]) && command == Str(submitted["command"])
                && JsonNode.DeepEquals(p["args"] ?? new JsonArray(), submitted["args"] ?? new JsonArray());
        }
        return found ? rows : null;
    }
    public static JsonObject CatalogArguments(JsonObject draft)
    {
        var url = (draft["api_url"]?.GetValue<string>() ?? "").Trim().TrimEnd('/');
        var suffix = (draft["endpoint_suffix"]?.GetValue<string>() ?? "").Trim().Trim('/');
        return new() { ["provider"] = draft["provider"]?.DeepClone(), ["apiUrl"] = suffix.Length == 0 ? url : url + "/" + suffix,
            ["model"] = draft["model"]?.DeepClone() };
    }
    public static void SetRole(JsonObject draft, string key, bool enabled)
    {
        draft[key] = enabled;
        if (key == "supports_vision" && !enabled) draft["use_for_vision"] = false;
        if (key == "use_for_vision" && enabled) draft["supports_vision"] = true;
        if (key == "use_for_image_generation" && enabled)
        {
            draft["image_generation_capable"] = true; draft["restore_chat_model"] = false;
            draft["supports_vision"] = false; draft["use_for_vision"] = false; draft["use_for_video_generation"] = false;
        }
        if (key == "use_for_video_generation" && enabled) draft["use_for_image_generation"] = false;
    }
    public static void SelectModel(JsonObject draft, string id)
    {
        draft["model"] = id; draft["image_generation_capable"] = false;
        if (NativeModelPresentation.IsImageId(draft) || NativeModelPresentation.IsVideoId(draft))
        {
            draft["restore_chat_model"] = false; draft["supports_vision"] = false; draft["use_for_vision"] = false;
            draft["use_for_image_generation"] = NativeModelPresentation.IsImageId(draft);
            draft["use_for_video_generation"] = NativeModelPresentation.IsVideoId(draft);
        }
    }
    public static bool SessionIdentityEnabled(JsonObject draft)
    {
        if (draft["send_session_id"] is JsonValue explicitValue) return explicitValue.GetValue<bool>();
        return Uri.TryCreate(CatalogArguments(draft)["apiUrl"]?.GetValue<string>(), UriKind.Absolute, out var url)
            && url.Scheme is "http" or "https" && url.Host == "opencode.ai"
            && (url.AbsolutePath == "/zen" || url.AbsolutePath.StartsWith("/zen/", StringComparison.Ordinal));
    }
    public static JsonObject SaveArguments(JsonObject draft)
    {
        var profile = (JsonObject)draft.DeepClone();
        var key = profile["key"]?.DeepClone(); profile.Remove("key");
        var restoreChat = profile["restore_chat_model"]?.DeepClone() ?? JsonValue.Create(false); profile.Remove("restore_chat_model");
        NativeProxyRoutes.Clean(profile, model: true);
        return new() { ["profile"] = profile, ["key"] = key, ["restoreChatModel"] = restoreChat,
            ["useForVision"] = profile["use_for_vision"]?.DeepClone() ?? JsonValue.Create(false),
            ["useForImageGeneration"] = profile["use_for_image_generation"]?.DeepClone() ?? JsonValue.Create(false),
            ["useForVideoGeneration"] = profile["use_for_video_generation"]?.DeepClone() ?? JsonValue.Create(false) };
    }
    public static JsonObject TestArguments(JsonObject settings, JsonObject draft, string? proxyUrl = null)
    {
        var merged = (JsonObject)settings.DeepClone();
        foreach (var entry in draft) if (merged.ContainsKey(entry.Key) && entry.Key is not ("key" or "proxy_url")) merged[entry.Key] = entry.Value?.DeepClone();
        var args = new JsonObject { ["settings"] = merged, ["key"] = draft["key"]?.DeepClone(), ["profileId"] = draft["id"]?.DeepClone(),
            ["useForImageGeneration"] = draft["use_for_image_generation"]?.DeepClone() ?? JsonValue.Create(false) };
        if (proxyUrl != null) args["proxyUrl"] = proxyUrl.Trim();
        return args;
    }
    public static JsonArray Reorder(IEnumerable<JsonObject> profiles, string id, int offset)
    {
        var ids = profiles.Select(p => p["id"]!.GetValue<string>()).ToList();
        var index = ids.IndexOf(id);
        if (index < 0 || index + offset < 0 || index + offset >= ids.Count) throw new ArgumentOutOfRangeException(nameof(offset));
        (ids[index], ids[index + offset]) = (ids[index + offset], ids[index]);
        return new JsonArray(ids.Select(value => (JsonNode?)JsonValue.Create(value)).ToArray());
    }
    public static JsonArray ReorderTo(IEnumerable<JsonObject> profiles, string from, string target)
    {
        var ids = profiles.Select(p => p["id"]!.GetValue<string>()).ToList();
        var sourceIndex = ids.IndexOf(from); var targetIndex = ids.IndexOf(target);
        if (sourceIndex < 0 || targetIndex < 0 || ids.Distinct(StringComparer.Ordinal).Count() != ids.Count)
            throw new ArgumentException("The dragged and target models must belong to the current unique model list.");
        ids.RemoveAt(sourceIndex); ids.Insert(targetIndex, from);
        return new JsonArray(ids.Select(value => (JsonNode?)JsonValue.Create(value)).ToArray());
    }
}
