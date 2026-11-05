using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;
namespace Wisp.ProjectBrowser;

public sealed record NativeRuntimeSlot(NativePanelContext Context, string Language, string Status, NativeRuntimeInfo? Info)
{
    public string LanguageLabel => Language == "python" ? "Python" : Language == "r" ? "R" : Language;
    public string Label => string.IsNullOrWhiteSpace(Context.Label) ? Context.Id : Context.Label;
}

/// A cursor for one project/frame; late reads and actions cannot retarget another draft.
public sealed class NativeRuntimeStripModel(INativeSettingsClient? settings)
{
    private readonly NativeRuntimesClient? client = settings == null ? null : new(settings);
    private string? project, session; private int generation, actionRevision; private bool reading;
    public event Action? Changed;
    public NativeRuntimeSnapshot? Snapshot { get; private set; }
    public NativeRuntimeSlot[] Slots { get; private set; } = [];
    public string? Error { get; private set; }
    public bool Busy { get; private set; }
    public bool Available => client != null && project != null;
    public bool AllIdle => Slots.All(s => s.Status is "missing" or "unavailable");
    public NativeRuntimeInspectorModel Inspector { get; } = new(settings);
    public NativeRuntimeInterpreterModel? InterpreterEditor(string contextId)
    {
        if (settings == null || project == null || Slots.FirstOrDefault(slot => slot.Context.Id == contextId) is not { } slot) return null;
        var current = generation;
        return new(settings, project, slot.Context, updated =>
        {
            if (current != generation || Snapshot == null) return;
            // Accept the acknowledged context immediately. A read started before
            // saving must not restore its old paths when the dialog is reopened.
            actionRevision++;
            Snapshot = Snapshot with { Contexts = Snapshot.Contexts.Select(c => c.Id == updated.Id ? updated : c).ToArray() };
            Slots = BuildSlots(Snapshot); Inspector.Observe(Slots); Changed?.Invoke();
        });
    }
    public void Bind(string? projectId, string? sessionId)
    {
        if (project == projectId && session == sessionId) return;
        generation++; project = projectId; session = sessionId; reading = false; Busy = false;
        Snapshot = null; Slots = []; Error = null; Changed?.Invoke();
        Inspector.Bind(projectId, sessionId);
    }
    public async Task RefreshAsync(CancellationToken token = default)
    {
        if (client == null || project == null || reading || Busy) return;
        var current = generation; var revision = actionRevision; var p = project; var s = session; reading = true;
        try
        {
            var snapshot = await client.ReadAsync(p, s, token);
            if (current != generation || revision != actionRevision) return;
            if (snapshot.ProjectId != p || snapshot.SessionId != s) throw new InvalidDataException("Runtime snapshot scope mismatch");
            Snapshot = snapshot; Slots = BuildSlots(snapshot); Inspector.Observe(Slots); Error = null;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (current == generation && revision == actionRevision) { Error = ex.Message; Snapshot = null; Slots = []; } }
        finally { if (current == generation) { reading = false; Changed?.Invoke(); } }
    }
    public async Task<NativeRuntimeObjects?> ActAsync(string action, NativeRuntimeSlot slot, CancellationToken token = default)
    {
        if (client == null || project == null || Busy || Snapshot == null || !Slots.Contains(slot)
            || action != "inspect" && Snapshot.ReadOnly) return null;
        var current = generation; var p = project; var s = session; actionRevision++; Busy = true; Error = null; Changed?.Invoke();
        try
        {
            JsonObject args = action == "start" ? new() { ["context_id"] = slot.Context.Id, ["language"] = slot.Language }
                : slot.Info is { } info ? new() { ["runtime_id"] = info.RuntimeId, ["runtime_generation"] = info.Generation }
                : throw new InvalidOperationException("Refresh the runtime first");
            var value = await client.CallAsync(action, p, s, args, token);
            return current == generation && action == "inspect" ? value?.Deserialize<NativeRuntimeObjects>(ConversationSnapshot.JsonOptions) : null;
        }
        catch (OperationCanceledException) { return null; }
        catch (Exception ex) { if (current == generation) Error = ex.Message; return null; }
        finally
        {
            if (current == generation) { Busy = false; Changed?.Invoke(); if (Error == null) await RefreshAsync(token); }
        }
    }
    public string IdleSummary(NativeLocalization locale) => string.Join(" · ", Slots.GroupBy(s => s.Status)
        .Select(g => string.Join(" · ", g.Select(s => s.LanguageLabel).Distinct()) + " " + StatusLabel(g.Key, locale)));
    public static string StatusLabel(string status, NativeLocalization locale) => locale.Text("runtime." +
        (status is "starting" or "ready" or "busy" or "stopping" or "dead" or "unavailable" ? status : "missing"));
    public static NativeRuntimeSlot[] BuildSlots(NativeRuntimeSnapshot snapshot)
    {
        var contexts = snapshot.Contexts.Where(c => c.Kind == "local" || snapshot.EnabledIds.Contains(c.Id)
            || c.Id == snapshot.DefaultContextId).ToArray();
        return contexts.SelectMany(context => new[] { "python", "r" }.Select(language =>
        {
            var info = snapshot.Runtimes.Where(r => r.Key.ProjectId == snapshot.ProjectId && r.Key.ContextId == context.Id
                && r.Key.Language == language && (r.Key.SessionId.Length == 0 || r.Key.SessionId == snapshot.SessionId))
                .OrderByDescending(r => r.Key.SessionId.Length > 0).ThenByDescending(r => r.LastActivityAtMs).FirstOrDefault();
            return new NativeRuntimeSlot(context, language, info?.Status ?? (AvailableLanguage(context, language) ? "missing" : "unavailable"), info);
        })).ToArray();
    }
    public static bool AvailableLanguage(NativePanelContext context, string language)
    {
        if (context.Kind == "local" && language == "python") return true;
        JsonObject Parse(string value) { try { return JsonNode.Parse(value) as JsonObject ?? new(); } catch (JsonException) { return new(); } }
        var config = Parse(context.ConfigJson); var capabilities = Parse(context.CapabilitiesJson);
        bool Has(JsonObject value, string key) => value[key] is JsonValue v && v.TryGetValue<string>(out var text) && !string.IsNullOrWhiteSpace(text);
        if (language == "python") return Has(config, "python_executable") || Has(config, "python_path") || Has(capabilities, "python_executable");
        if (language != "r") return false;
        if (Has(config, "rscript_executable") || Has(config, "rscript_path")) return true;
        if (Has(capabilities, "rscript_executable")) return capabilities["r_jsonlite"] is not JsonValue v || !v.TryGetValue<bool>(out var enabled) || enabled;
        return context.Kind == "local" && context.LastProbeStatus != "ok";
    }
}
