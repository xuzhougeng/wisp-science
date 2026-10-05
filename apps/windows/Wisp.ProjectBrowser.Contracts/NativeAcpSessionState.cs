using System.Text.Json.Nodes;
using System.Text.Json.Serialization;

namespace Wisp.ProjectBrowser.Contracts;

public sealed record NativeAcpSessionState(
    [property: JsonPropertyName("frameId")] string FrameId,
    [property: JsonPropertyName("modes")] JsonObject? Modes,
    [property: JsonPropertyName("configOptions")] JsonObject[]? ConfigOptions)
{
    public static string? Text(JsonNode? node) => node is JsonValue value && value.TryGetValue<string>(out var text) ? text : null;
    [JsonIgnore] public string? CurrentMode => Text(Modes?["currentModeId"]);
    [JsonIgnore] public NativeAcpChoice[] ModeChoices => Modes?["availableModes"] is JsonArray rows
        ? rows.OfType<JsonObject>().Select(row => new NativeAcpChoice(Text(row["id"]) ?? "", Text(row["name"]) ?? Text(row["id"]) ?? ""))
            .Where(row => row.Id.Length > 0).ToArray() : [];
    [JsonIgnore] public bool HasModeConfig => ConfigOptions?.Any(option => Text(option["id"]) == "mode"
        || string.Equals(Text(option["name"]), "mode", StringComparison.OrdinalIgnoreCase)) == true;
    public static NativeAcpChoice[] Choices(JsonObject option)
    {
        if (option["options"] is not JsonArray rows) return [];
        return rows.OfType<JsonObject>().SelectMany(row => Text(row["value"]) != null ? [row]
                : row["options"] is JsonArray group ? group.OfType<JsonObject>() : [])
            .Where(row => Text(row["value"]) is { Length: > 0 })
            .Select(row => new NativeAcpChoice(Text(row["value"])!, Text(row["name"]) ?? Text(row["value"])!)).ToArray();
    }
    public bool Allows(string id, JsonNode value)
    {
        var option = ConfigOptions?.FirstOrDefault(option => Text(option["id"]) == id);
        return option != null && (Text(option["type"]) switch
        {
            "boolean" => value is JsonValue boolean && boolean.TryGetValue<bool>(out _),
            "select" => Text(value) is { } selected && Choices(option).Any(choice => choice.Id == selected),
            _ => false
        });
    }
}
public sealed record NativeAcpChoice(string Id, string Label);
