using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// <summary>Choices come from the connected host's platform, never from guessed sound IDs.</summary>
public static class NativeNotificationSounds
{
    public static bool Supported(JsonNode? capabilities)
    {
        var commands = ((capabilities as JsonObject)?["commands"] as JsonArray)?.OfType<JsonValue>()
            .Select(value => value.TryGetValue<string>(out var name) ? name : "").ToHashSet();
        return commands?.Contains("list_notification_sounds") == true
            && commands.Contains("preview_notification_sound");
    }

    public static (string Id, string Label)[] Catalog(JsonNode? value, string locale = "zh")
        => (value as JsonArray)?.OfType<JsonObject>()
            .Select(row => (Id: Text(row["id"]), Label: Text(row["label"])))
            .Where(row => !string.IsNullOrWhiteSpace(row.Id))
            .DistinctBy(row => row.Id, StringComparer.Ordinal)
            .Select(row => (row.Id, Label(row.Id, row.Label, locale))).ToArray() ?? [];

    public static (string Id, string Label)[] Choices(JsonNode? value, string current, string locale = "zh")
    {
        var catalog = Catalog(value, locale);
        if (catalog.Length == 0) return [];
        var chinese = NativeLocalization.Normalize(locale) == "zh";
        var choices = new List<(string Id, string Label)> { ("", chinese ? "系统默认" : "System default") };
        choices.AddRange(catalog);
        if (current.Length > 0 && !catalog.Any(row => row.Id == current))
            choices.Add((current, chinese ? $"当前值：{current}（此平台不可用，请重新选择）" : $"Current: {current} (unavailable on this platform; choose another sound)"));
        return choices.ToArray();
    }

    public static bool CanPreview(JsonNode? value, string current)
    {
        var catalog = Catalog(value);
        return catalog.Length > 0 && (current.Length == 0 || catalog.Any(row => row.Id == current));
    }

    private static string Text(JsonNode? value) => value is JsonValue scalar && scalar.TryGetValue<string>(out var text) ? text : "";
    private static string Label(string id, string fallback, string locale) => NativeLocalization.Normalize(locale) == "zh" ? id switch
    {
        "Notification.Default" => "通知", "Notification.IM" => "即时消息", "Notification.Mail" => "邮件",
        "Notification.Reminder" => "提醒", "Notification.SMS" => "短信", "SystemAsterisk" => "星号",
        "SystemExclamation" => "感叹号", "SystemNotification" => "系统通知",
        _ => string.IsNullOrWhiteSpace(fallback) ? id : fallback
    } : id switch
    {
        "Notification.Default" => "Notification", "Notification.IM" => "Instant message", "Notification.Mail" => "Mail",
        "Notification.Reminder" => "Reminder", "Notification.SMS" => "SMS", "SystemAsterisk" => "Asterisk",
        "SystemExclamation" => "Exclamation", "SystemNotification" => "System notification",
        _ => string.IsNullOrWhiteSpace(fallback) ? id : fallback
    };
}
