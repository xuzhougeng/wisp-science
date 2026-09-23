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
        Check(!await model.SaveAsync() && fake.Calls == count, "duplicate save is rejected while response is pending");
        fake.Pending.SetResult(null); await save; fake.Pending = null;
        Check(model.Draft == null && !model.HasChanges && fake.Project == "project-a", "void success closes the editor under its explicit project scope");
        Check(fake.Args!["conn"]?["future"]?.GetValue<int>() == 7 && S(fake.Args["conn"]?["transport"]?["headers"]?[0], "secret_ref") == "existing-key",
            "nested unknown fields and existing credential references survive edits");
        source["transport"]!["auth"] = "oauth";
        model.Edit(source, "update_mcp_connection", "conn"); await model.SaveAsync();
        Check(fake.Command == "authorize_http_connection", "OAuth saves use the authorizing host command");
        model.Edit(source, "update_mcp_connection", "conn"); fake.Pending = new();
        var authorize = model.SaveAsync();
        Check(model.OAuthPending && model.Busy, "OAuth authorization exposes a cancellable pending operation");
        await model.CancelOAuthAsync();
        Check(fake.Command == "cancel_oauth_authorization" && fake.Project == "project-a", "OAuth cancellation can reach host while save is pending");
        fake.Pending.SetException(new IOException("authorization cancelled")); await authorize; fake.Pending = null;
        Check(!model.OAuthPending && !model.Busy && model.Draft != null, "cancelled OAuth retains the connection draft without resubmitting");
        model.Edit(new() { ["src_path"] = "C:/fixtures/plugin", ["expected_sha256"] = "" }, "install_plugin"); await model.SaveAsync();
        Check(fake.Args!["expectedSha256"] == null && S(fake.Args, "srcPath") == "C:/fixtures/plugin", "optional local plugin checksum becomes null without changing the selected source");
        model.Edit(JsonNode.Parse("{\"locale\":\"zh\",\"api_url\":\"http://localhost\",\"model\":\"fixture\",\"future_flag\":true}")!.AsObject(), "set_settings", "settings");
        model.Draft!["locale"] = "en"; await model.SaveAsync();
        Check(S(fake.Args!["settings"], "locale") == "en" && S(fake.Args["settings"], "model") == "fixture" && fake.Args["settings"]!["future_flag"]!.GetValue<bool>(),
            "editing one general preference preserves model configuration and unknown options");
        model.Edit(new() { ["name"] = "analysis", ["env_var"] = "ANALYSIS_KEY", ["value"] = "synthetic-secret" }, "add_custom_credential"); await model.SaveAsync();
        Check(S(fake.Args, "envVar") == "ANALYSIS_KEY" && S(fake.Args, "value") == "synthetic-secret" && model.Draft == null,
            "credential write uses the keyring host command and clears local draft after success");
        model.Edit(new() { ["content"] = "old" }, "update_global_memory", arguments: new() { ["id"] = "memory-id" });
        model.Draft!["content"] = "new"; await model.SaveAsync();
        Check(S(fake.Args, "id") == "memory-id" && S(fake.Args, "content") == "new", "memory edits preserve authoritative row identity");
        var action = NativeWorkflowDrafts.QuickAction(4);
        action["name"] = "Inspect selection"; action["workflow_template_id"] = "template-1";
        model.Edit(action, "save_quick_action", "action"); await model.SaveAsync();
        Check(S(fake.Args!["action"], "context") == "selection" && S(fake.Args["action"], "workflow_template_id") == "template-1"
            && fake.Args["action"]!["sort_order"]!.GetValue<int>() == 4, "quick action save binds the selected template with selection context and ordering");
        var specialist = NativeWorkflowDrafts.Specialist();
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
        model.Edit(new() { ["content"] = "draft" }, "create_global_memory");
        await model.LoadAsync("get_memory_view");
        Check(S(model.Draft, "content") == "draft", "reads do not replace an editor draft");
        model.Discard(); Check(model.Draft == null, "discard clears only the local draft");
        fake.Pending = new(); var read = model.LoadAsync("get_memory_view");
        model.Dispose(); fake.Pending.SetResult(new JsonObject { ["files"] = new JsonArray() }); await read;
        Check(model.Values.Count == 0 && model.Draft == null, "late response cannot repopulate a closed settings view");
        using var failedLoad = new NativeSettingsEditorModel(new Fake { Fail = true }, "other");
        Check(!await failedLoad.LoadAsync("list_skills") && failedLoad.Values.Count == 0 && failedLoad.Error != null, "failed reads expose an error without invented empty success");
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
