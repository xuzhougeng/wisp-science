using System.Globalization;
using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class NativeMandateKpiDraft
{
    public string Name { get; set; } = "";
    public string Definition { get; set; } = "";
    public string Target { get; set; } = "";
    public string Period { get; set; } = "";
    public string? Current { get; set; }
    public AssistantMandateKpi Value() => new(Name, Definition, Target, Period, Current);
}

public sealed class NativeMandateDraft
{
    public string? Id { get; set; }
    public string ProjectId { get; set; } = "";
    public string Name { get; set; } = "";
    public string Goal { get; set; } = "";
    public string Until { get; set; } = "";
    public List<NativeMandateKpiDraft> Kpis { get; set; } = [new()];
    public string Autonomous { get; set; } = "";
    public string ReviewFirst { get; set; } = "";
    public string AskForHelp { get; set; } = "";
    public string Notes { get; set; } = "";
    public bool ReviewMutations { get; set; } = true;
    public int Hours { get; set; } = 24;
    public int ReportDays { get; set; } = 7;
    public int MinMinutes { get; set; } = 15;
    public int MaxDays { get; set; } = 7;
    public int MaxRounds { get; set; } = 6;
    public static NativeMandateDraft From(AssistantMandate m, TimeZoneInfo zone) => new() {
        Id = m.Id, ProjectId = m.ProjectId, Name = m.Name, Goal = m.Goal,
        Until = m.EndsAt is { } at ? TimeZoneInfo.ConvertTime(DateTimeOffset.FromUnixTimeSeconds(at), zone).ToString("yyyy-MM-dd") : "",
        Kpis = m.Kpis.Select(k => new NativeMandateKpiDraft { Name = k.Name, Definition = k.Definition, Target = k.Target, Period = k.Period, Current = k.Current }).ToList(),
        Autonomous = m.Constraints.Autonomous, ReviewFirst = m.Constraints.ReviewFirst, AskForHelp = m.Constraints.AskForHelp,
        Notes = string.Join("\n", m.Constraints.Notes), ReviewMutations = m.Constraints.ReviewMutations,
        Hours = (int)Math.Max(1, m.IntervalSecs / 3600), ReportDays = (int)Math.Max(1, m.ReportIntervalSecs / 86400),
        MinMinutes = (int)Math.Max(1, m.Constraints.MinIntervalSecs / 60), MaxDays = (int)Math.Max(1, m.Constraints.MaxIntervalSecs / 86400), MaxRounds = m.Constraints.MaxRoundsPerDay };
    public long? EndAt(TimeZoneInfo zone)
    {
        if (string.IsNullOrWhiteSpace(Until)) return null;
        if (!DateTime.TryParseExact(Until, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out var date))
            throw new FormatException("Choose an end date in the future, or leave it empty.");
        var end = DateTime.SpecifyKind(date.AddDays(1).AddSeconds(-1), DateTimeKind.Unspecified);
        // The deadline is the selected local day, independent of UTC or DST.
        var offset = zone.IsAmbiguousTime(end) ? zone.GetAmbiguousTimeOffsets(end).Max() : zone.GetUtcOffset(end);
        return new DateTimeOffset(end, offset).ToUnixTimeSeconds();
    }
    public JsonObject Operation(DateTimeOffset now, TimeZoneInfo zone)
    {
        var ends = EndAt(zone);
        if (ends <= now.ToUnixTimeSeconds()) throw new FormatException("Choose an end date in the future, or leave it empty.");
        var constraints = new AssistantMandateConstraints(Autonomous, ReviewFirst, AskForHelp, ReviewMutations,
            Math.Max(1L, MinMinutes) * 60, Math.Max(1L, MaxDays) * 86400, Math.Max(1, MaxRounds),
            Notes.Split('\n').Select(n => n.Trim()).Where(n => n.Length > 0).ToArray());
        var draft = new JsonObject { ["project_id"] = ProjectId, ["name"] = Name, ["goal"] = Goal,
            ["kpis"] = JsonSerializer.SerializeToNode(Kpis.Select(k => k.Value()).ToArray(), ConversationSnapshot.JsonOptions),
            ["constraints"] = JsonSerializer.SerializeToNode(constraints, ConversationSnapshot.JsonOptions), ["ends_at"] = ends,
            ["interval_secs"] = Math.Max(1L, Hours) * 3600, ["report_interval_secs"] = Math.Max(1L, ReportDays) * 86400,
            ["start_at"] = Id == null ? now.ToUnixTimeSeconds() : null };
        var operation = new JsonObject { ["action"] = Id == null ? "create_mandate" : "update_mandate", ["draft"] = draft };
        if (Id != null) operation["id"] = Id;
        return operation;
    }
}

public sealed partial class NativeAutomationModel
{
    public NativeMandateDraft? MandateDraft { get; private set; }
    public string? MandateError { get; private set; }
    public string? DeletingMandate { get; private set; }
    private readonly Dictionary<string, string> replies = new(StringComparer.Ordinal);
    private void ReconcileMandates(AssistantWorkspace snapshot)
    {
        if (MandateDraft != null && !snapshot.Projects.Any(p => p.Id == MandateDraft.ProjectId)) MandateDraft.ProjectId = "";
        if (!snapshot.Mandates!.Any(m => m.Mandate.Id == DeletingMandate)) DeletingMandate = null;
        var requests = snapshot.Mandates!.Where(m => m.Request != null).Select(m => m.Request!.Id).ToHashSet(StringComparer.Ordinal);
        foreach (var key in replies.Keys.Where(k => !requests.Contains(k)).ToArray()) replies.Remove(key);
    }
    public void BeginMandate(int? template = null, string? id = null)
    {
        if (!Ready) return;
        NativeMandateDraft draft;
        if (id != null) { var m = Snapshot!.Mandates!.FirstOrDefault(m => m.Mandate.Id == id); if (m == null) return; draft = NativeMandateDraft.From(m.Mandate, Zone); }
        else { draft = template is { } index ? NativeMandateTemplates.Create(index, Chinese, Now, Zone) : new(); draft.ProjectId = Snapshot!.Projects.FirstOrDefault()?.Id ?? ""; }
        Draft = null; FormError = null; Deleting = null; DeletingMandate = null; MandateError = null; MandateDraft = draft; Changed?.Invoke();
    }
    public void CancelMandate() { MandateDraft = null; MandateError = null; Changed?.Invoke(); }
    public void ChangeMandateKpi(int? remove = null)
    {
        if (!Ready || MandateDraft == null) return;
        if (remove is { } index) { if (index >= 0 && index < MandateDraft.Kpis.Count) MandateDraft.Kpis.RemoveAt(index); }
        else MandateDraft.Kpis.Add(new());
        Changed?.Invoke();
    }
    public async Task SaveMandateAsync()
    {
        if (!Ready || MandateDraft is not { } draft) return;
        if (!Snapshot!.Projects.Any(p => p.Id == draft.ProjectId) || string.IsNullOrWhiteSpace(draft.Goal)
            || draft.Id != null && !Snapshot.Mandates!.Any(m => m.Mandate.Id == draft.Id && m.Mandate.ProjectId == draft.ProjectId))
        { MandateError = L("请选择项目并填写目标。", "Choose a project and write a goal."); Changed?.Invoke(); return; }
        JsonObject operation;
        try { operation = draft.Operation(Now, Zone); }
        catch (Exception ex) when (ex is FormatException or ArgumentException) { MandateError = L("请选择将来的结束日期，或留空。", "Choose an end date in the future, or leave it empty."); Changed?.Invoke(); return; }
        await Request(async () => {
            Error = null; MandateError = null;
            try { await client.MutateAsync(operation, lifetime.Token); }
            catch (Exception ex) { if (!Closed && ReferenceEquals(MandateDraft, draft)) MandateError = ex.Message; return; }
            if (Closed) return;
            if (ReferenceEquals(MandateDraft, draft)) MandateDraft = null;
            await Read();
        });
    }
    public string ReplyText(string requestId) => replies.GetValueOrDefault(requestId, "");
    public void SetReply(string requestId, string text) { if (!Closed) replies[requestId] = text; }
    public Task ReplyMandateAsync(string id, string requestId)
    {
        if (!Ready || !Snapshot!.Mandates!.Any(m => m.Mandate.Id == id && m.Request?.Id == requestId) || string.IsNullOrWhiteSpace(ReplyText(requestId))) return Task.CompletedTask;
        return MandateActionAsync(id, "reply_mandate", text: ReplyText(requestId));
    }
    public async Task DeleteMandateAsync(string id)
    {
        if (!Ready || !Snapshot!.Mandates!.Any(m => m.Mandate.Id == id)) return;
        if (DeletingMandate != id) { Deleting = null; DeletingMandate = id; Changed?.Invoke(); return; }
        DeletingMandate = null; await MandateActionAsync(id, "delete_mandate");
    }
    public Task MandateActionAsync(string id, string action, string? status = null, string? text = null)
    {
        var row = Snapshot?.Mandates?.FirstOrDefault(m => m.Mandate.Id == id);
        if (!Ready || row == null || action is not ("set_mandate_status" or "run_mandate" or "report_mandate" or "reply_mandate" or "delete_mandate")) return Task.CompletedTask;
        if (row.Mandate.Status == "done" && action is "run_mandate" or "report_mandate" or "reply_mandate") return Task.CompletedTask;
        if (action == "set_mandate_status" && status is not ("active" or "paused" or "done")) return Task.CompletedTask;
        var operation = new JsonObject { ["action"] = action, ["id"] = id };
        if (status != null) operation["status"] = status;
        if (text != null) operation["text"] = text;
        return Request(async () => {
            Error = null; await client.MutateAsync(operation, lifetime.Token); if (Closed) return;
            if (action == "reply_mandate" && row.Request != null) replies.Remove(row.Request.Id);
            Notice = action switch {
                "run_mandate" => L("已开始一轮，在该项目的职责对话中运行。", "Started a round. It runs in the mandate's conversation in that project."),
                "report_mandate" => L("正在生成汇报，完成后显示在卡片上。", "Writing the report. It appears on the card when it is ready."),
                "reply_mandate" => L("已发送，职责在它的对话中继续。", "Sent. The mandate continues in its conversation."),
                "set_mandate_status" when status == "done" => L("已结束，正在生成收尾汇报；重新打开开关即可恢复。", "Closed. A final report is being written; switch it back on to reopen."), _ => null };
            await Read();
        });
    }
}
