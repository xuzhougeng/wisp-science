using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Confirmed composer settings. Each load/save is bound to a project and session;
/// uncertain writes are never retried, and late replies cannot update another conversation.</summary>
public sealed class NativeComposerOptionsModel(INativeSettingsClient? client)
{
    public event Action? Changed;
    public ComposerSessionOptions? Session { get; private set; }
    public ComposerFailureAnalysis? Failure { get; private set; }
    public NativePanelContexts? Contexts { get; private set; }
    public bool MemoryEnabled { get; private set; }
    public bool Busy { get; private set; }
    public bool Writable { get; set; }
    public bool CanEdit => Writable && !Busy && Session != null && Error == null;
    public string? Error { get; private set; }
    public string Reviewer { get; private set; } = "http:";
    public string? GlobalDefaultContext { get; private set; }
    public ComposerOptionChoice[] Reviewers { get; private set; } = [];
    public ComposerOptionChoice[] Specialists { get; private set; } = [];
    private string? project, session;
    private int generation;
    public static string S(JsonNode? node, string field) => node?[field]?.GetValue<string>() ?? "";
    private static T Decode<T>(JsonNode? value) where T : class => value?.Deserialize<T>(ConversationSnapshot.JsonOptions)
        ?? throw new InvalidDataException("设置响应不完整。");
    public void Reset()
    {
        generation++; project = session = null; Session = null; Failure = null; Contexts = null;
        Busy = false; Writable = false; Error = null; Reviewers = []; Specialists = []; Changed?.Invoke();
    }
    public void Bind(string projectId, string sessionId) { Reset(); project = projectId; session = sessionId; }

    public async Task LoadAsync(CancellationToken token = default)
    {
        if (client == null || Busy || project == null || session == null) return;
        var current = generation; var p = project; var s = session;
        Busy = true; Error = null; Changed?.Invoke();
        try { await ReadAsync(p, s, current, token); }
        catch (Exception ex) { if (current == generation) Error = "无法读取对话选项：" + ex.Message; }
        finally { if (current == generation) { Busy = false; Changed?.Invoke(); } }
    }
    private async Task ReadAsync(string p, string s, int current, CancellationToken token)
    {
        var commands = new[] { "native_conversation_options", "get_auto_failure_analysis_settings", "get_memory_view",
            "list_specialists", "native_conversation_panel_side_chat_options", "native_conversation_panel_contexts", "get_default_execution_context" };
        var values = await Task.WhenAll(commands.Select(command => client!.InvokeAsync(command,
            command.StartsWith("native_conversation_") ? new() { ["session_id"] = s } : new(), p, token)));
        if (current != generation) return;
        var scoped = Decode<ComposerSessionOptions>(values[0]);
        if (scoped.SessionId != s) throw new InvalidDataException("会话选项响应不匹配。");
        var failure = Decode<ComposerFailureAnalysis>(values[1]);
        var memory = values[2]?["enabled"]?.GetValue<bool>() ?? throw new InvalidDataException("缺少记忆开关状态。");
        var specialists = Decode<JsonObject[]>(values[3]);
        var choices = new List<ComposerOptionChoice> { new("http:", "默认 HTTP 模型"), new("follow_session", "跟随会话") };
        // This host projection shares ModelProfile::is_chat_model with WebView,
        // including exact image/video model IDs and explicit capabilities.
        choices.AddRange(Decode<NativeSideChatOption[]>(values[4])
            .Select(m => new ComposerOptionChoice(m.Key, m.Label + (m.Kind == "acp" ? " · ACP" : ""))));
        var contexts = Decode<NativePanelContexts>(values[5]);
        Session = scoped; Failure = failure; MemoryEnabled = memory; Contexts = contexts;
        GlobalDefaultContext = values[6]?.GetValue<string>();
        Reviewers = choices.ToArray(); Reviewer = ReviewerKey(specialists.FirstOrDefault(v => S(v, "id") == "reviewer"));
        Specialists = new[] { new ComposerOptionChoice("", "无") }.Concat(specialists
            .Where(v => S(v, "id") is not ("reviewer" or "reader" or "archivist" or "recap"))
            .Select(v => new ComposerOptionChoice(S(v, "id"), S(v, "name")))).ToArray();
    }
    public static string ReviewerKey(JsonObject? spec) => S(spec?["review_backend"], "kind") switch
    {
        "follow_session" => "follow_session", "acp_agent" => "acp:" + S(spec?["review_backend"], "profile_id"),
        "http_model" => "http:" + S(spec?["review_backend"], "profile_id"), _ => "http:" + S(spec, "model_id")
    };
    private async Task<bool> SaveAsync(Func<string, string, int, Task> write, CancellationToken token)
    {
        if (client == null || !CanEdit || project == null || session == null) return false;
        var current = generation; var p = project; var s = session;
        Busy = true; Error = null; Changed?.Invoke();
        try
        {
            await write(p, s, current);
            if (current != generation) return false;
            await ReadAsync(p, s, current, token);
            return current == generation;
        }
        catch (Exception ex) { if (current == generation) Error = "设置未确认保存，请重新读取后核对；不会自动重试。" + ex.Message; return false; }
        finally { if (current == generation) { Busy = false; Changed?.Invoke(); } }
    }
    private Task<bool> SaveSessionAsync(JsonObject change, CancellationToken token) => SaveAsync(async (p, s, _) =>
    {
        var result = Decode<ComposerSessionOptions>(await client!.InvokeAsync("native_conversation_options_set",
            new() { ["session_id"] = s, ["change"] = change }, p, token));
        if (result.SessionId != s) throw new InvalidDataException("会话选项响应不匹配。");
    }, token);
    public Task<bool> SetToggleAsync(string kind, bool enabled, bool confirmed = false, CancellationToken token = default)
    {
        if (kind is not ("full_permission" or "delegation" or "auto_review") || kind == "full_permission" && enabled && !confirmed) return Task.FromResult(false);
        var change = new JsonObject { ["kind"] = kind, ["enabled"] = enabled };
        if (kind == "full_permission") change["confirmed"] = confirmed;
        return SaveSessionAsync(change, token);
    }
    public Task<bool> SetCompletionAsync(string policy, bool autoResume, CancellationToken token = default) =>
        Session?.Delegation != true || policy is not ("inline" or "background") ? Task.FromResult(false)
        : SaveSessionAsync(new() { ["kind"] = "completion", ["policy"] = policy, ["auto_resume"] = policy == "background" && autoResume }, token);
    public Task<bool> SetSpecialistAsync(string id, CancellationToken token = default) =>
        Session?.SpecialistLocked != false || !Specialists.Any(v => v.Id == id) ? Task.FromResult(false)
        : SaveSessionAsync(new() { ["kind"] = "specialist", ["id"] = id }, token);
    public Task<bool> SetFailureAsync(bool enabled, int threshold, int minimum, CancellationToken token = default) =>
        threshold is < 1 or > 100 || minimum is < 1 or > 100 ? Task.FromResult(false) : SaveAsync(async (p, _, _) =>
        {
            _ = Decode<ComposerFailureAnalysis>(await client!.InvokeAsync("set_auto_failure_analysis_settings", new()
            { ["settings"] = new JsonObject { ["enabled"] = enabled, ["failure_rate_threshold"] = threshold, ["minimum_failures"] = minimum } }, p, token));
        }, token);
    public Task<bool> SetMemoryAsync(bool enabled, CancellationToken token = default) => SaveAsync(async (p, _, _) =>
    {
        var value = await client!.InvokeAsync("set_memory_enabled", new() { ["enabled"] = enabled }, p, token);
        if (value?["enabled"]?.GetValue<bool>() != enabled) throw new InvalidDataException("记忆设置未确认。");
    }, token);
    public Task<bool> SetReviewerAsync(string id, CancellationToken token = default) => !Reviewers.Any(v => v.Id == id) ? Task.FromResult(false)
        : SaveAsync(async (p, _, current) =>
        {
            var specs = Decode<JsonObject[]>(await client!.InvokeAsync("list_specialists", new(), p, token));
            if (current != generation) return;
            var spec = specs.FirstOrDefault(v => S(v, "id") == "reviewer") ?? throw new InvalidDataException("审查专家已不存在。");
            var backend = new JsonObject { ["kind"] = id == "follow_session" ? "follow_session" : id.StartsWith("acp:") ? "acp_agent" : "http_model" };
            if (id != "follow_session") backend["profile_id"] = id[(id.IndexOf(':') + 1)..];
            if (id.StartsWith("http:")) spec["model_id"] = id[5..];
            spec["review_backend"] = backend;
            var saved = Decode<JsonObject[]>(await client.InvokeAsync("save_specialist_cmd", new() { ["spec"] = spec }, p, token));
            if (ReviewerKey(saved.FirstOrDefault(v => S(v, "id") == "reviewer")) != id) throw new InvalidDataException("审查模型未确认。");
        }, token);
    public Task<bool> SetContextAsync(string id, bool enabled, CancellationToken token = default) => SaveAsync(async (p, s, _) =>
    {
        var ids = await new NativePanelClient(client!).SetContextEnabledAsync(p, s, id, enabled, token);
        if (ids.Contains(id) != enabled) throw new InvalidDataException("计算环境未确认。");
    }, token);
    public Task<bool> SetDefaultContextAsync(string id, CancellationToken token = default) => SaveAsync(async (p, s, _) =>
        await new NativePanelClient(client!).SetDefaultContextAsync(p, s, id, token), token);
}
