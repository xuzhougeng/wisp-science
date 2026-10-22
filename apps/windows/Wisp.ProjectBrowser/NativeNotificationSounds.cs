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

    public static (string Id, string Label)[] Catalog(JsonNode? value)
        => (value as JsonArray)?.OfType<JsonObject>()
            .Select(row => (Id: Text(row["id"]), Label: Text(row["label"])))
            .Where(row => !string.IsNullOrWhiteSpace(row.Id))
            .DistinctBy(row => row.Id, StringComparer.Ordinal)
            .Select(row => (row.Id, Label(row.Id, row.Label))).ToArray() ?? [];

    public static (string Id, string Label)[] Choices(JsonNode? value, string current)
    {
        var catalog = Catalog(value);
        if (catalog.Length == 0) return [];
        var choices = new List<(string Id, string Label)> { ("", "系统默认") };
        choices.AddRange(catalog);
        if (current.Length > 0 && !catalog.Any(row => row.Id == current))
            choices.Add((current, $"当前值：{current}（此平台不可用，请重新选择）"));
        return choices.ToArray();
    }

    public static bool CanPreview(JsonNode? value, string current)
    {
        var catalog = Catalog(value);
        return catalog.Length > 0 && (current.Length == 0 || catalog.Any(row => row.Id == current));
    }

    private static string Text(JsonNode? value) => value is JsonValue scalar && scalar.TryGetValue<string>(out var text) ? text : "";
    private static string Label(string id, string fallback) => id switch
    {
        "Notification.Default" => "通知", "Notification.IM" => "即时消息", "Notification.Mail" => "邮件",
        "Notification.Reminder" => "提醒", "Notification.SMS" => "短信", "SystemAsterisk" => "星号",
        "SystemExclamation" => "感叹号", "SystemNotification" => "系统通知",
        _ => string.IsNullOrWhiteSpace(fallback) ? id : fallback
    };
}
