using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

// General, Session, Pet and manual project sync edit one settings document. Keep
// only edited fields between mounts; each new page starts with a fresh read.
public sealed class NativeWindowPreferences : IDisposable
{
    private readonly JsonObject edits = new();
    // A read returns credential presence only. Even an older host echoing
    // a token must never populate a password box or replay that credential.
    public static JsonObject EditableSnapshot(JsonObject settings)
    {
        var result = (JsonObject)settings.DeepClone();
        if (result.ContainsKey("sync_relay_token")) result["sync_relay_token"] = "";
        return result;
    }
    internal void Capture(JsonObject baseline, JsonObject form)
    {
        edits.Clear();
        foreach (var (key, value) in form)
            if (!baseline.ContainsKey(key) || !JsonNode.DeepEquals(value, baseline[key]))
                edits[key] = value?.DeepClone();
    }
    internal void ApplyTo(JsonObject form)
    {
        foreach (var (key, value) in edits) form[key] = value?.DeepClone();
    }
    public string? PreviewLocale => edits["locale"]?.GetValue<string>();
    internal void Clear() => edits.Clear();
    public void Dispose() => Clear();
}
