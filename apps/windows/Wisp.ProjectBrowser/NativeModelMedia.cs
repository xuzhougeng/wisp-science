using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public enum NativeModelMediaKind { Chat, Image, GrokImage, Video }

/// Editor classification follows ModelForm, including custom image capabilities.
/// Display defaults never turn an untouched empty/null setting into a saved override.
public static class NativeModelMedia
{
    public static NativeModelMediaKind Kind(JsonObject draft)
    {
        if (NativeModelPresentation.IsImage(draft))
            return (draft["model"]?.GetValue<string>() ?? "").Trim().Split('/').Last().Equals("grok-imagine-image-2.0", StringComparison.OrdinalIgnoreCase)
                ? NativeModelMediaKind.GrokImage : NativeModelMediaKind.Image;
        return NativeModelPresentation.IsVideoId(draft) ? NativeModelMediaKind.Video : NativeModelMediaKind.Chat;
    }
    public static string Display(JsonObject draft, string key, string fallback)
        => draft[key]?.ToString() is { Length: > 0 } value ? value : fallback;
    public static string[] Options(JsonObject catalog, string key)
        => catalog[key]!.AsArray().Select(value => value!.GetValue<string>()).ToArray();
    public static bool CanRestoreChat(JsonObject draft) => NativeModelPresentation.IsImage(draft)
        && !NativeModelPresentation.IsImageId(draft) && !NativeModelPresentation.IsVideoId(draft);
    public static void RestoreChat(JsonObject draft)
    {
        if (!CanRestoreChat(draft)) return;
        draft["image_generation_capable"] = false;
        draft["use_for_image_generation"] = false;
        draft["use_for_video_generation"] = false;
        draft["restore_chat_model"] = true;
    }
    public static uint Duration(string text, JsonObject catalog)
    {
        var value = uint.TryParse(text, out var parsed) ? parsed : catalog["VIDEO_DURATION_DEFAULT_SECS"]!.GetValue<uint>();
        return Math.Clamp(value, catalog["VIDEO_DURATION_MIN_SECS"]!.GetValue<uint>(), catalog["VIDEO_DURATION_MAX_SECS"]!.GetValue<uint>());
    }
}
