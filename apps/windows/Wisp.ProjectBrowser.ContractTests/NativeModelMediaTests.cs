using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;

internal static class NativeModelMediaTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static void Run(string fixture)
    {
        var root = Path.GetFullPath(Path.Combine(Path.GetDirectoryName(fixture)!, "../../.."));
        var catalog = JsonNode.Parse(File.ReadAllText(Path.Combine(root, "apps/macos/Sources/WispProjectBrowserUI/Resources/model-media-options.json")))!.AsObject();
        Check(NativeModelMedia.Options(catalog, "OPENAI_IMAGE_SIZES").SequenceEqual(["auto", "1024x1024", "1536x1024", "1024x1536"]), "image sizes keep shared option order");
        Check(NativeModelMedia.Options(catalog, "GROK_IMAGE_QUALITIES").SequenceEqual(["medium", "low"]), "Grok does not show unsupported auto/high qualities");
        Check(NativeModelMedia.Options(catalog, "VIDEO_RESOLUTIONS").SequenceEqual(["480p", "720p", "1080p"]), "video uses shared resolutions");
        var saved = new JsonObject { ["model"] = "vendor/grok-imagine-image-2.0", ["image_quality"] = "", ["video_duration_secs"] = null, ["future"] = "preserve" };
        var draft = NativeModelDrafts.EditDraft(saved); var snapshot = draft.DeepClone();
        Check(NativeModelMedia.Kind(draft) == NativeModelMediaKind.GrokImage && NativeModelMedia.Display(draft, "image_quality", "medium") == "medium", "exact gateway Grok ID shows Grok defaults");
        Check(NativeModelMedia.Display(draft, "video_duration_secs", "5") == "5" && JsonNode.DeepEquals(snapshot, draft), "displaying implicit defaults does not dirty a draft");
        foreach (var (id, kind) in new[] { ("vendor/GPT-IMAGE-2", NativeModelMediaKind.Image), ("grok-imagine-video-1.5-preview", NativeModelMediaKind.Video), ("grok-imagine-video-1.5-preview-next", NativeModelMediaKind.Chat) })
        {
            var candidate = new JsonObject { ["model"] = id };
            Check(NativeModelMedia.Kind(candidate) == kind, "media mode uses exact IDs, not family matching");
        }
        var custom = new JsonObject { ["model"] = "custom-image", ["image_generation_capable"] = true, ["use_for_image_generation"] = false, ["use_for_video_generation"] = true, ["image_size"] = "future-size" };
        Check(NativeModelMedia.Kind(custom) == NativeModelMediaKind.Image && NativeModelMedia.CanRestoreChat(custom), "persisted custom image capability takes precedence over the video assignment as in ModelForm");
        NativeModelMedia.RestoreChat(custom);
        Check(NativeModelMedia.Kind(custom) == NativeModelMediaKind.Chat && custom["restore_chat_model"]!.GetValue<bool>() && !custom["use_for_video_generation"]!.GetValue<bool>() && custom["image_size"]!.GetValue<string>() == "future-size", "restoring chat clears roles but preserves media values for a later edit");
        NativeModelMedia.RestoreChat(draft); Check(JsonNode.DeepEquals(snapshot, draft), "known image IDs cannot be restored to chat");
        NativeModelDrafts.SelectModel(draft, "grok-imagine-video"); Check(NativeModelMedia.Kind(draft) == NativeModelMediaKind.Video, "switching image to video replaces the media controls");
        Check(saved["model"]!.GetValue<string>() == "vendor/grok-imagine-image-2.0" && saved["image_quality"]!.GetValue<string>() == "" && saved["video_duration_secs"] == null, "draft edits leave persisted source untouched for Cancel");
        foreach (var (text, expected) in new[] { ("0", 1u), ("15", 15u), ("16", 15u), ("", 5u), ("invalid", 5u), ("-1", 5u), ("4294967296", 5u) })
            Check(NativeModelMedia.Duration(text, catalog) == expected, "video duration clamps and falls back like the WebView input");
        var unknown = new JsonObject { ["model"] = "custom", ["use_for_video_generation"] = true };
        Check(NativeModelMedia.Kind(unknown) == NativeModelMediaKind.Chat, "assignment alone does not infer a known video model's parameter schema");
    }
}
