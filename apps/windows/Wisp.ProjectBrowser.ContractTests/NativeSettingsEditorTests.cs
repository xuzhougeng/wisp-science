using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeSettingsEditorTests
{
    private static void Check(bool value, string description)
    { if (!value) throw new InvalidOperationException(description); Console.WriteLine("PASS settings editor: " + description); }
    public static async Task RunAsync()
    {
        var fake = new Fake();
        using var model = new NativeSettingsEditorModel(fake, "project-a");
        model.Edit(new() { ["src_path"] = "C:/skills/example" }, "install_skill");
        await model.SaveAsync();
        Check(fake.Args!["srcPath"]?.GetValue<string>() == "C:/skills/example" && !fake.Args.ContainsKey("src_path"), "flat fields use Tauri argument casing");
        var source = JsonNode.Parse("{\"id\":\"connection-1\",\"name\":\"Original\",\"future\":7,\"transport\":{\"kind\":\"http\",\"auth\":\"none\",\"headers\":[{\"name\":\"Authorization\",\"value\":\"\",\"secret_ref\":\"existing-key\"}]}}")!.AsObject();
        model.Edit(source, "update_mcp_connection", "conn"); model.Draft!["name"] = "Edited";
        Check(S(source, "name") == "Original", "editing never mutates the loaded snapshot");
        fake.Fail = true; var count = fake.Calls;
        Check(!await model.SaveAsync() && fake.Calls == count + 1 && model.HasChanges && model.Draft != null && !model.Busy,
            "failed write preserves draft without automatic replay");
        fake.Fail = false; fake.Pending = new();
        var save = model.SaveAsync(); count = fake.Calls;
        Check(model.Saving && !model.SaveConfirmed, "pending write is not presented as saved");
        Check(!await model.SaveAsync() && fake.Calls == count, "duplicate save is rejected while response is pending");
        fake.Pending.SetResult(null); await save; fake.Pending = null;
        Check(model.SaveConfirmed && !model.Saving, "only an authoritative write response confirms saving");
        Check(model.Draft == null && !model.HasChanges && fake.Project == "project-a", "void success closes the editor under its explicit project scope");
        Check(fake.Args!["conn"]?["future"]?.GetValue<int>() == 7 && S(fake.Args["conn"]?["transport"]?["headers"]?[0], "secret_ref") == "existing-key",
            "nested unknown fields and existing credential references survive edits");
        fake.Fail = true;
        Check(!await model.LoadAsync("list_mcp_connections") && model.SaveConfirmed && model.Error != null,
            "a refresh failure after confirmed save must not suggest replaying the write");
        fake.Fail = false;
        source["transport"]!["auth"] = "oauth";
        model.Edit(source, "update_mcp_connection", "conn"); await model.SaveAsync();
        Check(fake.Command == "authorize_http_connection", "OAuth saves use the authorizing host command");
        model.Edit(source, "update_mcp_connection", "conn"); fake.Pending = new();
        Check(!model.SaveConfirmed, "new edit clears previous saved feedback");
        var authorize = model.SaveAsync();
        Check(model.OAuthPending && model.Busy, "OAuth authorization exposes a cancellable pending operation");
        await model.CancelOAuthAsync();
        Check(fake.Command == "cancel_oauth_authorization" && fake.Project == "project-a", "OAuth cancellation can reach host while save is pending");
        fake.Pending.SetException(new IOException("authorization cancelled")); await authorize; fake.Pending = null;
        Check(!model.OAuthPending && !model.Busy && model.Draft != null, "cancelled OAuth retains the connection draft without resubmitting");
        Check(!model.Saving && !model.SaveConfirmed, "failed authorization never confirms saving");
        model.Edit(new() { ["src_path"] = "C:/fixtures/plugin", ["expected_sha256"] = "" }, "install_plugin"); await model.SaveAsync();
        Check(fake.Args!["expectedSha256"] == null && S(fake.Args, "srcPath") == "C:/fixtures/plugin", "optional local plugin checksum becomes null without changing the selected source");
        model.Edit(JsonNode.Parse("{\"locale\":\"zh\",\"api_url\":\"http://localhost\",\"model\":\"fixture\",\"future_flag\":true}")!.AsObject(), "set_settings", "settings");
        model.Draft!["locale"] = "en"; await model.SaveAsync();
        Check(S(fake.Args!["settings"], "locale") == "en" && S(fake.Args["settings"], "model") == "fixture" && fake.Args["settings"]!["future_flag"]!.GetValue<bool>(),
            "editing one general preference preserves model configuration and unknown options");
        model.Edit(new() { ["model_proxy_url"] = "none", ["subscription_proxy_url"] = "", ["mcp_proxy_url"] = "none" }, "set_network_settings", "settings");
        model.Draft!["subscription_proxy_url"] = "http://localhost:7897"; await model.SaveAsync();
        Check(S(fake.Args!["settings"], "subscription_proxy_url") == "http://localhost:7897"
            && S(fake.Args["settings"], "model_proxy_url") == "none" && S(fake.Args["settings"], "mcp_proxy_url") == "none",
            "subscription sign-in proxy saves without replacing model or MCP routing");
        model.Edit(new() { ["name"] = "analysis", ["env_var"] = "ANALYSIS_KEY", ["value"] = "synthetic-secret" }, "add_custom_credential"); await model.SaveAsync();
        Check(S(fake.Args, "envVar") == "ANALYSIS_KEY" && S(fake.Args, "value") == "synthetic-secret" && model.Draft == null,
            "credential write uses the keyring host command and clears local draft after success");
        model.Edit(new() { ["content"] = "old" }, "update_global_memory", arguments: new() { ["id"] = "memory-id" });
        model.Draft!["content"] = "new"; await model.SaveAsync();
        Check(S(fake.Args, "id") == "memory-id" && S(fake.Args, "content") == "new", "memory edits preserve authoritative row identity");
        var action = NativeWorkflowDrafts.QuickAction(4);
        var templates = new[] { new JsonObject { ["id"] = "template-1", ["name"] = "Evidence review" } };
        var newAction = NativeQuickActions.Create([action, new JsonObject { ["sort_order"] = 35 }], templates);
        Check(S(newAction, "workflow_template_id") == "template-1" && S(newAction, "id") == "" && S(newAction, "icon") == "sparkles"
            && newAction["sort_order"]!.GetValue<long>() == 45, "new selection actions bind the first workflow and append after the largest sort order");
        Check(NativeQuickActions.WorkflowName(newAction, templates) == "Evidence review" && NativeQuickActions.WorkflowName(newAction, []) == "template-1",
            "list rows show the bound workflow name and retain unavailable reference IDs");
        count = fake.Calls; model.Edit(newAction, "save_quick_action", "action");
        Check(!await model.SaveAsync() && fake.Calls == count && model.Draft != null, "blank action names block host writes and retain the inline draft");
        model.Draft!["name"] = "Synthetic selection"; model.Draft["workflow_template_id"] = "";
        Check(!await model.SaveAsync() && fake.Calls == count, "missing workflow bindings cannot be saved");
        Check(NativeQuickActions.Create([], [])["workflow_template_id"]!.GetValue<string>() == "", "an empty workflow catalog does not invent a binding");
        model.Discard();
        Check(model.Draft == null && !model.HasChanges && fake.Calls == count,
            "explicit Cancel discards a quick-action draft without saving or changing its source");
        action["name"] = "Inspect selection"; action["workflow_template_id"] = "template-1";
        model.Edit(action, "save_quick_action", "action"); model.Draft!["enabled"] = false; await model.SaveAsync();
        Check(action["enabled"]!.GetValue<bool>() && !fake.Args!["action"]!["enabled"]!.GetValue<bool>(),
            "clearing Show for selected text saves the draft flag without modifying the input record");
        Check(S(fake.Args!["action"], "context") == "selection" && S(fake.Args["action"], "workflow_template_id") == "template-1"
            && fake.Args["action"]!["sort_order"]!.GetValue<int>() == 4, "quick action save binds the selected template with selection context and ordering");
        var specialist = NativeWorkflowDrafts.Specialist();
        model.Edit(specialist, "save_specialist_cmd", "spec"); count = fake.Calls;
        Check(!await model.SaveAsync() && fake.Calls == count && model.Draft != null, "blank specialist names cannot reach the host and keep the editor draft");
        model.Draft!["name"] = "Cancelled specialist"; model.Discard();
        Check(model.Draft == null && !model.HasChanges && fake.Calls == count && S(specialist, "name") == "",
            "explicit specialist Cancel drops edits without touching the host or loaded source");
        specialist["name"] = "Synthetic specialist";
        Check(specialist["skills"] == null && specialist["connectors"] == null, "new specialists inherit project capabilities instead of disabling all capabilities");
        NativeWorkflowDrafts.SetWhitelist(specialist, "skills", false, "  analysis\nanalysis\nplot\n");
        Check(specialist["skills"]!.AsArray().Count == 2 && specialist["connectors"] == null, "explicit whitelist normalizes names without changing the other inherited capability set");
        NativeWorkflowDrafts.SetWhitelist(specialist, "skills", false, "");
        Check(specialist["skills"] is JsonArray { Count: 0 }, "an empty explicit whitelist disables all skills");
        NativeWorkflowDrafts.SetWhitelist(specialist, "skills", true, "analysis");
        model.Edit(specialist, "save_specialist_cmd", "spec"); await model.SaveAsync();
        Check(fake.Command == "save_specialist_cmd" && fake.Args!["spec"]!["skills"] == null, "restoring inheritance sends null rather than an empty capability whitelist");
        var reviewer = JsonNode.Parse("{\"id\":\"reviewer\",\"name\":\"Reviewer\",\"builtin\":true,\"review_backend\":{\"kind\":\"acp_agent\",\"profile_id\":\"agent-a\"},\"skills\":[\"review\"],\"future\":true}")!.AsObject();
        var copied = NativeWorkflowDrafts.Copy(reviewer); copied["skills"]!.AsArray().Add("extra");
        Check(S(copied, "id") != "reviewer" && !copied["builtin"]!.GetValue<bool>() && copied["review_backend"] == null
            && reviewer["skills"]!.AsArray().Count == 1 && copied["future"]!.GetValue<bool>(), "copying a reviewer creates an independent editable persona without reviewer-only backend state");
        model.Edit(reviewer, "save_specialist_cmd", "spec"); model.Draft!["description"] = "Updated"; await model.SaveAsync();
        Check(S(fake.Args!["spec"]?["review_backend"], "profile_id") == "agent-a", "editing reviewer text preserves its ACP backend binding");
        var legacyReviewer = new JsonObject { ["id"] = "reviewer", ["name"] = "Reviewer", ["model_id"] = "legacy-model", ["builtin"] = true, ["instructions"] = "Built-in prompt", ["connectors"] = new JsonArray("existing-connector"), ["future"] = 7 };
        var backendDraft = (JsonObject)legacyReviewer.DeepClone();
        var backendModels = new[] { new JsonObject { ["id"] = "chat", ["label"] = "Chat model", ["model"] = "chat" }, new JsonObject { ["id"] = "image", ["model"] = "gpt-image-2" } };
        var backendOptions = NativeSpecialists.BackendOptions(backendDraft, backendModels, [new JsonObject { ["id"] = "agent-a", ["label"] = "Agent A" }]);
        Check(backendOptions.Any(o => o.Id == "http:" && o.Available) && backendOptions.Any(o => o.Id == "follow_session")
            && backendOptions.Any(o => o.Id == "acp:agent-a") && !backendOptions.Any(o => o.Id == "http:image")
            && backendOptions.Any(o => o.Id == "http:legacy-model" && !o.Available), "reviewer choices filter non-chat models and preserve missing legacy references visibly");
        NativeSpecialists.SelectBackend(backendDraft, "acp:agent-a", legacyReviewer);
        Check(NativeSpecialists.BackendKey(backendDraft) == "acp:agent-a" && S(backendDraft, "model_id") == "legacy-model", "ACP binding keeps the legacy HTTP model field");
        NativeSpecialists.SelectBackend(backendDraft, "follow_session", legacyReviewer);
        Check(S(backendDraft["review_backend"], "kind") == "follow_session" && backendDraft["review_backend"]?["profile_id"] == null, "follow-session binding does not carry a stale profile ID");
        NativeSpecialists.SelectBackend(backendDraft, "http:chat", legacyReviewer);
        Check(S(backendDraft, "model_id") == "chat" && S(backendDraft["review_backend"], "profile_id") == "chat", "HTTP reviewer selection updates both explicit and legacy bindings");
        NativeSpecialists.SelectBackend(backendDraft, "http:legacy-model", legacyReviewer);
        Check(JsonNode.DeepEquals(backendDraft, legacyReviewer), "restoring the initial backend exactly restores the legacy shape without a false dirty warning");
        model.Edit(legacyReviewer, "save_specialist_cmd", "spec"); model.Draft!["instructions"] = "Unexpected edit"; model.Draft["name"] = "Renamed";
        await model.SaveAsync();
        Check(S(fake.Args!["spec"], "instructions") == "Built-in prompt" && S(fake.Args["spec"], "name") == "Renamed"
            && fake.Args["spec"]?["connectors"]?[0]?.GetValue<string>() == "existing-connector" && fake.Args["spec"]?["future"]?.GetValue<int>() == 7,
            "saving built-in metadata protects its instructions and preserves hidden connector and future fields");
        var skillCatalog = new[] { new JsonObject { ["name"] = "analysis", ["description"] = "Analyze observations", ["scope"] = "project", ["tags"] = new JsonArray("Evidence") } };
        Check(NativeSpecialists.SearchSkills(skillCatalog, "").Count == 0 && NativeSpecialists.SearchSkills(skillCatalog, " EVIDENCE ").Count == 1
            && NativeSpecialists.SearchSkills(skillCatalog, "project").Count == 1 && NativeSpecialists.SearchSkills(skillCatalog, "absent").Count == 0, "skill search is case-insensitive, includes tags and keeps the empty search compact");
        var skillDraft = new JsonObject { ["skills"] = new JsonArray("unavailable-skill"), ["connectors"] = null };
        NativeSpecialists.SetSkill(skillDraft, "analysis", true); NativeSpecialists.SetSkill(skillDraft, "analysis", true);
        Check(skillDraft["skills"]!.AsArray().Count == 2 && skillDraft["skills"]![0]!.GetValue<string>() == "unavailable-skill", "selecting a skill is unique and preserves unavailable saved skill names");
        NativeSpecialists.SetSkill(skillDraft, "analysis", false); NativeSpecialists.SetSkill(skillDraft, "unavailable-skill", false);
        Check(skillDraft["skills"] is JsonArray { Count: 0 } && skillDraft["connectors"] == null, "removing the final skill preserves explicit empty versus inherited semantics");
        skillDraft["skills"] = null; NativeSpecialists.SetSkill(skillDraft, "analysis", true);
        Check(skillDraft["skills"] == null, "a stale selection event cannot change inherited skill settings");
        var skillSource = new JsonObject { ["id"] = "custom-skills", ["name"] = "Skill draft", ["skills"] = new JsonArray("missing-from-catalog"), ["future"] = "preserve" };
        model.Edit(skillSource, "save_specialist_cmd", "spec"); count = fake.Calls;
        NativeSpecialists.SetSkill(model.Draft!, "analysis", true);
        NativeSpecialists.SetSkill(model.Draft!, "missing-from-catalog", false);
        Check(NativeSpecialists.SearchSkills(skillCatalog, "evidence").Count == 1
            && model.Draft!["skills"] is JsonArray { Count: 1 } && model.Draft["skills"]![0]!.GetValue<string>() == "analysis"
            && fake.Calls == count && skillSource["skills"]![0]!.GetValue<string>() == "missing-from-catalog",
            "selection chips and search results edit only the local draft and retain the saved source until explicit Save");
        model.Discard();
        Check(model.Draft == null && fake.Calls == count && S(skillSource, "future") == "preserve",
            "canceling a skill-picker draft never writes or changes unrelated specialist metadata");
        var workflow = NativeWorkflowDrafts.Workflow(); var tasks = workflow["proposal"]!["tasks"]!.AsArray();
        tasks.Add(NativeWorkflowDrafts.TaskNode([])); tasks.Add(NativeWorkflowDrafts.TaskNode(tasks.OfType<JsonObject>()));
        Check(S(tasks[0], "id") != S(tasks[1], "id") && tasks[0]!["executor"] == null && tasks[0]!["budget"] == null,
            "new workflow tasks get unique identities and inherit executor and budget policy");
        model.Edit(workflow, "save_workflow_template", "template", new() { ["conversionSourceSha256"] = "source-hash" });
        fake.Fail = true; count = fake.Calls;
        Check(!await model.SaveAsync() && fake.Calls == count + 1 && model.Draft?["proposal"]?["tasks"]?.AsArray().Count == 2,
            "host workflow validation errors retain the complete task graph without replaying saves");
        fake.Fail = false; await model.SaveAsync();
        Check(S(fake.Args, "conversionSourceSha256") == "source-hash" && fake.Args!["template"]?["proposal"]?["approval_policy"]?.GetValue<string>() == "review_all",
            "reviewed conversion save preserves source provenance and approval policy outside the template payload");
        model.Edit(new() { ["name"] = "Renamed", ["description"] = "Research", ["agent_context"] = "Keep project instructions" }, "update_project", arguments: new() { ["id"] = "project-a" });
        await model.SaveAsync();
        Check(fake.Command == "update_project" && fake.Project == "project-a" && S(fake.Args, "id") == "project-a"
            && S(fake.Args, "agentContext") == "Keep project instructions", "project settings retain explicit identity and camelCase context without switching project scope");
        model.Edit(new() { ["run_retention_days"] = null, ["failed_run_retention_days"] = 30, ["orphan_file_retention_days"] = 7 }, "set_project_run_retention");
        count = fake.Calls; model.Discard();
        Check(fake.Calls == count, "opening and dismissing relocated retention settings performs no write");
        model.Edit(new() { ["run_retention_days"] = null, ["failed_run_retention_days"] = 30, ["orphan_file_retention_days"] = 7 }, "set_project_run_retention");
        await model.SaveAsync();
        Check(fake.Command == "set_project_run_retention" && fake.Project == "project-a" && fake.Args!.ContainsKey("runRetentionDays")
            && fake.Args["runRetentionDays"] == null && fake.Args["failedRunRetentionDays"]!.GetValue<int>() == 30
            && fake.Args["orphanFileRetentionDays"]!.GetValue<int>() == 7,
            "retention keeps explicit project identity, null/off values and existing camelCase arguments after moving to project settings");
        model.Edit(new() { ["content"] = "draft" }, "create_global_memory");
        await model.LoadAsync("get_memory_view");
        Check(S(model.Draft, "content") == "draft", "reads do not replace an editor draft");
        model.Discard(); Check(model.Draft == null, "discard clears only the local draft");
        fake.Pending = new(); var read = model.LoadAsync("get_memory_view");
        model.Dispose(); fake.Pending.SetResult(new JsonObject { ["files"] = new JsonArray() }); await read;
        Check(model.Values.Count == 0 && model.Draft == null, "late response cannot repopulate a closed settings view");
        var offline = new Fake { Fail = true };
        using var failedLoad = new NativeSettingsEditorModel(offline, "other")
        { Localization = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>(), "en") };
        Check(!await failedLoad.LoadAsync("list_skills") && failedLoad.LoadFailed && failedLoad.Values.Count == 0
            && failedLoad.Error?.StartsWith("Could not load settings.") == true && offline.Calls == 1,
            "failed reads show a localized read error without an invented snapshot or an automatic retry");
        failedLoad.Edit(new() { ["name"] = "original" }, "unused_write");
        failedLoad.Draft!["name"] = "unsaved";
        var probe = new TaskCompletionSource();
        var retry = failedLoad.RetryLoadAsync(["list_skills"], () => probe.Task);
        Check(!await failedLoad.RetryLoadAsync(["list_skills"], () => throw new Exception("duplicate probe"))
            && offline.Calls == 1, "repeat clicks do not reconnect or read while the first probe is pending");
        offline.Fail = false; probe.SetResult();
        Check(await retry && !failedLoad.LoadFailed && failedLoad.Error == null && offline.Calls == 2
            && offline.Command == "list_skills" && offline.Project == "other" && failedLoad.HasChanges && S(failedLoad.Draft, "name") == "unsaved",
            "recovery reloads the same scope, preserves the draft, and does not send it");
        offline.Fail = true; await failedLoad.SaveAsync();
        Check(failedLoad.Error?.StartsWith("The operation could not be confirmed") == true
            && !await failedLoad.RetryLoadAsync(["list_skills"], () => throw new Exception("unexpected probe")) && offline.Calls == 3,
            "a failed write does not expose read recovery or replay a mutation");
    }
    private static string S(JsonNode? value, string key) => value?[key]?.GetValue<string>() ?? "";
    private sealed class Fake : INativeSettingsClient
    {
        public bool Fail; public int Calls; public string? Command, Project; public JsonObject? Args;
        public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Calls++; Command = command; Project = projectId; Args = (JsonObject)arguments.DeepClone();
            if (command == "cancel_oauth_authorization") return Task.FromResult<JsonNode?>(null);
            return Fail ? Task.FromException<JsonNode?>(new IOException("lost response")) : Pending?.Task ?? Task.FromResult<JsonNode?>(null);
        }
    }
}
