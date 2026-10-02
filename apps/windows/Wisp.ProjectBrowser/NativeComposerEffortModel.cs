using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Composer effort edits the model profile default, matching WebView.
/// Catalog results and mutation acknowledgements are scoped to the mounted binding.</summary>
public sealed class NativeComposerEffortModel(INativeSettingsClient? client)
{
    public event Action? Changed;
    public string[] Options { get; private set; } = [];
    public string Value { get; private set; } = "";
    public bool Busy { get; private set; }
    public string? Error { get; private set; }
    private string? binding, project, session, profileId, catalogKey;
    private int generation;
    public void Reset()
    {
        generation++; binding = project = session = profileId = catalogKey = null;
        Options = []; Value = ""; Busy = false; Error = null; Changed?.Invoke();
    }
    private static string S(JsonObject row, string key) => row[key]?.GetValue<string>() ?? "";
    private static string Key(JsonObject row) => string.Join("\n", S(row, "provider"), S(row, "api_url"), S(row, "model"));
    public async Task BindAsync(string projectId, string sessionId, JsonObject? profile, CancellationToken token = default)
    {
        if (client == null || profile == null) { if (binding != null || profileId != null) Reset(); return; }
        var key = projectId + "/" + sessionId + "/" + S(profile, "id") + "/" + Key(profile);
        if (binding == key) return;
        Reset(); var current = generation;
        binding = key; project = projectId; session = sessionId; profileId = S(profile, "id"); catalogKey = Key(profile); Value = S(profile, "reasoning_effort");
        Changed?.Invoke();
        try
        {
            var catalog = await client.InvokeAsync("model_catalog_lookup", new() { ["provider"] = S(profile, "provider"), ["apiUrl"] = S(profile, "api_url"), ["model"] = S(profile, "model") }, projectId, token);
            if (current != generation) return;
            Options = (catalog?["efforts"] as JsonArray ?? []).Select(value => value?.GetValue<string>() ?? "")
                .Where(value => value is "none" or "minimal" or "low" or "medium" or "high" or "xhigh" or "max" or "ultra").Distinct().ToArray();
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (current == generation) Error = "无法读取模型思考强度：" + ex.Message; }
        finally { if (current == generation) Changed?.Invoke(); }
    }

    public async Task<bool> SaveAsync(string effort, CancellationToken token = default)
    {
        if (client == null || Busy || project == null || profileId == null || Options.Length == 0 || effort.Length > 0 && !Options.Contains(effort)) return false;
        var current = generation; var target = profileId; var scope = project; var expectedCatalog = catalogKey;
        Busy = true; Error = null; Changed?.Invoke();
        try
        {
            // Read the latest profile to avoid replacing unrelated edits with the
            // older composer snapshot. Never send a key from this control.
            var profiles = await client.InvokeAsync("list_models", new(), scope, token) as JsonArray;
            if (current != generation) return false;
            var profile = profiles?.OfType<JsonObject>().FirstOrDefault(row => S(row, "id") == target)
                ?? throw new InvalidDataException("模型已不存在，请重新打开会话。");
            if (Key(profile) != expectedCatalog) throw new InvalidDataException("模型配置已变化，请重新打开会话以刷新可用档位。");
            var draft = (JsonObject)profile.DeepClone(); draft.Remove("key"); draft["reasoning_effort"] = effort;
            var saved = await client.InvokeAsync("save_model", NativeModelDrafts.SaveArguments(draft), scope, token) as JsonArray;
            if (current != generation) return false;
            var confirmed = saved?.OfType<JsonObject>().FirstOrDefault(row => S(row, "id") == target);
            if (confirmed == null || S(confirmed, "reasoning_effort") != effort) throw new InvalidDataException("保存响应未确认所选思考强度。");
            Value = effort; return true;
        }
        catch (Exception ex) { if (current == generation) Error = "思考强度未确认保存；不会自动重试。" + ex.Message; return false; }
        finally { if (current == generation) { Busy = false; Changed?.Invoke(); } }
    }

    public async Task ReloadAsync(CancellationToken token = default)
    {
        if (client == null || Busy || project == null || session == null) return;
        var current = generation; var p = project; var s = session; var id = profileId;
        try
        {
            var profiles = await client.InvokeAsync("list_models", new(), p, token) as JsonArray;
            if (current != generation) return;
            var profile = profiles?.OfType<JsonObject>().FirstOrDefault(row => S(row, "id") == id);
            binding = null; await BindAsync(p, s, profile, token);
        }
        catch (Exception ex) { if (current == generation) { Error = "无法重新读取模型：" + ex.Message; Changed?.Invoke(); } }
    }
}
