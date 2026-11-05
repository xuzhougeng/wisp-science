using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>One database/project-bound editing session. Failed writes retain drafts and are never retried.</summary>
public sealed class AppearanceSettingsModel(INativeSettingsClient client, string? projectId, Func<JsonObject>? local = null)
{
    /// <summary>Footer theme changes patch a fresh host snapshot, preserving every other preference.
    /// A lost acknowledgement is surfaced to the caller; this method never retries the write.</summary>
    public static async Task<JsonObject> ChangeThemeAsync(INativeSettingsClient client, string theme,
        CancellationToken cancellationToken = default, Func<JsonObject>? local = null)
    {
        if (theme is not ("system" or "light" or "dark")) throw new ArgumentException("Invalid theme", nameof(theme));
        var editor = new AppearanceSettingsModel(client, null, local);
        await editor.LoadAsync(cancellationToken);
        editor.Draft!["theme"] = theme;
        return await editor.SaveAsync(cancellationToken) ?? throw new InvalidDataException("Theme save was not confirmed");
    }
    private JsonObject? snapshot;
    public static async Task<JsonObject> StepFontAsync(INativeSettingsClient client, bool code, int direction, CancellationToken token = default, Func<JsonObject>? local = null)
    {
        if (direction is not (1 or -1)) throw new ArgumentOutOfRangeException(nameof(direction));
        var editor = new AppearanceSettingsModel(client, null, local);
        await editor.LoadAsync(token);
        var draft = editor.Draft!;
        var typography = NativeTypography.From(draft);
        var current = (int)(code ? typography.CodeSize : typography.UiSize);
        var next = Math.Clamp(current + direction, 0, 30);
        if (next == current) return draft;
        draft[code ? "code_font_size" : "ui_font_size"] = next;
        return await editor.SaveAsync(token) ?? throw new InvalidDataException("Font size save was not confirmed");
    }
    public JsonObject? Draft { get; private set; }
    public bool Busy { get; private set; }
    public bool HasChanges => !JsonNode.DeepEquals(snapshot, Draft);

    public async Task LoadAsync(CancellationToken cancellationToken = default)
    {
        if (Busy) return;
        Busy = true;
        try
        {
            var loaded = (await client.InvokeAsync("get_appearance_prefs", new(), projectId, cancellationToken)) as JsonObject
                ?? throw new InvalidDataException("Missing appearance preferences.");
            var effective = NativeAppearanceSnapshot.Resolve(loaded, local);
            if (!HasChanges) Draft = (JsonObject)effective.DeepClone();
            snapshot = effective;
        }
        finally { Busy = false; }
    }

    public void Discard() { if (!Busy) Draft = (JsonObject?)snapshot?.DeepClone(); }

    public async Task<JsonObject?> SaveAsync(CancellationToken cancellationToken = default)
    {
        if (Busy || Draft is null) return null;
        Busy = true;
        try
        {
            var saved = (await client.InvokeAsync("set_appearance_prefs", new() { ["prefs"] = Draft.DeepClone() },
                projectId, cancellationToken)) as JsonObject ?? throw new InvalidDataException("Missing saved preferences.");
            snapshot = (JsonObject)saved.DeepClone();
            Draft = (JsonObject)saved.DeepClone();
            return saved;
        }
        finally { Busy = false; }
    }
}
