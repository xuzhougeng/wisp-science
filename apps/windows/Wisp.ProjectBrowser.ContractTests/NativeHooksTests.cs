using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeHooksTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var multiline = "  first\targument\r\n  second  \nthird\fline  ";
        Check(NativeHooksModel.CommandSummary(multiline) == "first argument second third line" && multiline.Contains('\n'),
            "list previews collapse CSS whitespace without changing the command's editable source");
        Check(NativeHooksModel.CommandSummary("one\u00a0two") == "one\u00a0two" && NativeHooksModel.CommandSummary(" \t\r\n") == "",
            "preview whitespace follows CSS collapse without consuming nonbreaking spaces");
        var fake = new Fake(); using var model = new NativeHooksModel(fake, "project-a");
        Check(await model.LoadAsync() && model.Loaded && model.Hooks.Count == 2 && model.ReviewerLabel == "跟随会话后端", "hooks load global defaults and the configured reviewer binding");
        Check(fake.Calls.Single(c => c.Command == "get_project_hooks").Project == "project-a", "project hooks read through the settings page's owned project context");
        Check(fake.Calls.Where(c => c.Command != "get_project_hooks").All(c => c.Project == null && !c.Args.ContainsKey("sessionId")), "global hooks and auto-review defaults do not target the current conversation");
        Check(model.Edit() && model.Draft!["event"]!.GetValue<string>() == "PreToolUse" && !model.CanSave && !model.HasChanges, "new hooks start clean with a blank command and default tool event");
        var count = fake.Calls.Count;
        Check(!await model.LoadAsync() && fake.Calls.Count == count, "refresh cannot overwrite an open hook draft");
        model.SetDraftText("command", "first line\nsecond line"); model.SetDraftText("matcher", "shell|edit");
        foreach (var invalid in new[] { "0", "-1", "1.5", "abc", "18446744073709551616" })
        {
            model.SetTimeout(invalid); Check(!model.CanSave && !await model.SaveAsync(), "invalid timeout text cannot become an omitted/default timeout on save");
        }
        Check(fake.Calls.Count == count, "invalid timeout input causes no host requests");
        model.SetTimeout("  "); Check(model.ValidTimeout && model.CanSave, "blank timeouts select the host default");
        model.SetDraftText("event", "Stop"); Check(model.Draft!["matcher"]!.GetValue<string>() == "shell|edit", "changing events retains the hidden matcher draft until confirmation");
        fake.Fail = "set_command_hooks";
        Check(!await model.SaveAsync() && model.Draft != null && model.Hooks.Count == 2 && model.Draft["command"]!.GetValue<string>().Contains('\n'), "a failed write preserves the original list and multiline draft");
        Check(fake.Calls[^1].Args["hooks"]![2]!["matcher"]!.GetValue<string>() == "" && fake.Calls[^1].Args["hooks"]![2]!["timeout"] == null, "non-tool events submit no matcher and blank timeout uses the default");
        fake.Fail = null; fake.PendingCommand = "set_command_hooks"; fake.Pending = new();
        var saving = model.SaveAsync(); count = fake.Calls.Count;
        Check(model.Busy && !await model.SaveAsync() && fake.Calls.Count == count, "pending hook saves cannot be submitted twice");
        var confirmed = (JsonArray)fake.Calls[^1].Args["hooks"]!.DeepClone(); confirmed[2]!["command"] = "host-normalized";
        fake.Pending.SetResult(confirmed); await saving; fake.Pending = null;
        Check(model.Draft == null && model.Hooks.Count == 3 && model.Hooks[2]!["command"]!.GetValue<string>() == "host-normalized", "acknowledged host normalization is authoritative");
        Check(model.Edit(0), "existing hook opens by its list position"); model.SetTimeout("18446744073709551615"); model.SetDraftText("command", "changed\ncommand"); await model.SaveAsync();
        Check(fake.Calls[^1].Args["hooks"]![0]!["timeout"]!.GetValue<ulong>() == ulong.MaxValue && model.Hooks[0]!["future_field"]!.GetValue<string>() == "preserve", "editing preserves unsigned timeouts and unknown hook fields");
        Check(await model.SetHookEnabledAsync(1, false) && !model.Hooks[1]!["enabled"]!.GetValue<bool>(), "row switches save their full list and follow the acknowledgement");
        Check(await model.DeleteHookAsync(1) && model.Hooks.Count == 2 && model.Hooks[0]!["command"]!.GetValue<string>() == "changed\ncommand", "removal preserves the other command rows");
        model.Edit(); model.SetDraftText("command", "unsaved");
        Check(await model.SetAutoReviewAsync(true) && model.AutoReview && model.Draft!["command"]!.GetValue<string>() == "unsaved", "builtin updates do not discard custom command drafts");
        Check(!fake.Calls[^1].Args.ContainsKey("sessionId") && !fake.Calls[^1].Args.ContainsKey("session_id"), "auto-review changes only the new-session default");
        model.SetDraftText("command", ""); Check(!model.HasChanges, "restoring original text makes the new editor clean again"); model.Discard();
        count = fake.Calls.Count; Check(!await model.SetFailureAsync("minimum_failures", 0) && !await model.SetFailureAsync("failure_rate_threshold", 101) && fake.Calls.Count == count, "failure-analysis numeric limits are validated before sending");
        Check(await model.SetFailureAsync("minimum_failures", 7) && model.FailureAnalysis["minimum_failures"]!.GetValue<int>() == 7 && model.FailureAnalysis["future_field"]!.GetValue<string>() == "keep", "failure-analysis updates retain unrelated values");
        fake.Fail = "set_auto_review_enabled";
        Check(!await model.SetAutoReviewAsync(false) && model.AutoReview, "failed toggle acknowledgement keeps the confirmed default"); fake.Fail = null;
        count = fake.Calls.Count; Check(!await model.SetProjectTrustAsync("unseen") && fake.Calls.Count == count, "only the displayed project content hash can be trusted");
        fake.Fail = "set_project_hooks_trust"; Check(!await model.SetProjectTrustAsync("reviewed-hash") && !model.ProjectFile!["trusted"]!.GetValue<bool>(), "changed-file or failed trust acknowledgements never mark a file trusted"); fake.Fail = null;
        Check(await model.SetProjectTrustAsync("reviewed-hash") && model.ProjectFile!["trusted"]!.GetValue<bool>() && fake.Calls[^1].Project == "project-a", "trust uses the displayed hash in the owned project context");
        Check(await model.SetProjectTrustAsync(null) && fake.Calls[^1].Args.ContainsKey("sha256") && fake.Calls[^1].Args["sha256"] == null, "revoke explicitly submits null");
        var homeFake = new Fake(); using var home = new NativeHooksModel(homeFake, null);
        Check(await home.LoadAsync() && home.ProjectId == "restored" && home.ProjectFile != null
            && homeFake.Calls.Single(c => c.Command == "get_project_hooks").Project == "restored",
            "Home reads project hooks from the restored workspace alongside global defaults");
        homeFake.ActiveProject = "another-workspace";
        Check(await home.LoadAsync() && await home.SetProjectTrustAsync("reviewed-hash") && homeFake.Calls[^1].Project == "restored"
            && homeFake.Calls.Count(c => c.Command == "get_project_info") == 1,
            "later project reads and reviewed-hash acknowledgements retain the original owned scope");
        foreach (var identity in new JsonNode?[] { null, JsonValue.Create(""), JsonValue.Create("  "), JsonValue.Create(42) })
        {
            var missing = new Fake { ActiveProject = identity }; using var unavailable = new NativeHooksModel(missing, null);
            Check(await unavailable.LoadAsync() && unavailable.Loaded && unavailable.Hooks.Count == 2 && unavailable.ProjectFile == null
                && unavailable.ProjectError != null && !await unavailable.SetProjectTrustAsync("reviewed-hash")
                && missing.Calls.All(c => c.Command != "get_project_hooks" && c.Command != "set_project_hooks_trust"),
                "invalid restored identity leaves global defaults usable without inventing a project target");
        }
        var broken = new Fake { Fail = "get_project_hooks" }; using var partial = new NativeHooksModel(broken, "a");
        Check(await partial.LoadAsync() && partial.Loaded && partial.ProjectFile == null && partial.ProjectError != null, "project-file read errors do not hide working global settings");
        var delayed = new Fake { PendingCommand = "get_command_hooks", Pending = new() }; var closed = new NativeHooksModel(delayed, null); var loading = closed.LoadAsync(); closed.Dispose(); delayed.Pending.SetResult(new JsonArray()); await loading;
        Check(!closed.Loaded && closed.Hooks.Count == 0, "late responses cannot repopulate a disposed hook page");
        Check(NativeHooksModel.Events.Length == 5 && NativeHooksModel.Events.Count(NativeHooksModel.MatchesTools) == 3, "all supported events distinguish tool matchers from prompt/stop hooks");
        Check(NativeHooksModel.Events.Select(NativeHooksModel.EventHintKey).SequenceEqual(new[] {
            "hooks.event_hint.user_prompt_submit", "hooks.event_hint.pre_tool_use", "hooks.event_hint.post_tool_use",
            "hooks.event_hint.post_tool_use_failure", "hooks.event_hint.stop" }), "each event selects its distinct shared description");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>
        {
            ["en"] = new() { ["composer.reviewer.default_http"] = "Default HTTP model", ["composer.reviewer.follow_session"] = "Follow session backend" },
            ["zh"] = new() { ["composer.reviewer.default_http"] = "默认 HTTP 模型", ["composer.reviewer.follow_session"] = "跟随会话后端" }
        });
        using var localized = new NativeHooksModel(new Fake(), "owned", locale);
        Check(await localized.LoadAsync() && localized.ReviewerLabel == "Follow session backend", "the reviewer binding follows the page's locale");
        locale.Preview("zh"); Check(await localized.LoadAsync() && localized.ReviewerLabel == "跟随会话后端", "reloading in Chinese retains the same reviewer identity");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public string? Fail, PendingCommand;
        public JsonNode? ActiveProject = JsonValue.Create("restored");
        public TaskCompletionSource<JsonNode?>? Pending;
        public List<(string Command, JsonObject Args, string? Project)> Calls { get; } = [];
        private static JsonObject Hook(string command) => new() { ["event"] = "PreToolUse", ["matcher"] = "shell", ["command"] = command, ["enabled"] = true, ["future_field"] = "preserve" };
        private static JsonObject File(bool trusted = false) => new() { ["path"] = "C:/workspace/.wisp/hooks.json", ["sha256"] = "reviewed-hash", ["trusted"] = trusted, ["error"] = null, ["hooks"] = new JsonArray(Hook("project hook\nfull command")) };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (Fail == command) return Task.FromException<JsonNode?>(new IOException("lost acknowledgement or changed file"));
            if (PendingCommand == command && Pending != null) return Pending.Task;
            JsonNode? response = command switch
            {
                "get_auto_review_enabled" => JsonValue.Create(false),
                "set_auto_review_enabled" => arguments["enabled"]!.DeepClone(),
                "get_auto_failure_analysis_settings" => new JsonObject { ["enabled"] = false, ["failure_rate_threshold"] = 50, ["minimum_failures"] = 3, ["future_field"] = "keep" },
                "set_auto_failure_analysis_settings" => arguments["settings"]!.DeepClone(),
                "get_command_hooks" => new JsonArray(Hook("one"), Hook("two")),
                "set_command_hooks" => arguments["hooks"]!.DeepClone(),
                "list_specialists" => new JsonArray(new JsonObject { ["id"] = "reviewer", ["review_backend"] = new JsonObject { ["kind"] = "follow_session" } }),
                "list_models" or "list_acp_agents" => new JsonArray(),
                "get_project_hooks" => File(),
                "get_project_info" => new JsonObject { ["id"] = ActiveProject?.DeepClone() },
                "set_project_hooks_trust" => File(arguments["sha256"] != null),
                _ => throw new InvalidOperationException(command)
            };
            return Task.FromResult<JsonNode?>(response);
        }
    }
}
