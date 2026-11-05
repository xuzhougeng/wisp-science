using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// An unsaved studio document. Native and WebView share the host's template contract.
public sealed class NativeWorkflowStudioModel(INativeSettingsClient client, string? projectId, NativeLocalization? localization = null) : WorkspaceActionModel
{
    private string T(string key, string fallback) => localization?.Text(key) ?? fallback;
    private string L(string zh, string en) => localization?.Choose(zh, en) ?? zh;
    protected override string UnconfirmedFailure(Exception exception) => L("操作结果未能确认；不会自动重试。", "The operation could not be confirmed; it will not be retried automatically.") + "\n" + exception.Message;
    private string TaskError(string id, string reason) => localization?.Format("workflow_studio.task_error", ("task", id), ("reason", reason)) ?? $"任务 {id}：{reason}";
    public JsonObject[] Capabilities { get; private set; } = [];
    public JsonObject[] Executors { get; private set; } = [];
    public JsonObject[] Skills { get; private set; } = [];
    public string? ConversionSourceSha256 { get; private set; }
    public string? OptionsError { get; private set; }
    public JsonObject[] Templates { get; private set; } = [];
    public JsonObject[] Models { get; private set; } = [];
    public JsonObject[] Specialists { get; private set; } = [];
    public JsonObject? Draft { get; private set; }
    private JsonObject? original;
    private readonly List<long> taskKeys = [];
    private readonly Dictionary<long, string> referenceIds = [];
    private readonly Dictionary<long, JsonObject> initialTasks = [];
    private readonly Dictionary<(long Key, string Field), (string Text, string Error)> invalidInputs = [];
    private long nextTaskKey;
    public bool HasChanges => Draft != null && (!JsonNode.DeepEquals(Draft, original) || taskKeys.Any(key => TaskInputError(key) != null));
    public bool Builtin => Draft?["builtin"]?.GetValue<bool>() == true;
    public string SelectedId => Draft?["id"]?.GetValue<string>() ?? "";
    public JsonObject? Proposal => Draft?["proposal"] as JsonObject;
    public JsonObject[] Tasks => Rows(Proposal?["tasks"]);
    public string ApprovalPolicyLabel => Text(Proposal, "approval_policy") == "auto_safe"
        ? T("agents.approval.auto_safe", "安全任务自动执行") : T("agents.approval.review_all", "每个草稿都审批");
    public string ApprovalPolicyHelp => Text(Proposal, "approval_policy") == "auto_safe"
        ? T("agents.approval.auto_safe_help", "只有策略判定为低风险的计划才会自动启动。") : T("agents.approval.review_all_help", "每个解析后的计划都等待人工批准。");
    public string TaskCountLabel => localization?.Format("workflow_studio.tasks", ("count", Tasks.Length.ToString())) ?? $"{Tasks.Length} 个任务";
    public long? SelectedTaskKey { get; private set; }
    public string? SelectedTaskId => SelectedTask is { } task ? Text(task, "id") : null;
    public JsonObject? SelectedTask => SelectedTaskKey is { } key ? TaskForKey(key) : null;
    public bool CanAddAfterSelected => SelectedTaskId is { } id && !string.IsNullOrWhiteSpace(id)
        && Tasks.Count(task => Text(task, "id").Trim() == id.Trim()) == 1;
    public long TaskKeyAt(int index) => index >= 0 && index < taskKeys.Count ? taskKeys[index] : -1;
    public JsonObject? TaskForKey(long key) => taskKeys.IndexOf(key) is var index && index >= 0 ? Tasks.ElementAtOrDefault(index) : null;
    public string? TaskInputError(long key)
    {
        var run = Text(TaskForKey(key), "task_kind") == "run_activity";
        return invalidInputs.Where(entry => entry.Key.Key == key && entry.Key.Field.StartsWith("run_activity.", StringComparison.Ordinal) == run)
            .Select(entry => entry.Value.Error).FirstOrDefault();
    }
    public string TaskInputText(long key, string field, string fallback) => invalidInputs.TryGetValue((key, field), out var input) ? input.Text : fallback;
    public void SetTaskInputError(long key, string field, string text, string? error)
    {
        if (Busy || Closed || !taskKeys.Contains(key)) return;
        if (error == null) invalidInputs.Remove((key, field)); else invalidInputs[(key, field)] = (text, error);
        SavedMessage = null; ClearFailure(); Notify();
    }
    public void ClearAgentInputErrors(long key)
    { foreach (var field in invalidInputs.Keys.Where(field => field.Key == key && !field.Field.StartsWith("run_activity.", StringComparison.Ordinal)).ToArray()) invalidInputs.Remove(field); }
    public string? SavedMessage { get; private set; }
    public string? SaveBlockReason
    {
        get
        {
            if (Draft == null) return L("请选择或新建工作流。", "Select or create a workflow.");
            if (string.IsNullOrWhiteSpace(Text(Draft, "name"))) return T("workflow_studio.name_required", "请填写工作流名称。");
            if (string.IsNullOrWhiteSpace(Text(Proposal, "goal"))) return T("workflow_studio.goal_required", "请填写委派目标。");
            var tasks = Tasks;
            if (tasks.Length == 0) return T("workflow_studio.task_required", "请添加至少一个任务。");
            if (tasks.Select(task => Text(task, "id").Trim()).Distinct(StringComparer.Ordinal).Count() != tasks.Length) return L("任务 ID 必须非空且唯一。", "Task IDs must be nonempty and unique.");
            for (var index = 0; index < tasks.Length; index++)
            {
                var task = tasks[index];
                var id = Text(task, "id");
                if (string.IsNullOrWhiteSpace(id)) return T("workflow_studio.task_id_required", "请填写任务 ID。");
                if (TaskInputError(TaskKeyAt(index)) is { } inputError) return TaskError(id, inputError);
                if (string.IsNullOrWhiteSpace(Text(task, "instruction"))) return TaskError(id, T("workflow_studio.instruction_required", "请填写指令。"));
                if (task["skill_ids"] is JsonArray { Count: > 0 }) return TaskError(id, T("workflow_studio.legacy_warning", "请将旧版绑定技能转换为独立指令。"));
                if (Text(task, "task_kind") == "run_activity")
                {
                    var activity = task["run_activity"] as JsonObject;
                    if (string.IsNullOrWhiteSpace(Text(activity, "context_id")) || string.IsNullOrWhiteSpace(Text(activity, "input_task_id"))
                        || !NativeWorkflowGraph.Dependencies(task).Contains(Text(activity, "input_task_id")))
                        return TaskError(id, T("workflow_studio.run_input_required", "请指定执行环境，以及依赖中的输入任务。"));
                    foreach (var (key, maximum) in new[] { ("max_candidates", 50UL), ("max_wall_seconds", 604800UL), ("max_evaluator_seconds", 300UL), ("max_cost_microunits", ulong.MaxValue) })
                        if (!ulong.TryParse(activity?[key]?.ToString(), out var number) || number == 0 || number > maximum)
                            return TaskError(id, L($"请填写有效的计算活动预算（{key}）。", $"Enter a valid run activity budget ({key})."));
                }
                else if (NativeWorkflowGraph.CapabilityCount(task) == 0) return TaskError(id, T("workflow_studio.capability_required", "请选择至少一项工具能力。"));
            }
            return NativeWorkflowGraph.Validate(tasks, localization);
        }
    }
    private static JsonObject[] Rows(JsonNode? value) => (value as JsonArray)?.OfType<JsonObject>().Select(o => (JsonObject)o.DeepClone()).ToArray() ?? [];
    public static string Text(JsonNode? value, string key) => value?[key]?.GetValue<string>() ?? "";
    private async Task<JsonObject?> LoadOptionsAsync()
    {
        OptionsError = null;
        try
        {
            if (await client.InvokeAsync("get_dynamic_agent_options", new(), projectId) is JsonObject options
                && options["capabilities"] is JsonArray) return options;
            throw new InvalidDataException(L("宿主未返回能力列表。", "The host did not return a capability list."));
        }
        catch (Exception ex) { OptionsError = L("无法加载执行选项：", "Could not load execution options: ") + ex.Message; return null; }
    }
    public Task<bool> LoadAsync(string? templateId = null) => RunAsync(async () => (
        Templates: Rows(await client.InvokeAsync("list_workflow_templates", new(), projectId)),
        Models: Rows(await client.InvokeAsync("list_models", new(), projectId)),
        Specialists: Rows(await client.InvokeAsync("list_specialists", new(), projectId)),
        Options: await LoadOptionsAsync()), value =>
    {
        Templates = value.Templates; Models = value.Models; Specialists = value.Specialists;
        Capabilities = Rows(value.Options?["capabilities"]); Executors = Rows(value.Options?["executors"]);
        Skills = Rows(value.Options?["skills"]);
        if (value.Options?["models"] is JsonArray)
            Models = Rows(value.Options["models"]).Select(option =>
            {
                var profile = Models.FirstOrDefault(row => Text(row, "id") == Text(option, "id"));
                option["label"] = Text(profile, "label") is { Length: > 0 } label ? label : Text(option, "id");
                option["active"] = profile?["active"]?.GetValue<bool>() == true;
                return option;
            }).ToArray();
        if (Draft == null) LoadDraft(Templates.FirstOrDefault(template => Text(template, "id") == templateId) ?? Templates.FirstOrDefault());
    });
    private void LoadDraft(JsonObject? value)
    {
        ConversionSourceSha256 = null;
        Draft = (JsonObject?)value?.DeepClone(); original = (JsonObject?)Draft?.DeepClone(); SelectedTaskKey = null;
        taskKeys.Clear(); referenceIds.Clear(); initialTasks.Clear(); invalidInputs.Clear();
        foreach (var task in Tasks) { var key = ++nextTaskKey; taskKeys.Add(key); referenceIds[key] = Text(task, "id"); initialTasks[key] = task; }
        SavedMessage = null; ClearFailure();
    }
    public void SelectTask(string? id)
    { var index = Array.FindIndex(Tasks, task => Text(task, "id") == id); SelectTaskKey(index >= 0 ? TaskKeyAt(index) : null); }
    public void SelectTaskKey(long? key)
    { if (Busy || Closed) return; SelectedTaskKey = key is { } value && taskKeys.Contains(value) ? value : null; Notify(); }
    public void Select(string id)
    {
        if (Busy || Closed) return;
        LoadDraft(Templates.FirstOrDefault(t => Text(t, "id") == id)); Notify();
    }
    public void New(JsonObject? template = null)
    {
        if (Busy || Closed) return;
        var value = template == null ? NativeWorkflowDrafts.Workflow() : NativeWorkflowDrafts.Copy(template);
        if (template != null) value["name"] = Text(template, "name");
        value["id"] = ""; value["builtin"] = false;
        if (template == null) value["proposal"]!["tasks"] = new JsonArray(CreateTask([], null));
        LoadDraft(value); Notify();
    }
    public void Change(JsonObject target, string key, JsonNode? value)
    { if (Busy || Closed) return; target[key] = value; SavedMessage = null; ClearFailure(); Notify(); }
    public bool UseConversion(JsonObject result, string? legacyTemplateId = null)
    {
        if (Busy || Closed || result["proposal"] is not JsonObject proposal) return false;
        var prior = Templates.FirstOrDefault(t => Text(t, "id") == legacyTemplateId);
        var draft = prior == null ? NativeWorkflowDrafts.Workflow() : (JsonObject)prior.DeepClone();
        if (prior == null) { draft["id"] = ""; draft["builtin"] = false; draft["name"] = NativeWorkflowConversionModel.ShortName(Text(proposal, "goal")); }
        draft["description"] = localization?.Format("workflow_studio.portfolio.template_description", ("model", Text(result["plan"], "planner_model_label")))
            ?? "Independent workflow converted from source methods with " + Text(result["plan"], "planner_model_label");
        draft["proposal"] = proposal.DeepClone(); LoadDraft(draft);
        original = prior == null ? null : (JsonObject)prior.DeepClone();
        ConversionSourceSha256 = Text(result["plan"], "source_sha256") is { Length: > 0 } hash ? hash : null;
        Notify(); return true;
    }
    public JsonObject NewTask(string? after = null) => CreateTask(Tasks, after);
    public long AddTaskDraft(JsonObject task)
    {
        if (Busy || Closed || Proposal?["tasks"] is not JsonArray tasks) return -1;
        var key = ++nextTaskKey; tasks.Add(task.DeepClone()); taskKeys.Add(key); referenceIds[key] = Text(task, "id");
        initialTasks[key] = (JsonObject)task.DeepClone();
        SelectedTaskKey = key; SavedMessage = null; ClearFailure(); Notify(); return key;
    }
    /// Edits stay in the workflow draft. Stable keys let blank/duplicate IDs remain editable.
    public bool UpdateTaskDraft(long key, JsonObject task)
    {
        var index = taskKeys.IndexOf(key);
        if (Busy || Closed || Proposal?["tasks"] is not JsonArray tasks || index < 0) return false;
        var copy = (JsonObject)task.DeepClone();
        RestoreEmptyOptionalFields(copy, initialTasks[key]);
        tasks[index] = copy; var id = Text(task, "id");
        // Keep references on the last unambiguous name while an ID is incomplete.
        // Otherwise typing a temporary duplicate can silently rewire another node.
        if (!string.IsNullOrWhiteSpace(id) && tasks.OfType<JsonObject>().Count(row => Text(row, "id").Trim() == id.Trim()) == 1)
        {
            RenameReferences(tasks, referenceIds[key], id); referenceIds[key] = id;
        }
        SavedMessage = null; ClearFailure(); Notify(); return true;
    }
    // Clearing an optional control restores its original absent/null shape. In particular,
    // an empty budget must not turn an unchanged node into an unsaved edit.
    private static void RestoreEmptyOptionalFields(JsonObject task, JsonObject baseline)
    {
        foreach (var field in new[] { "specialist_id", "model_id", "executor", "timeout_secs", "output_schema" })
            if (task[field] == null && !baseline.ContainsKey(field)) task.Remove(field);
        if (task["budget"] is not JsonObject budget) return;
        var originalBudget = baseline["budget"] as JsonObject;
        foreach (var field in new[] { "max_tokens", "max_tool_calls", "max_cost_microunits" })
            if (budget[field] == null && originalBudget?.ContainsKey(field) != true) budget.Remove(field);
        if (budget.Count == 0 && originalBudget == null)
        {
            if (baseline.ContainsKey("budget")) task["budget"] = null;
            else task.Remove("budget");
        }
    }
    private static void RenameReferences(JsonArray tasks, string previousId, string id)
    {
        if (previousId == id) return;
        foreach (var row in tasks.OfType<JsonObject>())
        {
            if (row["depends_on"] is JsonArray dependencies)
                for (var i = 0; i < dependencies.Count; i++) if (dependencies[i]?.GetValue<string>() == previousId) dependencies[i] = id;
            if (row["run_activity"] is JsonObject activity && Text(activity, "input_task_id") == previousId) activity["input_task_id"] = id;
        }
    }
    public bool SetDraftDependency(long key, string source, bool enabled)
    {
        var task = TaskForKey(key); if (task == null || Busy || Closed) return false;
        var dependencies = NativeWorkflowGraph.Dependencies(task).ToList();
        if (enabled) { if (dependencies.Contains(source)) return true; dependencies.Add(source); } else dependencies.Remove(source);
        task["depends_on"] = new JsonArray(dependencies.Select(id => (JsonNode?)JsonValue.Create(id)).ToArray()); ReconcileActivityInput(task);
        var candidates = Tasks; candidates[taskKeys.IndexOf(key)] = task;
        if (enabled && NativeWorkflowGraph.Validate(candidates, localization) is { } error) { Fail(error); return false; }
        return UpdateTaskDraft(key, task);
    }
    public static void ChangeTaskKind(JsonObject task, string kind)
    {
        task["task_kind"] = kind;
        if (kind == "run_activity")
        {
            task["run_activity"] ??= new JsonObject
            {
                ["activity"] = "method_search", ["context_id"] = "local",
                ["input_task_id"] = NativeWorkflowGraph.Dependencies(task).FirstOrDefault() ?? "",
                ["spec_output_pointer"] = "method_search_spec_artifact_version_id",
                ["max_candidates"] = 20, ["max_wall_seconds"] = 14400,
                ["max_evaluator_seconds"] = 120, ["max_cost_microunits"] = 5000000
            };
            task["capabilities"] = new JsonArray(); task["isolated"] = false;
            foreach (var key in new[] { "specialist_id", "output_schema", "model_id", "executor", "budget", "timeout_secs" }) task[key] = null;
            ReconcileActivityInput(task);
        }
        else if (NativeWorkflowGraph.CapabilityCount(task) == 0) task["capabilities"] = new JsonArray("reasoning");
    }
    public static void ReconcileActivityInput(JsonObject task)
    {
        if (task["run_activity"] is not JsonObject activity) return;
        var dependencies = NativeWorkflowGraph.Dependencies(task);
        if (!dependencies.Contains(Text(activity, "input_task_id"))) activity["input_task_id"] = dependencies.FirstOrDefault() ?? "";
    }
    private static JsonObject CreateTask(JsonObject[] existing, string? after)
    {
        var task = NativeWorkflowDrafts.TaskNode(existing);
        var number = 1; while (existing.Any(row => Text(row, "id") == $"task_{number}")) number++;
        task["id"] = $"task_{number}"; task["capabilities"] = new JsonArray("reasoning");
        if (after != null) task["depends_on"] = new JsonArray(JsonValue.Create(after));
        return task;
    }
    public bool ApplyTask(string? previousId, JsonObject task)
    {
        if (Busy || Closed || Draft == null) return false;
        var id = Text(task, "id").Trim();
        if (id.Length == 0) { Fail(T("workflow_studio.task_id_required", "请填写任务 ID。")); return false; }
        var next = (JsonObject)Draft.DeepClone(); var tasks = next["proposal"]!["tasks"]!.AsArray();
        var index = tasks.ToList().FindIndex(t => Text(t, "id") == previousId);
        if (tasks.OfType<JsonObject>().Any(t => Text(t, "id") == id && Text(t, "id") != previousId))
        { Fail(L("任务 ID 已存在。", "This task ID already exists.")); return false; }
        var copy = (JsonObject)task.DeepClone(); copy["id"] = id;
        if (previousId != null && index < 0) { Fail(L("任务已不存在。", "This task no longer exists.")); return false; }
        if (index < 0) tasks.Add(copy); else tasks[index] = copy;
        if (previousId != null) RenameReferences(tasks, previousId, id);
        if (NativeWorkflowGraph.Validate(tasks.OfType<JsonObject>(), localization) is { } error) { Fail(error); return false; }
        Draft = next;
        if (index < 0) { var key = ++nextTaskKey; taskKeys.Add(key); referenceIds[key] = id; initialTasks[key] = (JsonObject)copy.DeepClone(); }
        else referenceIds[taskKeys[index]] = id;
        SavedMessage = null; ClearFailure(); Notify(); return true;
    }
    public void RemoveTask(string id)
    { var index = Array.FindIndex(Tasks, task => Text(task, "id") == id); if (index >= 0) RemoveTaskKey(TaskKeyAt(index)); }
    public void RemoveTaskKey(long key)
    {
        if (Busy || Closed || Proposal == null) return;
        var tasks = Proposal["tasks"]!.AsArray();
        var index = taskKeys.IndexOf(key); if (index < 0) return;
        var id = referenceIds[key]; tasks.RemoveAt(index); taskKeys.RemoveAt(index); referenceIds.Remove(key); initialTasks.Remove(key);
        foreach (var input in invalidInputs.Keys.Where(input => input.Key == key).ToArray()) invalidInputs.Remove(input);
        if (SelectedTaskKey == key) SelectedTaskKey = null;
        foreach (var task in tasks.OfType<JsonObject>())
        {
            task["depends_on"] = new JsonArray(NativeWorkflowGraph.Dependencies(task)
                .Where(d => d != id).Select(d => (JsonNode?)JsonValue.Create(d)).ToArray());
            ReconcileActivityInput(task);
        }
        SavedMessage = null; ClearFailure(); Notify();
    }
    public bool Connect(string source, string target)
    {
        var task = Tasks.FirstOrDefault(t => Text(t, "id") == target);
        if (task == null || source == target || !Tasks.Any(t => Text(t, "id") == source)) return false;
        var dependencies = NativeWorkflowGraph.Dependencies(task).ToList();
        if (dependencies.Contains(source)) return true;
        dependencies.Add(source); task["depends_on"] = new JsonArray(dependencies.Select(d => (JsonNode?)JsonValue.Create(d)).ToArray());
        ReconcileActivityInput(task);
        return ApplyTask(target, task);
    }
    public void Disconnect(string source, string target)
    {
        var task = Tasks.FirstOrDefault(t => Text(t, "id") == target);
        if (task == null) return;
        task["depends_on"] = new JsonArray(NativeWorkflowGraph.Dependencies(task).Where(d => d != source).Select(d => (JsonNode?)JsonValue.Create(d)).ToArray());
        ReconcileActivityInput(task);
        ApplyTask(target, task);
    }
    public async Task<bool> SaveAsync()
    {
        if (Draft == null || Busy || Closed) return false;
        if (SaveBlockReason is { } error) { Fail(error); return false; }
        var submitted = (JsonObject)Draft.DeepClone();
        var submittedTasks = submitted["proposal"]!["tasks"]!.AsArray();
        foreach (var task in submittedTasks.OfType<JsonObject>())
        {
            var id = Text(task, "id"); task["id"] = id.Trim(); RenameReferences(submittedTasks, id, id.Trim());
            if (Text(task, "task_kind") == "run_activity") ChangeTaskKind(task, "run_activity"); else task["run_activity"] = null;
        }
        if (Builtin) submitted["id"] = "";
        submitted["builtin"] = false;
        var args = new JsonObject { ["template"] = submitted };
        if (ConversionSourceSha256 != null) args["conversionSourceSha256"] = ConversionSourceSha256;
        return await RunAsync(async () => await client.InvokeAsync("save_workflow_template", args, projectId) as JsonObject
            ?? throw new InvalidDataException(L("宿主未返回保存后的工作流。", "The host did not return the saved workflow.")), saved =>
        {
            if (Text(saved, "id").Length == 0) throw new InvalidDataException(L("宿主返回的工作流缺少 ID。", "The saved workflow has no ID."));
            Templates = Templates.Where(t => Text(t, "id") != Text(saved, "id")).Append((JsonObject)saved.DeepClone())
                .OrderByDescending(t => t["builtin"]?.GetValue<bool>() == true).ThenBy(t => Text(t, "name"), StringComparer.Ordinal).ToArray();
            LoadDraft(saved); SavedMessage = T("workflow_studio.saved", "已保存。");
        });
    }
    public Task<bool> DeleteAsync()
    {
        if (Builtin || SelectedId.Length == 0) return Task.FromResult(false);
        var id = SelectedId;
        return RunAsync(async () => Rows(await client.InvokeAsync("remove_workflow_template", new() { ["templateId"] = id }, projectId)), rows =>
        { Templates = rows; LoadDraft(rows.FirstOrDefault()); });
    }
    public override void Dispose() { base.Dispose(); Draft = original = null; SelectedTaskKey = null; taskKeys.Clear(); referenceIds.Clear(); initialTasks.Clear(); invalidInputs.Clear(); Templates = Models = Specialists = Capabilities = Executors = Skills = []; }
}
