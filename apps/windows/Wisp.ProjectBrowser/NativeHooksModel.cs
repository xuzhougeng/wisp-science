using System.Globalization;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Lifecycle hook configuration; commands are saved, never executed by this editor.
public sealed class NativeHooksModel(INativeSettingsClient client, string? projectId, NativeLocalization? localization = null) : WorkspaceActionModel
{
    public string? ProjectId { get; private set; } = projectId;
    public static readonly string[] Events = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure", "Stop"];
    public static bool MatchesTools(string value) => value is "PreToolUse" or "PostToolUse" or "PostToolUseFailure";
    // Match the list's CSS white-space: nowrap. The editable command and the
    // project trust review still retain the full original text.
    public static string CommandSummary(string command)
        => System.Text.RegularExpressions.Regex.Replace(command, "[ \\t\\r\\n\\f]+", " ").Trim(' ');
    public static string EventHintKey(string value) => "hooks.event_hint." + (value switch
    {
        "UserPromptSubmit" => "user_prompt_submit", "PreToolUse" => "pre_tool_use", "PostToolUse" => "post_tool_use",
        "PostToolUseFailure" => "post_tool_use_failure", "Stop" => "stop", _ => throw new ArgumentOutOfRangeException(nameof(value))
    });
    public bool Loaded { get; private set; }
    public bool AutoReview { get; private set; }
    public JsonObject FailureAnalysis { get; private set; } = new();
    public JsonArray Hooks { get; private set; } = new();
    public JsonObject? ProjectFile { get; private set; }
    public string? ProjectError { get; private set; }
    public string ReviewerLabel { get; private set; } = localization?.Text("composer.reviewer.default_http") ?? "默认 HTTP 模型";
    public JsonObject? Draft { get; private set; }
    public int? EditingIndex { get; private set; }
    public string TimeoutText { get; private set; } = "";
    private JsonObject? original;
    private string originalTimeout = "";
    public bool HasChanges => Draft != null && (!JsonNode.DeepEquals(Draft, original) || TimeoutText != originalTimeout);
    public bool ValidTimeout => string.IsNullOrWhiteSpace(TimeoutText) || ulong.TryParse(TimeoutText.Trim(), NumberStyles.None, CultureInfo.InvariantCulture, out var value) && value > 0;
    public bool CanSave => Loaded && !Busy && !Closed && Draft != null && Events.Contains(S(Draft, "event")) && !string.IsNullOrWhiteSpace(S(Draft, "command")) && ValidTimeout;
    private static string S(JsonNode? node, string key) => node?[key]?.GetValue<string>() ?? "";
    private static JsonArray Array(JsonNode? value) => value is JsonArray rows ? (JsonArray)rows.DeepClone() : throw new InvalidDataException("钩子列表响应无效。");
    private static bool Boolean(JsonNode? value) => value?.GetValue<bool>() ?? throw new InvalidDataException("钩子开关响应无效。");
    private static JsonObject Failure(JsonNode? value)
    {
        if (value is not JsonObject settings || settings["enabled"] is not JsonValue flag || !flag.TryGetValue<bool>(out _)
            || settings["failure_rate_threshold"]?.GetValue<int>() is not (>= 1 and <= 100)
            || settings["minimum_failures"]?.GetValue<int>() is not (>= 1 and <= 100)) throw new InvalidDataException("失败分析设置响应无效。");
        return (JsonObject)settings.DeepClone();
    }
    private static JsonObject? Project(JsonNode? value)
    {
        if (value == null) return null;
        if (value is not JsonObject file || file["hooks"] is not JsonArray || S(file, "sha256").Length == 0 || file["trusted"] is not JsonValue flag || !flag.TryGetValue<bool>(out _))
            throw new InvalidDataException("项目钩子响应无效。");
        return (JsonObject)file.DeepClone();
    }
    public Task<bool> LoadAsync()
    {
        if (Draft != null) return Task.FromResult(false);
        return RunAsync(async () =>
        {
            var review = Boolean(await client.InvokeAsync("get_auto_review_enabled", new()));
            var failure = Failure(await client.InvokeAsync("get_auto_failure_analysis_settings", new()));
            var hooks = Array(await client.InvokeAsync("get_command_hooks", new()));
            var specialists = Array(await client.InvokeAsync("list_specialists", new()));
            var models = Array(await client.InvokeAsync("list_models", new()));
            var agents = Array(await client.InvokeAsync("list_acp_agents", new()));
            var reviewer = specialists.OfType<JsonObject>().FirstOrDefault(s => S(s, "id") == "reviewer");
            var label = reviewer == null ? localization?.Text("composer.reviewer.default_http") ?? "默认 HTTP 模型" : NativeSpecialists.BackendOptions(reviewer, models.OfType<JsonObject>(), agents.OfType<JsonObject>(), localization).First(o => o.Id == NativeSpecialists.BackendKey(reviewer)).Label;
            JsonObject? project = null; string? projectError = null; var scope = ProjectId;
            try
            {
                scope = await NativeSettingsWorkspace.ResolveAsync(client, scope);
                project = Project(await client.InvokeAsync("get_project_hooks", new(), scope));
            }
            catch (Exception ex) { projectError = (localization?.Choose("项目钩子读取失败：", "Could not read project hooks: ") ?? "项目钩子读取失败：") + ex.Message; }
            return (review, failure, hooks, label, scope, project, projectError);
        }, result =>
        {
            AutoReview = result.review; FailureAnalysis = result.failure; Hooks = result.hooks; ReviewerLabel = result.label;
            ProjectId = result.scope; ProjectFile = result.project; ProjectError = result.projectError; Loaded = true;
        });
    }
    public bool Edit(int? index = null)
    {
        if (Busy || Closed || !Loaded || Draft != null || index is { } i && (i < 0 || i >= Hooks.Count)) return false;
        Draft = index is { } row ? (JsonObject)Hooks[row]!.DeepClone() : new() { ["event"] = "PreToolUse", ["matcher"] = "", ["command"] = "", ["enabled"] = true };
        EditingIndex = index; original = (JsonObject)Draft.DeepClone(); TimeoutText = originalTimeout = Draft["timeout"]?.ToString() ?? "";
        ClearFailure(); Notify(); return true;
    }
    public void SetDraftText(string key, string value)
    {
        if (Busy || Closed || Draft == null || key is not ("event" or "matcher" or "command")) return;
        if (key == "event" && !Events.Contains(value)) return;
        Draft[key] = value; Notify();
    }
    public void SetTimeout(string text) { if (!Busy && !Closed && Draft != null) { TimeoutText = text; Notify(); } }
    public void Discard() { if (Busy) return; Draft = original = null; EditingIndex = null; TimeoutText = originalTimeout = ""; ClearFailure(); Notify(); }
    public Task<bool> SaveAsync()
    {
        if (!CanSave) return Task.FromResult(false);
        var draft = (JsonObject)Draft!.DeepClone();
        if (string.IsNullOrWhiteSpace(TimeoutText)) draft.Remove("timeout"); else draft["timeout"] = ulong.Parse(TimeoutText.Trim(), CultureInfo.InvariantCulture);
        if (!MatchesTools(S(draft, "event"))) draft["matcher"] = "";
        var next = (JsonArray)Hooks.DeepClone();
        if (EditingIndex is { } index) next[index] = draft; else next.Add(draft);
        return WriteHooks(next, true);
    }
    private Task<bool> WriteHooks(JsonArray next, bool closeDraft = false) => RunAsync(
        async () => Array(await client.InvokeAsync("set_command_hooks", new() { ["hooks"] = next })), rows =>
        {
            Hooks = rows;
            if (closeDraft) { Draft = original = null; EditingIndex = null; TimeoutText = originalTimeout = ""; }
        });
    public Task<bool> SetHookEnabledAsync(int index, bool enabled)
    {
        if (!Loaded || Draft != null || index < 0 || index >= Hooks.Count) return Task.FromResult(false);
        var next = (JsonArray)Hooks.DeepClone(); next[index]!["enabled"] = enabled; return WriteHooks(next);
    }
    public Task<bool> DeleteHookAsync(int index)
    {
        if (!Loaded || Draft != null || index < 0 || index >= Hooks.Count) return Task.FromResult(false);
        var next = (JsonArray)Hooks.DeepClone(); next.RemoveAt(index); return WriteHooks(next);
    }
    public Task<bool> SetAutoReviewAsync(bool enabled) => !Loaded ? Task.FromResult(false) : RunAsync(
        async () => Boolean(await client.InvokeAsync("set_auto_review_enabled", new() { ["enabled"] = enabled })), value => AutoReview = value);
    public Task<bool> SetFailureAsync(string field, int value)
    {
        if (!Loaded || field is not ("enabled" or "failure_rate_threshold" or "minimum_failures")
            || field != "enabled" && value is not (>= 1 and <= 100)) return Task.FromResult(false);
        var next = (JsonObject)FailureAnalysis.DeepClone();
        next[field] = field == "enabled" ? JsonValue.Create(value != 0) : JsonValue.Create(value);
        return RunAsync(async () => Failure(await client.InvokeAsync("set_auto_failure_analysis_settings", new() { ["settings"] = next })), saved => FailureAnalysis = saved);
    }
    public Task<bool> SetProjectTrustAsync(string? reviewedHash)
    {
        if (!Loaded || ProjectId == null || ProjectFile == null || reviewedHash != null &&
            (reviewedHash != S(ProjectFile, "sha256") || ProjectFile["error"] != null || ProjectFile["hooks"] is not JsonArray { Count: > 0 })) return Task.FromResult(false);
        return RunAsync(async () => Project(await client.InvokeAsync("set_project_hooks_trust", new() { ["sha256"] = reviewedHash }, ProjectId)), file => { ProjectFile = file; ProjectError = null; });
    }
    public override void Dispose() { base.Dispose(); Draft = original = ProjectFile = null; Hooks = new(); Loaded = false; }
}
