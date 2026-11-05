using System.Text.Json;
using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed record NativeConnectorParameter(string Name, string Type, string Description, bool Required, string? Default);

public static class NativeConnections
{
    public static JsonObject Transport(string kind) => kind == "http"
        ? new() { ["kind"] = "http", ["url"] = "", ["auth"] = "none", ["headers"] = new JsonArray(new JsonObject { ["name"] = "", ["value"] = "" }) }
        : new() { ["kind"] = "stdio", ["command"] = "", ["args"] = new JsonArray(), ["cwd"] = null, ["env"] = new JsonArray(new JsonObject { ["name"] = "", ["value"] = "" }) };
    public static JsonObject Create() => new() { ["id"] = Guid.NewGuid().ToString(), ["name"] = "", ["enabled"] = true, ["transport"] = Transport("stdio") };
    public static (string Name, string Value) SecretPlaceholders(JsonObject transport, JsonObject row, NativeLocalization locale)
    {
        var http = transport["kind"]?.GetValue<string>() == "http";
        var oauth = http && transport["auth"]?.GetValue<string>() == "oauth";
        var stored = row["has_value"]?.GetValue<bool>() == true || !string.IsNullOrEmpty(row["secret_ref"]?.GetValue<string>());
        return (http ? oauth ? "X-Custom-Header" : "Authorization" : "NAME",
            stored ? locale.Text("conn.secret_keep") : http ? oauth ? "value" : "Bearer token" : locale.Text("conn.secret_value"));
    }
    public static JsonObject SaveDraft(JsonObject source)
    {
        var result = (JsonObject)source.DeepClone();
        NativeProxyRoutes.Clean(result, model: false);
        if (result["name"] is JsonValue name && name.TryGetValue<string>(out var title)) result["name"] = title.Trim();
        if (result["transport"] is not JsonObject transport) return result;
        var kind = transport["kind"]?.GetValue<string>(); var secretKey = kind == "http" ? "headers" : "env";
        var endpoint = kind == "http" ? "url" : "command";
        if (transport[endpoint] is JsonValue text && text.TryGetValue<string>(out var value)) transport[endpoint] = value.Trim();
        if (transport[secretKey] is JsonArray secrets)
        {
            var clean = new JsonArray();
            foreach (var field in secrets.OfType<JsonObject>())
            {
                var key = field["name"]?.GetValue<string>()?.Trim() ?? ""; if (key.Length == 0) continue;
                var row = (JsonObject)field.DeepClone(); row["name"] = key;
                var replacement = row["value"]?.GetValue<string>()?.Trim() ?? "";
                if (replacement.Length == 0) row.Remove("value"); else row["value"] = replacement;
                clean.Add(row);
            }
            transport[secretKey] = clean;
        }
        return result;
    }
    public static JsonObject[] Bundled(JsonNode? catalog) => (catalog?["connectors"] as JsonArray)?.OfType<JsonObject>().Where(row => row["kind"]?.GetValue<string>() == "bundled").ToArray() ?? [];
    public static string Description(JsonObject connector, string? locale)
    {
        var chinese = connector["description_zh"]?.GetValue<string>() ?? "";
        return NativeLocalization.Normalize(locale) == "zh" && chinese.Length > 0
            ? chinese : connector["description"]?.GetValue<string>() ?? "";
    }
    public static string ParameterType(JsonNode? schema) => (schema as JsonObject)?["type"] switch
    {
        JsonValue value when value.TryGetValue<string>(out var text) => text,
        JsonArray array => string.Join(" | ", array.OfType<JsonValue>().Select(value => value.TryGetValue<string>(out var text) ? text : "").Where(text => text.Length > 0)),
        _ => ""
    };
    public static NativeConnectorParameter[] Parameters(JsonNode? schema) => (schema as JsonObject)?["properties"] is JsonObject properties
        ? properties.Select(pair => new NativeConnectorParameter(pair.Key, ParameterType(pair.Value), (pair.Value as JsonObject)?["description"]?.GetValue<string>() ?? "",
            (schema["required"] as JsonArray)?.Any(value => value is JsonValue item && item.TryGetValue<string>(out var text) && text == pair.Key) == true,
            pair.Value is JsonObject property && property.ContainsKey("default") ? property["default"]?.ToJsonString() ?? "null" : null)).ToArray() : [];
    public static string Json(JsonNode? value) => value?.ToJsonString(new JsonSerializerOptions { WriteIndented = true }) ?? "null";
}
