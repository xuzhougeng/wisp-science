using System.Text.Json;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Display projection shared by the native composer and its regression checks.</summary>
public static class NativeComputeMenu
{
    public sealed record Resource(string Id, string Name, string Icon, bool Attached, bool Default, bool WslDefault)
    {
        public string StateKey => Attached ? "compute.attached" : Default ? "compute.auto_attaches" : "compute.not_attached";
    }

    public static string DefaultId(string? stored, string? global)
    {
        var id = string.IsNullOrWhiteSpace(stored) ? global?.Trim() : stored.Trim();
        return string.IsNullOrEmpty(id) || id == "local" ? "local" : id;
    }
    public static string Label(NativePanelContext context) => string.IsNullOrWhiteSpace(context.Label) ? context.Id : context.Label;
    public static ComposerOptionChoice[] DefaultChoices(NativePanelContexts? contexts, NativeLocalization locale) =>
        new[] { new ComposerOptionChoice("local", locale.Text("compute.default_local")) }.Concat((contexts?.Contexts ?? [])
            .Where(c => c.Kind is "ssh" or "wsl").Select(c => new ComposerOptionChoice(c.Id, Label(c)))).ToArray();

    public static Resource[] Resources(NativePanelContexts? contexts, IEnumerable<string> sshAliases, string? globalDefault, string query)
    {
        var id = DefaultId(contexts?.DefaultContext?.ContextId, globalDefault);
        var enabled = contexts?.EnabledIds ?? [];
        var search = query.Trim();
        Resource Row(string key, string name, string icon, bool wsl = false) => new(key, name, icon, enabled.Contains(key), id == key, wsl);
        return sshAliases.Select(alias => Row("ssh:" + alias, alias, "server"))
            .Concat((contexts?.Contexts ?? []).Where(c => c.Kind == "wsl").Select(c => Row(c.Id, Label(c), "terminal", IsWslDefault(c.ConfigJson))))
            .Where(row => search.Length == 0 || row.Name.Contains(search, StringComparison.OrdinalIgnoreCase)).ToArray();
    }
    private static bool IsWslDefault(string json)
    {
        try { using var document = JsonDocument.Parse(json); return document.RootElement.ValueKind == JsonValueKind.Object
            && document.RootElement.TryGetProperty("is_default", out var flag) && flag.ValueKind == JsonValueKind.True; }
        catch (JsonException) { return false; }
    }

    public static Resource[] AttachCandidates(NativePanelContexts? contexts, IEnumerable<string> aliases, string query)
    {
        var search = query.Trim();
        return Resources(contexts, aliases, null, "").Where(row => !row.Attached &&
            (row.Name.Contains(search, StringComparison.OrdinalIgnoreCase) ||
             row.Icon == "terminal" && row.Id.Contains(search, StringComparison.OrdinalIgnoreCase))).ToArray();
    }

    public static string CapabilitySummary(NativePanelContext context)
    {
        try
        {
            using var doc = JsonDocument.Parse(context.CapabilitiesJson);
            var root = doc.RootElement;
            if (root.ValueKind == JsonValueKind.Object)
            {
                string? Get(string key) => root.TryGetProperty(key, out var val) && val.ValueKind == JsonValueKind.String ? val.GetString() : null;
                var parts = new List<string>();
                var platform = string.Join("/", new[] { Get("os"), Get("arch") }.Where(s => !string.IsNullOrEmpty(s)));
                if (platform.Length != 0) parts.Add(platform);
                foreach (var key in new[] { "gpu_summary", "scheduler", "python", "r_version" }) if (Get(key) is { Length: > 0 } value) parts.Add(value);
                if (Get("probe_skill") != null && (!root.TryGetProperty("gpu_summary", out var gpu) || gpu.ValueKind == JsonValueKind.Null)) parts.Add("No GPU");
                if (Get("privilege") is { } privilege) parts.Add(privilege);
                if (parts.Count > 0) return string.Join(" · ", parts);
            }
        }
        catch (JsonException) { }
        return context.LastProbeStatus ?? "not probed";
    }
}
