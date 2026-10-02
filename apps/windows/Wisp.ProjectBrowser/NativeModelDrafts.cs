using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public static class NativeModelDrafts
{
    public static JsonObject Create(string url = "https://api.openai.com/v1", string name = "") => new()
    {
        ["id"] = "", ["provider"] = "openai", ["api_url"] = url, ["model"] = name, ["label"] = name,
        ["context_window"] = 128000, ["max_tokens"] = 0, ["send_user_agent"] = true
    };
    public static JsonObject SaveArguments(JsonObject draft)
    {
        var profile = (JsonObject)draft.DeepClone();
        var key = profile["key"]?.DeepClone(); profile.Remove("key");
        return new() { ["profile"] = profile, ["key"] = key,
            ["useForVision"] = profile["use_for_vision"]?.DeepClone() ?? JsonValue.Create(false),
            ["useForImageGeneration"] = profile["use_for_image_generation"]?.DeepClone() ?? JsonValue.Create(false),
            ["useForVideoGeneration"] = profile["use_for_video_generation"]?.DeepClone() ?? JsonValue.Create(false) };
    }
    public static JsonObject TestArguments(JsonObject settings, JsonObject draft)
    {
        var merged = (JsonObject)settings.DeepClone();
        foreach (var entry in draft) if (merged.ContainsKey(entry.Key) && entry.Key != "key") merged[entry.Key] = entry.Value?.DeepClone();
        return new() { ["settings"] = merged, ["key"] = draft["key"]?.DeepClone(), ["profileId"] = draft["id"]?.DeepClone(),
            ["useForImageGeneration"] = draft["use_for_image_generation"]?.DeepClone() ?? JsonValue.Create(false) };
    }
    public static JsonArray Reorder(IEnumerable<JsonObject> profiles, string id, int offset)
    {
        var ids = profiles.Select(p => p["id"]!.GetValue<string>()).ToList();
        var index = ids.IndexOf(id);
        if (index < 0 || index + offset < 0 || index + offset >= ids.Count) throw new ArgumentOutOfRangeException(nameof(offset));
        (ids[index], ids[index + offset]) = (ids[index + offset], ids[index]);
        return new JsonArray(ids.Select(value => (JsonNode?)JsonValue.Create(value)).ToArray());
    }
}
