using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// <summary>Host-owned interaction preferences, with the same defaults as WebView.</summary>
public sealed record NativeInputPreferences(bool SendWithModifier = false, bool SelectionPopupEnabled = true)
{
    public static NativeInputPreferences From(JsonObject preferences) => new(
        Read(preferences, "send_with_modifier", false), Read(preferences, "selection_popup_enabled", true));

    private static bool Read(JsonObject preferences, string key, bool fallback) =>
        preferences[key] is JsonValue value && value.TryGetValue<bool>(out var result) ? result : fallback;

    public string Hint => SendWithModifier ? "Ctrl+Enter 发送 · Enter 换行" : "Enter 发送 · Shift+Enter 换行";
}
