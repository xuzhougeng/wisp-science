using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

public sealed record NativeAcpMethod(string Id, string Name, string Description, string Kind);
public sealed record NativeAcpInfo(string Version, NativeAcpMethod[] Methods);

// Connection checks belong to the tested profile. Keep only presentation data from the reply.
public sealed class NativeAcpChecks
{
    private readonly Dictionary<string, (JsonObject Profile, NativeAcpInfo Info)> checks = [];
    public void Store(JsonObject profile, JsonNode? reply)
    {
        var id = profile["id"]?.GetValue<string>();
        if (string.IsNullOrWhiteSpace(id) || reply is not JsonObject obj ||
            obj["protocolVersion"] is not JsonValue protocol || !protocol.TryGetValue<int>(out var version) ||
            version is < 0 or > ushort.MaxValue || obj["authMethods"] is not JsonArray methods)
            throw new InvalidDataException("Invalid ACP connection response.");
        var implementation = obj["implementation"];
        var name = implementation?["title"]?.GetValue<string>() ?? implementation?["name"]?.GetValue<string>();
        var number = implementation?["version"]?.GetValue<string>();
        var label = "ACP v" + version;
        if (!string.IsNullOrEmpty(name)) label = name + (string.IsNullOrEmpty(number) ? "" : " " + number) + " · " + label;
        var auth = methods.Select(m => new NativeAcpMethod(m?["id"]?.GetValue<string>() ?? "",
            m?["name"]?.GetValue<string>() ?? "", m?["description"]?.GetValue<string>() ?? "", m?["type"]?.GetValue<string>() ?? "")).ToArray();
        if (auth.Any(m => m.Id.Length == 0 || m.Name.Length == 0))
            throw new InvalidDataException("Invalid ACP authentication method.");
        checks[id] = ((JsonObject)profile.DeepClone(), new(label, auth));
    }
    public NativeAcpInfo? For(JsonObject profile) =>
        profile["id"] is JsonValue key && key.TryGetValue<string>(out var id) && !string.IsNullOrEmpty(id)
        && checks.TryGetValue(id, out var value) && JsonNode.DeepEquals(profile, value.Profile) ? value.Info : null;
    public void Retain(IEnumerable<JsonObject> profiles)
    {
        var valid = profiles.Where(p => For(p) != null).Select(p => p["id"]!.GetValue<string>()).ToHashSet();
        foreach (var id in checks.Keys.Where(id => !valid.Contains(id)).ToArray()) checks.Remove(id);
    }
}
