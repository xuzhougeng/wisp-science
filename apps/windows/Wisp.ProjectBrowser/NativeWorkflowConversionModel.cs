using System.Text;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Window-owned, project-pinned conversion. Hiding a view never cancels or replays it.
public sealed class NativeWorkflowConversionModel(string projectId) : WorkspaceActionModel
{
    public string ProjectId { get; } = projectId;
    public string Request { get; private set; } = "";
    public string ModelId { get; private set; } = "";
    public bool AutoSources { get; private set; } = true;
    public string? LegacyTemplateId { get; private set; }
    public string? LegacyWorkflowId { get; private set; }
    public string Search { get; set; } = "";
    private readonly List<string> sources = [];
    public IReadOnlyList<string> Sources => sources;
    public JsonObject[] Skills { get; private set; } = [];
    public JsonObject[] Models { get; private set; } = [];
    public JsonObject? Result { get; private set; }
    public string? RequestId { get; private set; }
    public bool DialogOpen { get; private set; }
    public DateTimeOffset? StartedAt { get; private set; }
    public string Stage { get; private set; } = "preparing";
    public string StatusKey => "workflow_studio.conversion." + (Error != null ? "failed" : Result != null ? "ready" : Stage);
    private CancellationTokenSource? progressCancellation;
    private TimeSpan elapsed;
    public TimeSpan Elapsed => Busy && StartedAt is { } start ? DateTimeOffset.UtcNow - start : elapsed;
    public bool Legacy => LegacyTemplateId != null || LegacyWorkflowId != null;
    public bool Ready => !Busy && !Closed && ProjectId.Length > 0 && !string.IsNullOrWhiteSpace(Request)
        && Request.EnumerateRunes().Count() <= 10000 && Models.Any(m => S(m, "id") == ModelId)
        && (Legacy || (Skills.Length > 0 && (AutoSources || sources.Count > 0)));
    public void Configure(JsonObject[] skills, JsonObject[] models)
    {
        if (Busy) return;
        Skills = skills.Select(Clone).ToArray(); Models = models.Select(Clone).ToArray();
        if (RequestId == null && !Models.Any(m => S(m, "id") == ModelId)) ModelId = S(Models.FirstOrDefault(m => m["active"]?.GetValue<bool>() == true) ?? Models.FirstOrDefault(), "id");
    }
    public void Show(bool open) { DialogOpen = open; Notify(); }
    private void Edit(Action change)
    { if (Busy || Closed) return; change(); Result = null; RequestId = null; ClearFailure(); Notify(); }
    public void SetRequest(string value) { if (Request != value) Edit(() => Request = value); }
    public void SetModel(string value) { if (ModelId != value) Edit(() => ModelId = value); }
    public void SetAutomatic(bool value) { if (AutoSources != value) Edit(() => AutoSources = value); }
    public void SetSource(string id, bool selected)
    {
        if (selected == sources.Contains(id) || (selected && (sources.Count >= 8 || !Skills.Any(s => S(s, "id") == id)))) return;
        Edit(() => { if (selected) sources.Add(id); else sources.Remove(id); });
    }
    public void SetLegacy(string? templateId, string? workflowId = null, string? request = null)
    { Edit(() => { LegacyTemplateId = templateId; LegacyWorkflowId = templateId == null ? workflowId : null; if (request != null) Request = request; }); }
    public Task<bool> GenerateAsync(INativeSettingsClient client, NativeLocalization locale)
    {
        if (Busy || Closed) return Task.FromResult(false);
        if (!Ready) { Fail(locale.Text("workflow_studio.portfolio.validation")); return Task.FromResult(false); }
        var request = new JsonObject { ["request"] = Request.Trim(), ["model_id"] = ModelId,
            ["source_skill_ids"] = new JsonArray((AutoSources || Legacy ? [] : sources.ToArray()).Select(id => (JsonNode?)JsonValue.Create(id)).ToArray()),
            ["legacy_template_id"] = LegacyTemplateId, ["legacy_workflow_id"] = LegacyWorkflowId };
        RequestId = Guid.NewGuid().ToString(); StartedAt = DateTimeOffset.UtcNow; Result = null; Stage = "preparing";
        var id = RequestId;
        var args = new JsonObject { ["request"] = request, ["conversionId"] = RequestId, ["expectedProjectId"] = ProjectId };
        return RunAsync(async () =>
        {
            using var cancellation = new CancellationTokenSource(); progressCancellation = cancellation;
            var progress = PollProgressAsync(client, id, cancellation.Token);
            try { return await client.InvokeAsync("plan_skill_portfolio", args, ProjectId); }
            finally
            {
                elapsed = DateTimeOffset.UtcNow - StartedAt.Value;
                cancellation.Cancel(); await progress;
                if (ReferenceEquals(progressCancellation, cancellation)) progressCancellation = null;
            }
        }, value =>
        {
            if (value is not JsonObject draft || draft["plan"] is not JsonObject || draft["proposal"] is not JsonObject proposal
                || proposal["tasks"] is not JsonArray { Count: > 0 } tasks || tasks.Any(t => t is not JsonObject)
                || string.IsNullOrWhiteSpace(S(proposal, "goal")) || NativeWorkflowGraph.Validate(tasks.OfType<JsonObject>(), locale) != null)
                throw new InvalidDataException(locale.Choose("转换未返回有效工作流草稿。", "Conversion did not return a valid workflow draft."));
            Result = Clone(draft);
        });
    }
    private async Task PollProgressAsync(INativeSettingsClient client, string id, CancellationToken cancellation)
    {
        try
        {
            var capabilities = await ReadAsync(client, "native_settings_capabilities", new(), cancellation);
            if (capabilities is not JsonObject || capabilities["commands"] is not JsonArray commands
                || !commands.Any(command => command?.ToString() == "native_workflow_conversion_progress")) return;
            while (!cancellation.IsCancellationRequested && Busy && !Closed && RequestId == id)
            {
                await RefreshProgressAsync(client, id, cancellation);
                await Task.Delay(TimeSpan.FromSeconds(1), cancellation);
            }
        }
        // Progress is advisory. Losing this read channel must not fail or replay the conversion.
        catch (Exception) { }
    }
    public async Task RefreshProgressAsync(INativeSettingsClient client, string id, CancellationToken cancellation = default)
    {
        if (Closed || !Busy || RequestId != id || cancellation.IsCancellationRequested) return;
        var value = await ReadAsync(client, "native_workflow_conversion_progress", new() { ["conversionId"] = id }, cancellation);
        if (Closed || !Busy || RequestId != id || cancellation.IsCancellationRequested || value is not JsonObject progress
            || S(progress, "conversion_id") != id) return;
        var stage = S(progress, "stage") switch
        {
            "preparing" => "preparing", "selecting_sources" => "selecting", "reading_sources" => "reading",
            "generating" => "generating", "validating" => "validating", "repairing" => "repairing", "saving" => "saving", _ => null
        };
        if (stage != null && stage != Stage) { Stage = stage; Notify(); }
    }
    private async Task<JsonNode?> ReadAsync(INativeSettingsClient client, string command, JsonObject args, CancellationToken cancellation)
    {
        using var deadline = CancellationTokenSource.CreateLinkedTokenSource(cancellation);
        deadline.CancelAfter(TimeSpan.FromSeconds(10));
        return await client.InvokeAsync(command, args, ProjectId, deadline.Token).WaitAsync(deadline.Token);
    }
    public override void Dispose() { base.Dispose(); progressCancellation?.Cancel(); }
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;
    public void Consume() { if (Busy) return; Result = null; RequestId = null; DialogOpen = false; ClearFailure(); Notify(); }
    public static string ShortName(string goal)
    {
        var line = goal.Trim().Split('\n')[0].Trim(); var runes = line.EnumerateRunes().ToArray();
        return runes.Length <= 60 ? line : string.Concat(runes.Take(59).Select(r => r.ToString())) + "…";
    }
    public static string[] SourceIds(JsonObject result) => (result["plan"]?["tasks"] as JsonArray)?.OfType<JsonObject>()
        .SelectMany(t => (t["skill_ids"] as JsonArray)?.Select(v => v?.GetValue<string>() ?? "") ?? [])
        .Where(s => s.Length > 0).Distinct(StringComparer.Ordinal).Order(StringComparer.Ordinal).ToArray() ?? [];
    private static JsonObject Clone(JsonObject value) => (JsonObject)value.DeepClone();
    private static string S(JsonNode? value, string key) => NativeWorkflowStudioModel.Text(value, key);
}
