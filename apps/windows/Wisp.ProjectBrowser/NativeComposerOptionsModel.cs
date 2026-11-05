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
    public NativePanelContexts? Contexts { get; private set; }
    public bool Busy { get; private set; }
    public bool Writable { get; set; }
    public bool IsDraft => project != null && session == null;
    private bool draftLoaded;
    public bool CanEdit => Writable && !Busy && (Session != null || IsDraft && draftLoaded) && Error == null;
    public bool CanEditCompute => CanEdit && Contexts is { ReadOnly: false };
    public string[] SshAliases { get; private set; } = [];
    public string? Error { get; private set; }
    private bool saveError;
    public string? ErrorText(NativeLocalization locale) => Error == null ? null : (saveError
        ? locale.Choose("设置未确认保存，请重新读取后核对；不会自动重试。", "The change was not confirmed. Reload to check before trying again. ")
        : locale.Choose("无法读取对话选项：", "Could not load conversation options: ")) + Error;
    public string Reviewer { get; private set; } = "http:";
    public string? GlobalDefaultContext { get; private set; }
    public ComposerOptionChoice[] Reviewers { get; private set; } = [];
    public ComposerOptionChoice[] Specialists { get; private set; } = [];
    private string? project, session;
    private int generation;
    public static string S(JsonNode? node, string field) => node?[field]?.GetValue<string>() ?? "";
    private static T Decode<T>(JsonNode? value) where T : class => value?.Deserialize<T>(ConversationSnapshot.JsonOptions)
        ?? throw new InvalidDataException("Incomplete settings response.");
    public void Reset()
    {
        generation++; project = session = null; Session = null; Contexts = null;
        Busy = false; Writable = false; draftLoaded = false; Error = null; Reviewer = "http:"; Reviewers = []; Specialists = []; SshAliases = []; GlobalDefaultContext = null; Changed?.Invoke();
    }
    public void Bind(string projectId, string? sessionId) { Reset(); project = projectId; session = sessionId; }

    public async Task LoadAsync(CancellationToken token = default)
    {
        if (client == null || Busy || project == null) return;
        var current = generation; var p = project; var s = session;
        Busy = true; Error = null; saveError = false; Changed?.Invoke();
        try { await ReadAsync(p, s, current, token); }
        catch (Exception ex) { if (current == generation) Error = ex.Message; }
        finally { if (current == generation) { Busy = false; Changed?.Invoke(); } }
    }
    private async Task ReadAsync(string p, string? s, int current, CancellationToken token)
    {
        if (s == null) { await ReadDraftAsync(p, current, token); return; }
        var commands = new[] { "native_conversation_options",
            "list_specialists", "native_conversation_panel_side_chat_options", "native_conversation_panel_contexts", "get_default_execution_context", "list_ssh_hosts" };
        var values = await Task.WhenAll(commands.Select(command => client!.InvokeAsync(command,
            command.StartsWith("native_conversation_") ? new() { ["session_id"] = s } : new(), p, token)));
        if (current != generation) return;
        var scoped = Decode<ComposerSessionOptions>(values[0]);
        if (scoped.SessionId != s) throw new InvalidDataException("Conversation options belong to a different session.");
        var specialists = Decode<JsonObject[]>(values[1]);
        var choices = new List<ComposerOptionChoice> { new("http:", "Default HTTP model"), new("follow_session", "Follow session backend") };
        // This host projection shares ModelProfile::is_chat_model with WebView,
        // including exact image/video model IDs and explicit capabilities.
        choices.AddRange(Decode<NativeSideChatOption[]>(values[2])
            .Select(m => new ComposerOptionChoice(m.Key, m.Label + (m.Kind == "acp" ? " · ACP" : ""))));
        var contexts = Decode<NativePanelContexts>(values[3]);
        var aliases = Decode<JsonObject[]>(values[5]).Select(host => S(host, "alias")).Where(alias => alias.Length > 0).ToArray();
        Session = scoped; Contexts = contexts;
        SshAliases = aliases;
        GlobalDefaultContext = values[4]?.GetValue<string>();
        ApplyChoices(specialists, choices);
    }
    private async Task ReadDraftAsync(string p, int current, CancellationToken token)
    {
        // A draft has no session identity. Use project/global reads only, matching
        // the WebView menu refresh; never create a session just to inspect options.
        var commands = new[] { "list_specialists", "list_models", "list_acp_agents", "list_execution_contexts", "get_default_execution_context", "list_ssh_hosts" };
        var values = await Task.WhenAll(commands.Select(command => client!.InvokeAsync(command, new(), p, token)));
        if (current != generation) return;
        var specialists = Decode<JsonObject[]>(values[0]);
        var models = Decode<JsonObject[]>(values[1]); var agents = Decode<JsonObject[]>(values[2]);
        var contexts = Decode<NativePanelContext[]>(values[3]);
        var aliases = Decode<JsonObject[]>(values[5]).Select(host => S(host, "alias")).Where(alias => alias.Length > 0).ToArray();
        var choices = new List<ComposerOptionChoice> { new("http:", "Default HTTP model"), new("follow_session", "Follow session backend") };
        choices.AddRange(models.Where(NativeModelPresentation.IsChat).Select(m => new ComposerOptionChoice("http:" + S(m, "id"), NativeModelPresentation.Label(m))));
        choices.AddRange(agents.Select(a => new ComposerOptionChoice("acp:" + S(a, "id"), S(a, "label") + " · ACP")));
        Session = null; Contexts = new(contexts, [], false, new(null));
        SshAliases = aliases; GlobalDefaultContext = values[4]?.GetValue<string>();
        ApplyChoices(specialists, choices); draftLoaded = true;
    }
    private void ApplyChoices(JsonObject[] specialists, List<ComposerOptionChoice> choices)
    {
        Reviewers = choices.ToArray(); Reviewer = ReviewerKey(specialists.FirstOrDefault(v => S(v, "id") == "reviewer"));
        Specialists = new[] { new ComposerOptionChoice("", "None") }.Concat(specialists
            .Where(v => S(v, "id") is not ("reviewer" or "reader" or "archivist" or "recap"))
            .Select(v => new ComposerOptionChoice(S(v, "id"), S(v, "name")))).ToArray();
    }
    public static string ReviewerKey(JsonObject? spec) => S(spec?["review_backend"], "kind") switch
    {
        "follow_session" => "follow_session", "acp_agent" => "acp:" + S(spec?["review_backend"], "profile_id"),
        "http_model" => "http:" + S(spec?["review_backend"], "profile_id"), _ => "http:" + S(spec, "model_id")
    };
    private async Task<bool> SaveAsync(Func<string, string?, int, Task> write, CancellationToken token, bool allowDraft = false)
    {
        if (client == null || !CanEdit || project == null || session == null && !allowDraft) return false;
        var current = generation; var p = project; var s = session;
        Busy = true; Error = null; saveError = true; Changed?.Invoke();
        try
        {
            await write(p, s, current);
            if (current != generation) return false;
            await ReadAsync(p, s, current, token);
            return current == generation;
        }
        catch (Exception ex) { if (current == generation) Error = ex.Message; return false; }
        finally { if (current == generation) { Busy = false; Changed?.Invoke(); } }
    }
    private Task<bool> SaveSessionAsync(JsonObject change, CancellationToken token) => SaveAsync(async (p, s, _) =>
    {
        var result = Decode<ComposerSessionOptions>(await client!.InvokeAsync("native_conversation_options_set",
            new() { ["session_id"] = s, ["change"] = change }, p, token));
        if (result.SessionId != s) throw new InvalidDataException("Conversation options belong to a different session.");
    }, token);
    public Task<bool> SetToggleAsync(string kind, bool enabled, bool confirmed = false, CancellationToken token = default)
    {
        if (kind is not ("full_permission" or "delegation" or "auto_review") || kind == "full_permission" && enabled && !confirmed) return Task.FromResult(false);
        var change = new JsonObject { ["kind"] = kind, ["enabled"] = enabled };
        if (kind == "full_permission") change["confirmed"] = confirmed;
        return SaveSessionAsync(change, token);
    }
    public Task<bool> SetSpecialistAsync(string id, CancellationToken token = default) =>
        Session?.SpecialistLocked != false || !Specialists.Any(v => v.Id == id) ? Task.FromResult(false)
        : SaveSessionAsync(new() { ["kind"] = "specialist", ["id"] = id }, token);
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
        }, token, allowDraft: true);
    private bool KnownRemote(string id) => SshAliases.Any(alias => "ssh:" + alias == id) || Contexts?.Contexts.Any(c => c.Id == id && c.Kind == "wsl") == true;
    public Task<bool> SetContextAsync(string id, bool enabled, CancellationToken token = default) => !CanEditCompute || !KnownRemote(id) ? Task.FromResult(false) : SaveAsync(async (p, s, _) =>
    {
        var ids = await new NativePanelClient(client!).SetContextEnabledAsync(p, s!, id, enabled, token);
        if (ids.Contains(id) != enabled) throw new InvalidDataException("计算环境未确认。");
    }, token);
    public Task<bool> SetDefaultContextAsync(string id, CancellationToken token = default) => !CanEditCompute
        || id != "local" && Contexts?.Contexts.Any(c => c.Id == id && c.Kind is "ssh" or "wsl") != true
        ? Task.FromResult(false) : SaveAsync(async (p, s, current) =>
        {
            var panel = new NativePanelClient(client!);
            await panel.SetDefaultContextAsync(p, s!, id, token);
            // WebView also attaches a newly chosen remote default. Do not continue
            // this two-step action after navigation, and never replay an uncertain write.
            if (id != "local" && current == generation && Contexts?.EnabledIds.Contains(id) != true)
            {
                var ids = await panel.SetContextEnabledAsync(p, s!, id, true, token);
                if (!ids.Contains(id)) throw new InvalidDataException("The default environment was saved, but attachment was not confirmed.");
            }
        }, token);
}
