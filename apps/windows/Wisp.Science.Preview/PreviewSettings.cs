using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class PreviewSettings
{
    public string? DatabasePath { get; set; }
    public string Appearance { get; set; } = "system";
    public bool ModelSwitchWarningDisabled { get; set; }
    public Wisp.ProjectBrowser.NativeContextPanelGeometry? ContextUsageGeometry { get; set; }
    public double? ComposerTextHeight { get; set; }
    public string LightPalette { get; set; } = "paper";
    public string DarkPalette { get; set; } = "charcoal";
    public Wisp.ProjectBrowser.NativeTypography Typography { get; set; } = new();
    public Wisp.ProjectBrowser.NativeInputPreferences InputPreferences { get; set; } = new();
    public JsonObject? CachedAppearance { get; set; }

    public JsonObject ReadAppearanceCache()
    {
        // Older previews kept only the rendering values. Preserve them on the
        // first launch after upgrade, then retain exact values (including empty
        // font-family choices and custom CSS) from subsequent replies.
        var defaultFonts = new NativeTypography(); var fonts = Typography ?? defaultFonts; var input = InputPreferences ?? new();
        var cache = new JsonObject { ["theme"] = Appearance, ["light_palette"] = LightPalette, ["dark_palette"] = DarkPalette,
            ["ui_font_size"] = fonts.UiSize, ["code_font_size"] = fonts.CodeSize,
            ["ui_font_family"] = fonts.UiFamily == defaultFonts.UiFamily ? "" : fonts.UiFamily,
            ["code_font_family"] = fonts.CodeFamily == defaultFonts.CodeFamily ? "" : fonts.CodeFamily,
            ["selection_popup_enabled"] = input.SelectionPopupEnabled, ["send_with_modifier"] = input.SendWithModifier };
        if (CachedAppearance != null)
            foreach (var (key, value) in NativeAppearanceSnapshot.ForCache(CachedAppearance)) cache[key] = value?.DeepClone();
        return cache;
    }
    public bool PanelVisible { get; set; }
    // WebView keeps the inspector width for this window only; each launch starts at 400.
    [System.Text.Json.Serialization.JsonIgnore]
    public double PanelWidth { get; set; } = Wisp.ProjectBrowser.PreviewLayout.DefaultPanelWidth;
    public string PanelTab { get; set; } = Wisp.ProjectBrowser.Contracts.NativePanelTabs.Home;
    public string PanelTabs { get; set; } = "";
    public bool AssistantProjectsVisible { get; set; } = true;
    public bool AssistantCalendarVisible { get; set; } = true;
    public string SessionSort { get; set; } = "newest";
    public string SessionGroup { get; set; } = "folder";
    private static string SettingsPath => Environment.GetEnvironmentVariable("WISP_PREVIEW_SETTINGS_PATH") is { Length: > 0 } path
        && Path.IsPathFullyQualified(path) ? path : Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "WispSciencePreview", "settings.json");

    public static PreviewSettings Load()
    {
        try { return JsonSerializer.Deserialize<PreviewSettings>(File.ReadAllText(SettingsPath)) ?? new(); }
        catch (Exception ex) when (ex is IOException or UnauthorizedAccessException or JsonException) { return new(); }
    }

    public void Save()
    {
        Directory.CreateDirectory(Path.GetDirectoryName(SettingsPath)!);
        var temporary = SettingsPath + "." + Guid.NewGuid().ToString("N") + ".tmp";
        File.WriteAllText(temporary, JsonSerializer.Serialize(this));
        File.Move(temporary, SettingsPath, overwrite: true);
    }

    public string ResolveDatabase() => Environment.GetEnvironmentVariable("WISP_BROWSER_DATABASE") ?? DatabasePath
        ?? Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "science.wisp-science", "wisp-science", "wisp.sqlite");
}
