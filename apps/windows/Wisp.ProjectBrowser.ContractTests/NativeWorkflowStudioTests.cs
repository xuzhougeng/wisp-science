using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeWorkflowStudioTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    private static JsonObject Node(string id, params string[] dependencies) => new()
    { ["id"] = id, ["instruction"] = "Synthetic instruction", ["capabilities"] = new JsonArray("code_run"), ["depends_on"] = new JsonArray(dependencies.Select(d => (JsonNode?)JsonValue.Create(d)).ToArray()), ["future_task_option"] = 42 };
    public static async Task RunAsync()
    {
        Check(NativeWorkflowLayout.ForWidth(1103) == new NativeWorkflowLayout(210, 260), "studio defaults match the WebView's restored columns");
        Check(NativeWorkflowLayout.ForWidth(1707) == new NativeWorkflowLayout(240, 300), "wide studio uses the larger inspector");
        var resized = NativeWorkflowLayout.ForWidth(1000, 900, 900);
        Check(resized.LibraryWidth == 320 && Math.Abs(resized.InspectorWidth - 353.6) < .0001, "dragging keeps sidebars within 32% and 52% of their containers");
        Check(NativeWorkflowLayout.ForWidth(0, 200, 300) == new NativeWorkflowLayout(0, 0), "collapsed windows never produce negative column widths");
        Check(NativeWorkflowLayout.StackPanels(720) && NativeWorkflowLayout.StackPanels(650) && !NativeWorkflowLayout.StackPanels(721), "the WebView 720px breakpoint stacks library, graph and inspector");
        Check(!NativeWorkflowLayout.ShowHeaderSummary(980) && NativeWorkflowLayout.ShowHeaderSummary(981), "the compact header hides its summary at the WebView breakpoint");
        Check(NativeWorkflowLayout.ConfigurationBodyMaxHeight(1707, 1000, false) == 760
            && NativeWorkflowLayout.ConfigurationBodyMaxHeight(1707, 1000, true) == 420
            && NativeWorkflowLayout.ConfigurationBodyMaxHeight(1000, 600, true) == 252,
            "configuration scrolling leaves space for the bottom-aligned selected-node summary");
        Check(NativeWorkflowLayout.ConfigurationBodyMaxHeight(653, 1000, true) == 400
            && NativeWorkflowLayout.ConfigurationBodyMaxHeight(1707, double.NaN, false) == 0,
            "stacked configuration uses its own cap and invalid viewport heights stay nonnegative");
        var source = NativeWorkflowDrafts.Workflow(); source["id"] = "builtin"; source["builtin"] = true; source["name"] = "Synthetic research";
        source["proposal"]!["tasks"] = new JsonArray(Node("data"), Node("literature"), Node("synthesis", "data", "literature"));
        source["proposal"]!["goal"] = "Synthetic goal";
        source["future_template_option"] = "keep";
        await LiveDrafts(source);
        await RestoreOptionalFields(source);
        var fake = new Fake(source); using var model = new NativeWorkflowStudioModel(fake, "project-a");
        Check(await model.LoadAsync() && model.Builtin && !model.HasChanges, "loading a template opens a clean isolated draft");
        var summaryLocale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> { ["en"] = new()
        { ["agents.approval.review_all"] = "Review every draft", ["agents.approval.auto_safe"] = "Auto-run safe tasks", ["workflow_studio.tasks"] = "{count} tasks",
          ["agents.approval.review_all_help"] = "Every resolved plan waits for approval.", ["agents.approval.auto_safe_help"] = "Only policy-approved low-risk plans start automatically." } }, "en");
        var summaryClient = new Fake(source); using var summaryModel = new NativeWorkflowStudioModel(summaryClient, "project-a", summaryLocale); await summaryModel.LoadAsync();
        Check(summaryModel.ApprovalPolicyLabel == "Review every draft" && summaryModel.TaskCountLabel == "3 tasks", "header summary reflects the loaded proposal in the current locale");
        Check(summaryModel.ApprovalPolicyHelp == "Every resolved plan waits for approval.", "initial approval help describes the actual selected policy");
        summaryModel.Change(summaryModel.Proposal!, "approval_policy", JsonValue.Create("auto_safe")); summaryModel.RemoveTask("literature");
        Check(summaryModel.ApprovalPolicyLabel == "Auto-run safe tasks" && summaryModel.TaskCountLabel == "2 tasks" && summaryClient.Writes.Count == 0, "policy and task edits immediately refresh the unsaved header without writing");
        Check(summaryModel.ApprovalPolicyHelp == "Only policy-approved low-risk plans start automatically.", "approval help updates with the header policy");
        summaryModel.Change(summaryModel.Proposal!, "approval_policy", JsonValue.Create("unknown"));
        Check(summaryModel.ApprovalPolicyLabel == "Review every draft", "unknown policies use the same conservative display fallback as WebView");
        Check(summaryModel.ApprovalPolicyHelp == "Every resolved plan waits for approval.", "unknown policies also retain conservative help text");
        summaryModel.Select("builtin");
        Check(summaryModel.TaskCountLabel == "3 tasks" && summaryModel.ApprovalPolicyLabel == "Review every draft", "reselecting a template resets both summary values");
        using var referenced = new NativeWorkflowStudioModel(new Fake(source), "project-a");
        Check(await referenced.LoadAsync("builtin") && referenced.SelectedId == "builtin" && !referenced.HasChanges,
            "opening a quick action's bound workflow loads a clean selected template");
        Check(model.SaveBlockReason == null && NativeWorkflowGraph.CapabilityCount(model.Tasks[0]) == 1 && NativeWorkflowGraph.Role(model.Tasks[0]) == "临时智能体", "valid agent nodes display their capability count and role");
        model.SelectTask("data");
        Check(model.SelectedTaskId == "data" && !model.HasChanges && fake.Writes.Count == 0, "selection is view state, not an edit or persistence request");
        var after = model.NewTask(model.SelectedTaskId);
        Check(NativeWorkflowGraph.Dependencies(after).SequenceEqual(["data"]) && model.Tasks.Length == 3, "after-selected drafts inherit only the selected dependency without inserting yet");
        Check(S(after, "id") == "task_1" && after["capabilities"]![0]!.GetValue<string>() == "reasoning", "new nodes use the WebView identity and default reasoning capability");
        var graph = NativeWorkflowGraph.Build(model.Tasks);
        Check(graph.Stages.Length == 2 && graph.Stages[0].Count == 2 && graph.Stages[1].Count == 1 && graph.Edges.Length == 2, "parallel evidence tasks precede their dependent synthesis");
        Check(graph.Nodes[0].X == graph.Nodes[1].X && graph.Nodes[2].X > graph.Nodes[0].X && graph.Nodes[0].Y < graph.Nodes[1].Y, "graph geometry reflects dependency direction and parallel branches");
        var gesture = new NativeWorkflowConnection(); var target = graph.Nodes[1];
        gesture.Begin("data", 316, 166); gesture.Move(318, 168);
        Check(gesture.Release(graph, 316, 166) == null && gesture.Source == "data", "a click arms connection mode without changing dependencies");
        Check(gesture.Target(graph, graph.Nodes[0].X + 20, graph.Nodes[0].Y + 20) == null, "source nodes are never drop targets");
        Check(gesture.Target(graph, target.X - 10, target.Y + NativeWorkflowGraph.NodeHeight / 2) == "literature", "the input port is a target outside the node card");
        gesture.Move(target.X + 40, target.Y + 40);
        Check(gesture.IsDragging && gesture.Release(graph, target.X + 40, target.Y + 40) == "literature" && gesture.Source == null, "a drag resolves the card under the pointer and clears capture state");
        gesture.Begin("data", 316, 166); gesture.Move(400, 500);
        Check(gesture.Release(graph, -20, -20) == null && gesture.Source == null, "dropping in empty space cancels without arming another connection");
        gesture.Begin("data", 316, 166); gesture.Move(600, 500);
        Check(gesture.Release(graph, double.NaN, double.NaN) == null && gesture.Source == null, "clipped or out-of-viewport releases cannot connect hidden nodes");
        gesture.Begin("data", 316, 166); gesture.Cancel();
        Check(gesture.Source == null && !gesture.IsDragging && gesture.Target(graph, target.X, target.Y) == null && !model.HasChanges && fake.Writes.Count == 0, "Escape/capture cancellation does not mutate or save a workflow");
        Check(graph.FitZoom(630, 600) == 76 && graph.FitZoom(0, 0) == 25 && graph.FitZoom(10000, 10000) == 140, "fit zoom uses the WebView bounds and geometry");
        Check(graph.ShowMinimap(69) && !graph.ShowMinimap(70) && NativeWorkflowGraph.Build(Enumerable.Range(0, 9).Select(i => Node("node_" + i))).ShowMinimap(100), "minimap matches the WebView threshold: over eight tasks or below 70% zoom");
        var activitySource = (JsonObject)source.DeepClone(); var activityTask = activitySource["proposal"]!["tasks"]![2]!.AsObject();
        activityTask["model_id"] = "old-model"; activityTask["specialist_id"] = "old-specialist"; activityTask["isolated"] = true;
        activityTask["budget"] = new JsonObject { ["max_tokens"] = 123 }; activityTask["timeout_secs"] = 42;
        NativeWorkflowStudioModel.ChangeTaskKind(activityTask, "run_activity");
        Check(S(activityTask["run_activity"], "input_task_id") == "data" && S(activityTask["run_activity"], "context_id") == "local"
            && activityTask["run_activity"]!["max_candidates"]!.GetValue<int>() == 20, "switching to a compute activity uses WebView defaults and the first dependency");
        Check(NativeWorkflowGraph.CapabilityCount(activityTask) == 0 && activityTask["model_id"] == null && activityTask["specialist_id"] == null
            && activityTask["budget"] == null && activityTask["timeout_secs"] == null && activityTask["isolated"]!.GetValue<bool>() == false, "compute activities clear agent-only overrides");
        var activityFake = new Fake(activitySource); using var activityModel = new NativeWorkflowStudioModel(activityFake, "project-a"); await activityModel.LoadAsync();
        Check(activityModel.SaveBlockReason == null, "the default compute activity is saveable with an upstream task");
        var badBudget = activityModel.Tasks[2]; badBudget["run_activity"]!["max_candidates"] = 0; activityModel.ApplyTask("synthesis", badBudget);
        Check(activityModel.SaveBlockReason?.Contains("预算") == true && !await activityModel.SaveAsync() && activityFake.Writes.Count == 0, "invalid compute budgets are blocked before host dispatch");
        activityModel.Select("builtin"); var renamedInput = activityModel.Tasks[0]; renamedInput["id"] = "observations"; activityModel.ApplyTask("data", renamedInput);
        Check(S(activityModel.Tasks[2]["run_activity"], "input_task_id") == "observations", "renaming preserves the computation's input reference");
        activityModel.Disconnect("observations", "synthesis");
        Check(S(activityModel.Tasks[2]["run_activity"], "input_task_id") == "literature", "disconnecting the input selects the remaining upstream task");
        activityModel.Connect("observations", "synthesis"); activityModel.RemoveTask("literature");
        Check(S(activityModel.Tasks[1]["run_activity"], "input_task_id") == "observations", "removing an input node chooses the next surviving dependency");
        activityModel.RemoveTask("observations");
        Check(S(activityModel.Tasks[0]["run_activity"], "input_task_id") == "" && activityModel.SaveBlockReason?.Contains("输入任务") == true, "an orphaned computation cannot save until its input is repaired");
        NativeWorkflowStudioModel.ChangeTaskKind(activityTask, "agent");
        Check(activityTask["capabilities"]![0]!.GetValue<string>() == "reasoning", "returning to an agent restores the default reasoning capability");
        Check(!model.Connect("synthesis", "data") && !model.HasChanges && fake.Writes.Count == 0, "a cycle is rejected without modifying or saving the draft");
        var renamed = model.Tasks[0]; renamed["id"] = "observations";
        Check(model.ApplyTask("data", renamed) && NativeWorkflowGraph.Dependencies(model.Tasks[2]).Contains("observations"), "renaming updates references in the same draft");
        Check(model.SelectedTaskId == "observations" && S(model.SelectedTask, "id") == "observations", "renaming preserves the selected node");
        Check(S(source["proposal"]!["tasks"]![0], "id") == "data" && S(model.Templates[0]["proposal"]!["tasks"]![0], "id") == "data", "editing never mutates the stored or library template");
        model.SelectTask("literature"); model.RemoveTask("literature");
        Check(model.SelectedTaskId == null, "removing the selected node closes its summary");
        Check(model.Tasks.Length == 2 && NativeWorkflowGraph.Dependencies(model.Tasks[1]).SequenceEqual(["observations"]), "removing a task removes incident dependencies");
        fake.FailNextSave = true;
        Check(!await model.SaveAsync() && model.HasChanges && model.Builtin && fake.Writes.Count == 1, "a failed save retains the draft and is not automatically retried");
        Check(await model.SaveAsync() && !model.HasChanges && !model.Builtin && model.SelectedId == "saved-copy", "explicit save-as-copy adopts the host-confirmed identity");
        var submitted = fake.Writes.Last()["template"]!;
        Check(S(submitted, "id") == "" && submitted["builtin"]!.GetValue<bool>() == false && S(submitted, "future_template_option") == "keep", "built-in saves request a fresh identity and retain future fields");
        Check(submitted["proposal"]!["tasks"]![0]!["future_task_option"]!.GetValue<int>() == 42, "task edits retain fields unknown to this UI");
        model.SelectTask("observations"); model.Select("builtin");
        Check(model.SelectedTaskId == null, "switching templates clears selection from the old document");
        Check(!await model.DeleteAsync() && fake.DeleteCount == 0, "built-in deletion is refused before dispatch");
        model.New(); Check(model.Tasks.Length == 1 && model.SelectedId.Length == 0 && !model.Builtin, "blank workflows start with an unsaved task");
        var writes = fake.Writes.Count;
        Check(!await model.SaveAsync() && fake.Writes.Count == writes, "empty names and instructions cannot reach persistence");
        model.Change(model.Draft!, "name", JsonValue.Create("Unsaved"));
        Check(model.SaveBlockReason?.Contains("目标") == true, "missing goal is visible before save");
        model.Change(model.Proposal!, "goal", JsonValue.Create("Goal"));
        Check(model.SaveBlockReason?.Contains("指令") == true, "missing instruction is visible before save");
        var incomplete = model.Tasks[0]; incomplete["instruction"] = "Instruction"; incomplete["capabilities"] = new JsonArray(); model.ApplyTask(S(incomplete, "id"), incomplete);
        Check(model.SaveBlockReason?.Contains("能力") == true && !await model.SaveAsync() && fake.Writes.Count == writes, "missing agent capabilities cannot reach persistence");
        incomplete["task_kind"] = "run_activity"; model.ApplyTask(S(incomplete, "id"), incomplete);
        Check(model.SaveBlockReason?.Contains("输入任务") == true && NativeWorkflowGraph.Role(incomplete) == "计算活动", "run activities require an input dependency and show their own role");
        incomplete["skill_ids"] = new JsonArray("legacy"); model.ApplyTask(S(incomplete, "id"), incomplete);
        Check(model.SaveBlockReason?.Contains("独立指令") == true, "legacy skill-bound nodes must be converted before saving");
        var invalid = new[] { Node("a", "b"), Node("b", "a") };
        Check(NativeWorkflowGraph.Validate(invalid) != null && NativeWorkflowGraph.Build(invalid).Nodes.Length == 2, "invalid cycles remain visible for review but cannot save");
        Check(NativeWorkflowGraph.Validate([Node("a", "missing")]) != null && NativeWorkflowGraph.Validate([Node("a"), Node("a")]) != null, "missing and duplicate identities are rejected");
        var pending = new TaskCompletionSource<JsonNode?>(); fake.Pending = pending.Task;
        using var late = new NativeWorkflowStudioModel(fake, null); var loading = late.LoadAsync(); late.Dispose(); pending.SetResult(new JsonArray(source.DeepClone())); await loading;
        Check(late.Draft == null && late.Templates.Length == 0, "a closed studio ignores a late load");
    }
    private static async Task LiveDrafts(JsonObject source)
    {
        var fake = new Fake(source); using var model = new NativeWorkflowStudioModel(fake, "project-a"); await model.LoadAsync();
        const string sharedContext = "Synthetic shared context — 保留原始文本\nConventions stay verbatim.";
        model.Change(model.Proposal!, "context", JsonValue.Create(sharedContext));
        var dataKey = model.TaskKeyAt(0); var literatureKey = model.TaskKeyAt(1); var synthesisKey = model.TaskKeyAt(2);
        model.SelectTaskKey(dataKey); var edit = model.TaskForKey(dataKey)!; edit["instruction"] = "Edited without Apply";
        Check(model.CanAddAfterSelected, "a unique selected ID can be used as a new dependency");
        Check(model.UpdateTaskDraft(dataKey, edit) && S(model.SelectedTask, "instruction") == "Edited without Apply" && model.HasChanges && fake.Writes.Count == 0,
            "node fields update the workflow draft immediately without dispatching a save");
        edit["id"] = ""; model.UpdateTaskDraft(dataKey, edit);
        Check(!model.CanAddAfterSelected, "blank IDs cannot create dependent tasks");
        Check(model.SelectedTaskKey == dataKey && S(model.SelectedTask, "id") == "" && model.SaveBlockReason != null && !await model.SaveAsync(),
            "clearing an ID preserves selection and editability but blocks persistence");
        edit["id"] = "literature"; model.UpdateTaskDraft(dataKey, edit);
        Check(!model.CanAddAfterSelected, "ambiguous duplicate IDs cannot create dependent tasks");
        Check(model.TaskKeyAt(0) != model.TaskKeyAt(1) && S(model.TaskForKey(literatureKey), "instruction") == "Synthetic instruction"
            && model.SaveBlockReason?.Contains("唯一") == true, "temporary duplicate IDs cannot select or overwrite a different node");
        var invalidGraph = NativeWorkflowGraph.Build(model.Tasks);
        Check(invalidGraph.Nodes[0].Index != invalidGraph.Nodes[1].Index, "duplicate labels retain distinct graph edit targets");
        edit["id"] = "observations"; model.UpdateTaskDraft(dataKey, edit);
        Check(model.CanAddAfterSelected, "repairing the ID re-enables dependent creation");
        Check(NativeWorkflowGraph.Dependencies(model.TaskForKey(synthesisKey)!).SequenceEqual(["observations", "literature"]),
            "repairing an ID updates only the original node's references after a transient duplicate");
        model.SetTaskInputError(dataKey, "output_schema", "{unfinished", "Invalid JSON");
        Check(model.TaskInputText(dataKey, "output_schema", "") == "{unfinished" && model.TaskInputError(dataKey) == "Invalid JSON"
            && model.SaveBlockReason?.Contains("Invalid JSON") == true && !await model.SaveAsync() && fake.Writes.Count == 0,
            "incomplete JSON survives closing/reopening and cannot reach the host");
        model.SetTaskInputError(dataKey, "output_schema", "{}", null);
        Check(model.TaskInputError(dataKey) == null && model.SaveBlockReason == null, "repairing an input removes its save blocker");
        model.SetTaskInputError(literatureKey, "budget.max_tokens", "not a number", "Invalid number");
        model.RemoveTaskKey(dataKey);
        Check(model.TaskKeyAt(0) == literatureKey && model.TaskInputError(literatureKey) == "Invalid number"
            && model.TaskForKey(dataKey) == null, "removing another node cannot shift invalid input state onto the wrong task");
        model.Select("builtin");
        Check(!model.HasChanges && S(model.Proposal, "context") != sharedContext
            && model.TaskForKey(literatureKey) == null && model.TaskInputError(literatureKey) == null,
            "switching documents drops old field state and invalidates old edit keys");
        dataKey = model.TaskKeyAt(0); synthesisKey = model.TaskKeyAt(2);
        Check(!model.SetDraftDependency(dataKey, "synthesis", true) && !model.HasChanges && fake.Writes.Count == 0,
            "live dependency checkboxes reject cycles without changing the draft");
        model.Change(model.Proposal!, "context", JsonValue.Create(sharedContext));
        Check(model.SetDraftDependency(synthesisKey, "literature", false)
            && NativeWorkflowGraph.Dependencies(model.TaskForKey(synthesisKey)!).SequenceEqual(["data"]), "dependency chips update the unsaved workflow immediately");
        var addedKey = model.AddTaskDraft(model.NewTask("data"));
        Check(model.Tasks.Length == 4 && model.SelectedTaskKey == addedKey && model.SaveBlockReason?.Contains("指令") == true,
            "Add Task inserts and selects a draft node before opening its editor");
        var added = model.TaskForKey(addedKey)!; added["instruction"] = "New instruction"; model.UpdateTaskDraft(addedKey, added);
        Check(model.SaveBlockReason == null && fake.Writes.Count == 0, "finishing a new task still does not implicitly save");
        var renamed = model.TaskForKey(dataKey)!; renamed["id"] = "  observations  "; model.UpdateTaskDraft(dataKey, renamed);
        Check(await model.SaveAsync() && fake.Writes.Count == 1, "only explicit workflow Save dispatches the edited document");
        Check(S(fake.Writes[0]["template"]!["proposal"], "context") == sharedContext,
            "shared context survives node edits and is submitted verbatim only on explicit Save");
        var submitted = fake.Writes[0]["template"]!["proposal"]!["tasks"]!.AsArray();
        Check(S(submitted[0], "id") == "observations" && NativeWorkflowGraph.Dependencies(submitted[2]!.AsObject()).SequenceEqual(["observations"])
            && submitted[0]!["future_task_option"]!.GetValue<int>() == 42, "save normalizes IDs and references while preserving unknown fields");
        model.New(source);
        Check(S(model.Draft, "name") == S(source, "name"), "creating from a template keeps its name as WebView does");
        var savedKey = model.TaskKeyAt(0); model.SetTaskInputError(savedKey, "budget.max_tokens", "x", "Invalid number");
        Check(model.HasChanges, "invalid text alone participates in unsaved-change detection");
        model.Dispose();
        Check(!model.UpdateTaskDraft(savedKey, added) && model.AddTaskDraft(added) == -1, "closed studios reject retained editor callbacks");
    }
    private static async Task RestoreOptionalFields(JsonObject source)
    {
        foreach (var explicitNull in new[] { false, true })
        {
            var template = (JsonObject)source.DeepClone();
            var first = template["proposal"]!["tasks"]![0]!.AsObject();
            if (explicitNull) { first["budget"] = null; first["output_schema"] = null; }
            var fake = new Fake(template); using var model = new NativeWorkflowStudioModel(fake, null);
            await model.LoadAsync(); var key = model.TaskKeyAt(0); var edit = model.TaskForKey(key)!;
            edit["budget"] = new JsonObject { ["max_tokens"] = 1200 }; edit["output_schema"] = new JsonObject { ["type"] = "object" };
            model.UpdateTaskDraft(key, edit);
            Check(model.HasChanges, "setting optional values changes the draft");
            edit = model.TaskForKey(key)!; edit["budget"]!["max_tokens"] = null; edit["output_schema"] = null;
            model.UpdateTaskDraft(key, edit);
            Check(!model.HasChanges && JsonNode.DeepEquals(model.Draft, template) && fake.Writes.Count == 0,
                "clearing optional values restores absent/null representation without a false leave warning");
            edit = model.TaskForKey(key)!; edit["budget"] = new JsonObject { ["max_tokens"] = null, ["future_budget_option"] = "keep" };
            model.UpdateTaskDraft(key, edit);
            Check(model.HasChanges && S(model.TaskForKey(key)!["budget"], "future_budget_option") == "keep",
                "empty known budget fields never erase unknown nonempty budget options");
        }
    }
    private static string S(JsonNode? node, string key) => NativeWorkflowStudioModel.Text(node, key);
    private sealed class Fake(JsonObject template) : INativeSettingsClient
    {
        public List<JsonObject> Writes { get; } = [];
        public bool FailNextSave;
        public int DeleteCount;
        public Task<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "list_workflow_templates") return Pending ?? Task.FromResult<JsonNode?>(new JsonArray(template.DeepClone()));
            if (command == "save_workflow_template")
            {
                Writes.Add((JsonObject)arguments.DeepClone());
                if (FailNextSave) { FailNextSave = false; throw new IOException("Synthetic failure"); }
                var saved = (JsonObject)arguments["template"]!.DeepClone(); saved["id"] = "saved-copy"; return Task.FromResult<JsonNode?>(saved);
            }
            if (command == "remove_workflow_template") DeleteCount++;
            return Task.FromResult<JsonNode?>(new JsonArray());
        }
    }
}
