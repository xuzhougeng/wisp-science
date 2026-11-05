using System.Text.Json.Nodes;

namespace Wisp.ProjectBrowser;

/// Specialist bindings share the WebView's HTTP/ACP keys and nullable skill inheritance.
public static class NativeSpecialists
{
    public static IReadOnlyList<SpecialistBackendOption> SubagentOptions(string current, IEnumerable<JsonObject> models, NativeLocalization? localization = null)
    {
        var options = new List<SpecialistBackendOption>
        {
            new("", localization?.Text("specialists.subagent_model.follow") ?? "Follow the conversation model")
        };
        options.AddRange(models.Where(NativeModelPresentation.IsChat)
            .Select(model => new SpecialistBackendOption(S(model, "id"), NativeModelPresentation.Label(model))));
        if (!options.Any(option => option.Id == current)) options.Add(new(current,
            (localization?.Choose("当前配置不可用：", "Current configuration unavailable: ") ?? "Current configuration unavailable: ") + current, false));
        return options;
    }

    public static string ReviewerResult(JsonObject report, NativeLocalization localization)
    {
        var backend = S(report, "backend") switch { "acp_agent" => "ACP", "http_model" => "HTTP", var other => other };
        var headline = localization.Format("specialists.reviewer.test_ok", ("backend", backend), ("model", S(report, "model")), ("status", S(report, "status")));
        var summary = S(report, "summary").Trim();
        return summary.Length == 0 ? headline : headline + " " + summary;
    }
    private static string S(JsonNode? node, string key) => node?[key]?.GetValue<string>() ?? "";
    public static string BackendKey(JsonObject draft)
    {
        if (S(draft, "id") != "reviewer") return S(draft, "model_id");
        var backend = draft["review_backend"];
        return S(backend, "kind") switch
        {
            "follow_session" => "follow_session",
            "acp_agent" => "acp:" + S(backend, "profile_id"),
            "http_model" => "http:" + S(backend, "profile_id"),
            _ => "http:" + S(draft, "model_id")
        };
    }
    public static IReadOnlyList<SpecialistBackendOption> BackendOptions(JsonObject draft, IEnumerable<JsonObject> models, IEnumerable<JsonObject> agents, NativeLocalization? localization = null)
    {
        var reviewer = S(draft, "id") == "reviewer";
        var options = new List<SpecialistBackendOption>();
        options.Add(new(reviewer ? "http:" : "", reviewer
            ? localization?.Text("composer.reviewer.default_http") ?? "默认 HTTP 模型"
            : localization?.Text("specialists.model.follow") ?? "跟随默认模型"));
        if (reviewer) options.Add(new("follow_session", localization?.Text("composer.reviewer.follow_session") ?? "跟随会话后端"));
        options.AddRange(models.Where(NativeModelPresentation.IsChat).Select(m => new SpecialistBackendOption((reviewer ? "http:" : "") + S(m, "id"), NativeModelPresentation.Label(m))));
        if (reviewer) options.AddRange(agents.Select(a => new SpecialistBackendOption("acp:" + S(a, "id"), S(a, "label") + " · ACP")));
        var current = BackendKey(draft);
        if (!options.Any(o => o.Id == current)) options.Add(new(current,
            (localization?.Choose("当前配置不可用：", "Current configuration unavailable: ") ?? "当前配置不可用：") + current, false));
        return options;
    }
    public static void SelectBackend(JsonObject draft, string key, JsonObject original)
    {
        // Returning to the initial choice must not turn a legacy/null binding into a dirty draft.
        if (key == BackendKey(original))
        {
            foreach (var field in new[] { "model_id", "review_backend" })
                if (original.ContainsKey(field)) draft[field] = original[field]?.DeepClone(); else draft.Remove(field);
            return;
        }
        if (S(draft, "id") != "reviewer") { draft["model_id"] = key; return; }
        if (key == "follow_session") draft["review_backend"] = new JsonObject { ["kind"] = "follow_session" };
        else if (key.StartsWith("acp:", StringComparison.Ordinal))
            draft["review_backend"] = new JsonObject { ["kind"] = "acp_agent", ["profile_id"] = key[4..] };
        else
        {
            var id = key.StartsWith("http:", StringComparison.Ordinal) ? key[5..] : key;
            draft["model_id"] = id;
            draft["review_backend"] = new JsonObject { ["kind"] = "http_model", ["profile_id"] = id };
        }
    }
    public static string? SaveBlockReason(JsonObject draft)
        => string.IsNullOrWhiteSpace(S(draft, "name")) ? "请输入专家名称。" : null;
    public static JsonObject SaveDraft(JsonObject draft, JsonObject original)
    {
        var saved = (JsonObject)draft.DeepClone();
        if (original["builtin"]?.GetValue<bool>() == true)
        {
            if (original.ContainsKey("instructions")) saved["instructions"] = original["instructions"]?.DeepClone();
            else saved.Remove("instructions");
        }
        return saved;
    }
    public static IReadOnlyList<JsonObject> SearchSkills(IEnumerable<JsonObject> skills, string query)
    {
        var term = query.Trim();
        if (term.Length == 0) return [];
        return skills.Where(s => new[] { S(s, "name"), S(s, "description"), S(s, "scope") }
            .Concat((s["tags"] as JsonArray ?? []).Select(t => t?.GetValue<string>() ?? ""))
            .Any(value => value.Contains(term, StringComparison.OrdinalIgnoreCase))).ToArray();
    }
    public static void SetSkill(JsonObject draft, string name, bool selected)
    {
        if (draft["skills"] is not JsonArray skills) return;
        var names = skills.Select(n => n?.GetValue<string>() ?? "").ToList();
        if (selected && !names.Contains(name, StringComparer.Ordinal)) names.Add(name);
        if (!selected) names.RemoveAll(n => n == name);
        draft["skills"] = new JsonArray(names.Select(n => (JsonNode?)JsonValue.Create(n)).ToArray());
    }
}

public sealed record SpecialistBackendOption(string Id, string Label, bool Available = true);
