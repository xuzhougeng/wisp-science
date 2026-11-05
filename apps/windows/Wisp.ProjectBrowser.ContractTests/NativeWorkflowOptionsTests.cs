using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeWorkflowOptionsTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        Check(NativeWorkflowLayout.StackToolbar(318) && !NativeWorkflowLayout.StackToolbar(630), "narrow graph controls wrap below their summary instead of squeezing it into a word-wide column");
        var task = new JsonObject { ["capabilities"] = new JsonArray("reasoning", "future:custom"), ["instruction"] = "请填写委派目标。" };
        Check(!NativeWorkflowOptions.CanAssignSpecialist("recap") && !NativeWorkflowOptions.CanAssignSpecialist("archivist")
            && NativeWorkflowOptions.CanAssignSpecialist("custom:archivist"), "only the two reserved document reviewers are excluded from task assignments");
        Check(NativeWorkflowOptions.ModelOptionLabel(new JsonObject { ["id"] = "local", ["label"] = "Lab" }) == "Lab"
            && NativeWorkflowOptions.ModelOptionLabel(new JsonObject { ["id"] = "remote", ["label"] = "Lab", ["external"] = true }) == "Lab · external"
            && NativeWorkflowOptions.ModelOptionLabel(new JsonObject { ["id"] = "unlabelled", ["external"] = true }) == "unlabelled · external",
            "model choices distinguish same-name external models and fall back to exact IDs");
        NativeWorkflowOptions.SetCapability(task, "code_run", true); NativeWorkflowOptions.SetCapability(task, "code_run", true);
        NativeWorkflowOptions.SetCapability(task, "reasoning", false);
        Check(NativeWorkflowOptions.CapabilityIds(task).SequenceEqual(["future:custom", "code_run"]), "catalog editing preserves unknown IDs and never duplicates an enabled capability");
        Check(task["instruction"]!.ToString() == "请填写委派目标。", "localized labels never translate user content");
        Check(NativeWorkflowOptions.ExecutorKey(NativeWorkflowOptions.Executor("acp:profile:with:colon")) == "acp:profile:with:colon", "executor profile IDs retain embedded colons");
        Check(NativeWorkflowOptions.Executor("") == null && NativeWorkflowOptions.ExecutorKey(NativeWorkflowOptions.Executor("native")) == "native", "policy default and native executor retain their contract shapes");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> {
            ["en"] = new() { ["workflow_studio.name_required"] = "A Workflow name is required.", ["workflow_studio.graph_cycle"] = "Cycle detected." },
            ["zh"] = new() { ["workflow_studio.name_required"] = "请填写工作流名称。" }
        }, "en");
        var fake = new Fake(); using var model = new NativeWorkflowStudioModel(fake, "project-options", locale);
        Check(await model.LoadAsync() && model.Capabilities.Length == 1 && model.Executors.Length == 2 && model.Models.Length == 1, "studio loads the shared execution catalog");
        Check(model.Models[0]["label"]!.ToString() == "Configured model" && fake.OptionProject == "project-options", "catalog models join labels without changing project scope");
        Check(!model.HasChanges && fake.Writes == 0, "catalog reads never mutate the stored template or create writes");
        var selected = JsonNode.Parse("""{"capabilities":["reasoning","future:cap","extra","last"]}""")!.AsObject();
        Check(NativeWorkflowOptions.CapabilitySummary(selected, model.Capabilities) == "Reasoning · future:cap · extra +1",
            "only catalogued IDs use display names; the summary preserves the draft order");
        fake.Response = new JsonObject { ["capabilities"] = new JsonArray(), ["models"] = new JsonArray() };
        Check(await model.LoadAsync() && model.OptionsError == null && model.Capabilities.Length == 0 && model.Models.Length == 0
            && NativeWorkflowOptions.CapabilitySummary(selected, model.Capabilities) == "reasoning · future:cap · extra +1"
            && NativeWorkflowOptions.CapabilityIds(selected).Length == 4 && !model.HasChanges,
            "an empty policy catalog offers no controls while old selections remain in the summary and draft");
        fake.Response = new JsonObject();
        Check(await model.LoadAsync() && model.OptionsError != null && model.Templates.Length == 1,
            "a missing capability list reports a catalog error instead of a valid empty policy");
        Check(!await model.RunAsync<bool>(() => throw new IOException("Synthetic failure"), _ => { })
            && model.Error == "The operation could not be confirmed; it will not be retried automatically.\nSynthetic failure",
            "unconfirmed errors localize the native prefix and preserve host error text");
        model.New(); Check(model.SaveBlockReason == "A Workflow name is required.", "workflow validation follows English locale");
        locale.Preview("zh"); Check(model.SaveBlockReason == "请填写工作流名称。", "validation uses the current locale");
        fake.FailOptions = true; Check(await model.LoadAsync() && model.OptionsError != null && model.Capabilities.Length == 0 && model.Templates.Length == 1, "catalog failure keeps the workflow available and reports missing options");
        Check(fake.Writes == 0, "failed reads never dispatch writes or retries");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool FailOptions; public string? OptionProject; public int Writes;
        public JsonObject? Response;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "list_workflow_templates") return Task.FromResult<JsonNode?>(new JsonArray(NativeWorkflowDrafts.Workflow()));
            if (command == "list_specialists") return Task.FromResult<JsonNode?>(new JsonArray());
            if (command == "list_models") return Task.FromResult<JsonNode?>(JsonNode.Parse("[{\"id\":\"model-a\",\"label\":\"Configured model\"},{\"id\":\"not-eligible\"}]"));
            if (command == "get_dynamic_agent_options")
            {
                OptionProject = projectId; if (FailOptions) throw new IOException("Synthetic catalog failure");
                if (Response != null) return Task.FromResult<JsonNode?>(Response.DeepClone());
                return Task.FromResult(JsonNode.Parse("{\"capabilities\":[{\"id\":\"reasoning\",\"display_name\":\"Reasoning\",\"risk\":\"read_only\"}],\"executors\":[{\"id\":\"native\",\"kind\":\"native\",\"available\":true},{\"id\":\"acp:missing\",\"kind\":\"acp\",\"available\":false}],\"models\":[{\"id\":\"model-a\",\"external\":false}]}"));
            }
            Writes++; throw new InvalidOperationException("Unexpected write");
        }
    }
}
