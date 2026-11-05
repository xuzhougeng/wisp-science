using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// List actions follow the WebView model roles. The host remains authoritative on save.
public static class NativeModelPresentation
{
    private static string ModelId(JsonObject profile) => (profile["model"]?.GetValue<string>() ?? "").Trim().Split('/').Last().ToLowerInvariant();
    public static bool IsImageId(JsonObject profile) => ModelId(profile) is "gpt-image-2" or "grok-imagine-image-2.0";
    public static bool IsVideoId(JsonObject profile) => ModelId(profile) is "grok-imagine-video" or "grok-imagine-video-1.5" or "grok-imagine-video-1.5-preview";
    public static bool IsImage(JsonObject profile) => IsImageId(profile) || profile["image_generation_capable"]?.GetValue<bool>() == true
        || profile["use_for_image_generation"]?.GetValue<bool>() == true;
    public static bool IsVideo(JsonObject profile) => IsVideoId(profile) || profile["use_for_video_generation"]?.GetValue<bool>() == true;
    public static bool IsChat(JsonObject profile)
    {
        var id = (profile["model"]?.GetValue<string>() ?? "").Trim().Split('/').Last().ToLowerInvariant();
        return profile["image_generation_capable"]?.GetValue<bool>() != true
            && profile["use_for_image_generation"]?.GetValue<bool>() != true
            && id is not ("gpt-image-2" or "grok-imagine-image-2.0" or "grok-imagine-video"
                or "grok-imagine-video-1.5" or "grok-imagine-video-1.5-preview");
    }
    public static bool CanDelete(JsonObject profile, IEnumerable<JsonObject> profiles)
        => profiles.Any(other => other["id"]?.ToString() != profile["id"]?.ToString() && IsChat(other));
    public static bool SupportsFast(JsonObject profile) => IsChat(profile)
        && profile["provider"]?.GetValue<string>().Trim() is "openai" or "openai_responses" or "openai-responses"
            or "responses" or "openai_codex" or "openai-codex" or "codex";
    // Same order as wisp_dto::session_profile; draft composers have no bound ID.
    public static JsonObject? SessionProfile(IEnumerable<JsonObject> profiles, string? boundId)
    {
        var chat = profiles.Where(IsChat).ToArray();
        return chat.FirstOrDefault(row => row["id"]?.GetValue<string>() == boundId)
            ?? chat.FirstOrDefault(row => row["active"]?.GetValue<bool>() == true) ?? chat.FirstOrDefault();
    }
    public static string Label(JsonObject profile)
        => profile["label"]?.GetValue<string>() is { Length: > 0 } label ? label : profile["model"]?.GetValue<string>() ?? "";
    public static string Subtitle(JsonObject profile)
        => profile["model"]?.GetValue<string>() is { Length: > 0 } id && id != Label(profile) ? id : "";
}
