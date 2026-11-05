using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// Read-only display projection of get_pet. It never opens a directory or URL.
public sealed record NativePetPresentation(bool HasAsset, string Name, string Description, string Id,
    int Version, string Directory, string Error, byte[]? SpriteBytes)
{
    public const int SheetWidth = 1536, SheetHeight = 2288, Columns = 8, Rows = 11;
    public const int PreviewWidth = 96, PreviewHeight = 104;
    private const int MaximumBytes = 16 * 1024 * 1024;
    public static NativePetPresentation From(JsonNode? status)
    {
        var asset = (status as JsonObject)?["asset"] as JsonObject;
        return new(asset != null, Text(asset, "displayName"), Text(asset, "description"), Text(asset, "id"),
            asset?["spriteVersionNumber"] is JsonValue version && version.TryGetValue<int>(out var number) ? number : 0,
            Text(status, "directory"), Text(status, "error"), DecodeSprite(Text(asset, "spritesheetDataUrl")));
    }
    private static string Text(JsonNode? node, string key) => node is JsonObject obj
        && obj[key] is JsonValue value && value.TryGetValue<string>(out var text) ? text : "";
    public static byte[]? DecodeSprite(string dataUrl)
    {
        var prefix = new[] { "data:image/png;base64,", "data:image/webp;base64," }.FirstOrDefault(value => dataUrl.StartsWith(value, StringComparison.Ordinal));
        if (prefix == null || dataUrl.Length - prefix.Length > ((MaximumBytes + 2) / 3) * 4) return null;
        try
        {
            var bytes = Convert.FromBase64String(dataUrl[prefix.Length..]);
            return bytes.Length is > 0 and <= MaximumBytes ? bytes : null;
        }
        catch (FormatException) { return null; }
    }
}
