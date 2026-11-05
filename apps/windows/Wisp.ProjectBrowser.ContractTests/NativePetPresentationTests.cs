using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativePetPresentationTests
{
    public static async Task RunAsync()
    {
        var unavailable = NativePetPresentation.From(new JsonObject { ["enabled"] = false, ["directory"] = "C:\\not-read" });
        Check(!unavailable.HasAsset && unavailable.SpriteBytes == null && unavailable.Directory == "C:\\not-read",
            "disabled pets are displayed without opening the configured folder");
        Check(!NativePetPresentation.From(JsonValue.Create("malformed")).HasAsset, "malformed optional reads cannot crash the page");
        var status = JsonNode.Parse("""{"directory":"D:\\研究\\宠物","asset":{"id":"my-pet","displayName":"我的宠物","description":"User text","spriteVersionNumber":2,"spritesheetDataUrl":"data:image/png;base64,AQID","future":true}}""");
        var pet = NativePetPresentation.From(status);
        Check(pet.HasAsset && pet.Name == "我的宠物" && pet.Description == "User text" && pet.Id == "my-pet" && pet.Version == 2
            && pet.SpriteBytes!.SequenceEqual(new byte[] { 1, 2, 3 }), "presentation consumes the host's camelCase metadata and embedded bytes verbatim");
        Check(NativePetPresentation.DecodeSprite("https://example.com/pet.png") == null
            && NativePetPresentation.DecodeSprite("file:///C:/pet.png") == null
            && NativePetPresentation.DecodeSprite("data:image/svg+xml;base64,AQID") == null
            && NativePetPresentation.DecodeSprite("data:image/png;base64,broken") == null,
            "preview accepts embedded supported raster bytes only and cannot fetch a URL or open a path");
        Check(NativePetPresentation.DecodeSprite("data:image/webp;base64,AQID")!.Length == 3
            && NativePetPresentation.DecodeSprite("data:image/png;base64," + new string('A', 23_000_000)) == null,
            "WebP is supported and oversized payloads are rejected before decoding");
        Check(NativePetPresentation.SheetWidth / NativePetPresentation.Columns == NativePetPresentation.PreviewWidth * 2
            && NativePetPresentation.SheetHeight / NativePetPresentation.Rows == NativePetPresentation.PreviewHeight * 2,
            "settings show the first frame at the WebView's half-size geometry");
        using var editor = new NativeSettingsEditorModel(new Fake(), "project-a");
        await editor.LoadAsync("get_settings", "get_pet");
        editor.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        editor.Draft!["get_settings"]!["pet_directory"] = "D:\\different";
        Check(!editor.Draft["get_settings"]!["pet_enabled"]!.GetValue<bool>() && !NativePetPresentation.From(editor.Values["get_pet"]).HasAsset,
            "picking or typing a folder cannot enable the pet or load preview bytes before saving");
        editor.Discard(); Check(editor.Values["get_settings"]!["pet_directory"]!.GetValue<string>() == "",
            "cancel restores the configured folder without writes");
        editor.EditPreferences(new NativeSettingsWrite("get_settings", "set_settings", "settings"));
        editor.Draft!["get_settings"]!["pet_directory"] = "D:\\different";
        Check(await editor.SaveAsync() && editor.Values["get_settings"]!["future"]!.GetValue<bool>(),
            "pet settings preserve unrelated fields through the existing settings save contract");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    private sealed class Fake : INativeSettingsClient
    {
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default) => command switch
        {
            "get_settings" => Task.FromResult<JsonNode?>(new JsonObject { ["pet_enabled"] = false, ["pet_directory"] = "", ["future"] = true }),
            "get_pet" => Task.FromResult<JsonNode?>(new JsonObject { ["enabled"] = false, ["directory"] = "" }),
            "set_settings" when projectId == "project-a" && arguments["settings"]!["future"]!.GetValue<bool>() => Task.FromResult<JsonNode?>(null),
            _ => throw new InvalidOperationException("Unexpected command: " + command)
        };
    }
}
