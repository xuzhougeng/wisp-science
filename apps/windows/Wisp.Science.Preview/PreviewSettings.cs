using System.Text.Json;

namespace Wisp.Science.Preview;

internal sealed class PreviewSettings
{
    public string? DatabasePath { get; set; }
    public string Appearance { get; set; } = "system";
    public string LightPalette { get; set; } = "paper";
    public string DarkPalette { get; set; } = "charcoal";
    public Wisp.ProjectBrowser.NativeTypography Typography { get; set; } = new();
    public bool PanelVisible { get; set; }
    public string PanelTab { get; set; } = "artifacts";
    public string PanelTabs { get; set; } = "";
    private static string SettingsPath => Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
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
