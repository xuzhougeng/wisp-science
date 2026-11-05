using System.Globalization;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public enum NativeScheduleCadence { Daily, Weekly, Hourly }
public sealed class NativeScheduleDraft
{
    public string ProjectId { get; set; } = "";
    public string Name { get; set; } = "";
    public string Prompt { get; set; } = "";
    public NativeScheduleCadence Cadence { get; set; }
    public string Time { get; set; } = "09:00";
    public int Weekday { get; set; } = 1;
    public int Hours { get; set; } = 6;
    public JsonObject Operation(DateTimeOffset now, TimeZoneInfo zone)
    {
        if (string.IsNullOrWhiteSpace(ProjectId) || string.IsNullOrWhiteSpace(Prompt)) throw new InvalidOperationException("Choose a project and write a prompt.");
        long? start = Cadence == NativeScheduleCadence.Hourly ? null : NextSlot(Time, Cadence == NativeScheduleCadence.Weekly ? Weekday : null, now, zone);
        return new() { ["action"] = "create_schedule", ["project_id"] = ProjectId, ["name"] = Name, ["prompt"] = Prompt,
            ["interval_secs"] = Cadence == NativeScheduleCadence.Hourly ? Math.Clamp(Hours, 1, 168) * 3600L : Cadence == NativeScheduleCadence.Weekly ? 604800L : 86400L,
            ["start_at"] = start, ["session_id"] = null, ["skill"] = null };
    }
    public static bool ValidTime(string value) => TimeOnly.TryParseExact(value, "HH:mm", CultureInfo.InvariantCulture, DateTimeStyles.None, out _);
    public static long NextSlot(string value, int? weekday, DateTimeOffset now, TimeZoneInfo zone)
    {
        if (!TimeOnly.TryParseExact(value, "HH:mm", CultureInfo.InvariantCulture, DateTimeStyles.None, out var time) || weekday is < 0 or > 6)
            throw new InvalidOperationException("Use a 24-hour HH:MM time.");
        var today = TimeZoneInfo.ConvertTime(now, zone).Date;
        for (var day = 0; day <= 7; day++)
        {
            var candidate = DateTime.SpecifyKind(today.AddDays(day).Add(time.ToTimeSpan()), DateTimeKind.Unspecified);
            // JS Date shifts a nonexistent local wall time forward by the DST gap,
            // and chooses the earlier occurrence of an ambiguous wall time.
            if (zone.IsInvalidTime(candidate))
            {
                var before = candidate.AddHours(-1); var after = candidate.AddHours(1);
                while (zone.IsInvalidTime(before)) before = before.AddHours(-1);
                while (zone.IsInvalidTime(after)) after = after.AddHours(1);
                candidate += zone.GetUtcOffset(after) - zone.GetUtcOffset(before);
            }
            var offset = zone.IsAmbiguousTime(candidate) ? zone.GetAmbiguousTimeOffsets(candidate).Max() : zone.GetUtcOffset(candidate);
            var slot = new DateTimeOffset(candidate, offset);
            if (slot > now && (weekday == null || (int)candidate.DayOfWeek == weekday)) return slot.ToUnixTimeSeconds();
        }
        throw new InvalidOperationException("Use a 24-hour HH:MM time.");
    }
}

public sealed partial class NativeAutomationModel(NativeAssistantClient client, Func<DateTimeOffset>? clock = null, TimeZoneInfo? zone = null) : IDisposable
{
    public event Action? Changed;
    public AssistantWorkspace? Snapshot { get; private set; }
    public NativeScheduleDraft? Draft { get; private set; }
    public string? Error { get; private set; }
    public string? FormError { get; private set; }
    public string? Notice { get; private set; }
    public string? Deleting { get; private set; }
    public bool Busy { get; private set; }
    public bool Closed { get; private set; }
    public bool Ready => !Busy && !Closed && Snapshot != null;
    public bool Chinese { get; set; }
    private readonly CancellationTokenSource lifetime = new();
    private DateTimeOffset Now => clock?.Invoke() ?? DateTimeOffset.Now;
    private TimeZoneInfo Zone => zone ?? TimeZoneInfo.Local;
    private string L(string zh, string en) => Chinese ? zh : en;
    public async Task RefreshAsync()
    {
        if (Closed || Busy) return;
        await Request(async () => { Error = null; await Read(); });
    }
    private async Task Read()
    {
        try
        {
            var snapshot = await client.ReadAutomationAsync(TimeZoneInfo.ConvertTime(Now, Zone).Date, lifetime.Token);
            if (!Closed)
            {
                Snapshot = snapshot;
                if (Draft != null && !snapshot.Projects.Any(p => p.Id == Draft.ProjectId)) Draft.ProjectId = "";
                if (!snapshot.Schedules!.Any(s => s.Id == Deleting)) Deleting = null;
                ReconcileMandates(snapshot);
            }
        }
        catch { if (!Closed) Snapshot = null; throw; }
    }
    public void Begin(int? template = null)
    {
        if (!Ready) return;
        MandateDraft = null; MandateError = null; DeletingMandate = null;
        Draft = template is { } index ? NativeAutomationTemplates.Create(index, Chinese) : new();
        Draft.ProjectId = Snapshot!.Projects.FirstOrDefault()?.Id ?? "";
        FormError = null; Deleting = null; Changed?.Invoke();
    }
    public void CancelForm() { Draft = null; FormError = null; Changed?.Invoke(); }
    public bool Escape()
    {
        if (DeletingMandate != null) { DeletingMandate = null; Changed?.Invoke(); return true; }
        if (Deleting != null) { Deleting = null; Changed?.Invoke(); return true; }
        if (MandateDraft != null) { CancelMandate(); return true; }
        if (Draft == null) return false;
        CancelForm(); return true;
    }
    public async Task CreateAsync()
    {
        if (!Ready || Draft == null) return;
        var draft = Draft;
        if (!Snapshot!.Projects.Any(p => p.Id == draft.ProjectId) || string.IsNullOrWhiteSpace(draft.Prompt))
        { FormError = L("请选择项目并填写提示词。", "Choose a project and write a prompt."); Changed?.Invoke(); return; }
        JsonObject operation;
        try { operation = draft.Operation(Now, Zone); }
        catch { FormError = L("请输入 24 小时制时间 HH:MM。", "Use a 24-hour HH:MM time."); Changed?.Invoke(); return; }
        await Request(async () =>
        {
            FormError = null; Error = null;
            await client.MutateAsync(operation, lifetime.Token);
            if (Closed) return;
            if (ReferenceEquals(Draft, draft)) Draft = null;
            await Read();
        }, form: true);
    }
    public Task SetEnabledAsync(string id, bool enabled) => ScheduleAction(id, "set_enabled", enabled);
    public Task RunAsync(string id) => ScheduleAction(id, "run_now");
    public Task DeleteAsync(string id)
    {
        if (!Ready || !Snapshot!.Schedules!.Any(s => s.Id == id)) return Task.CompletedTask;
        if (Deleting != id) { Deleting = id; Changed?.Invoke(); return Task.CompletedTask; }
        Deleting = null; return ScheduleAction(id, "delete");
    }
    private async Task ScheduleAction(string id, string action, bool? enabled = null)
    {
        if (!Ready || Snapshot!.Schedules!.FirstOrDefault(s => s.Id == id) is not { } schedule) return;
        var operation = new JsonObject { ["action"] = action, ["id"] = id };
        if (enabled != null) operation["enabled"] = enabled;
        await Request(async () =>
        {
            Error = null; Notice = null; await client.MutateAsync(operation, lifetime.Token);
            if (Closed) return;
            if (action == "run_now") Notice = schedule.ReplacePreviousTurn
                ? L("已请求在绑定会话中检查。", "Requested a check in the bound conversation.")
                : L("已触发，结果会出现在该项目的新会话中。", "Started. The result appears in a new session of that project.");
            await Read();
        });
    }
    public async Task SaveDailyAsync(bool enabled, string time)
    {
        if (!Ready) return;
        if (!NativeScheduleDraft.ValidTime(time)) { Error = L("请输入 24 小时制时间 HH:MM。", "Use a 24-hour HH:MM time."); Changed?.Invoke(); return; }
        await MutateDaily(new() { ["action"] = "set_daily_recap", ["enabled"] = enabled, ["time"] = time });
    }
    public Task RunDailyAsync() => Ready && Snapshot!.DailyRecap?.Running != true ? MutateDaily(new() { ["action"] = "run_daily_recap" }) : Task.CompletedTask;
    private Task MutateDaily(JsonObject operation) => Request(async () =>
    {
        Error = null; var recap = await client.MutateDailyRecapAsync(operation, lifetime.Token);
        // The run command reports Running immediately, before the spawned job
        // necessarily acquires its flag. Keep that reply so polling will start.
        if (!Closed && Snapshot != null) Snapshot = Snapshot with { DailyRecap = recap };
    });
    public Task SetMemoryTidyAsync(bool enabled) => Ready ? MutateTidy(new() { ["action"] = "set_memory_tidy", ["enabled"] = enabled }) : Task.CompletedTask;
    public Task RunMemoryTidyAsync() => Ready && Snapshot!.MemoryTidy?.Running != true ? MutateTidy(new() { ["action"] = "run_memory_tidy" }) : Task.CompletedTask;
    public Task SettleMemoryAsync(string id, bool apply)
        => Ready && Snapshot!.MemoryTidy!.Proposals.Any(p => p.Id == id)
            ? MutateTidy(new() { ["action"] = apply ? "apply_memory_tidy" : "dismiss_memory_tidy", ["id"] = id }) : Task.CompletedTask;
    private Task MutateTidy(JsonObject operation) => Request(async () =>
    {
        Error = null;
        try
        {
            var tidy = await client.MutateMemoryTidyAsync(operation, lifetime.Token);
            if (!Closed && Snapshot != null) Snapshot = Snapshot with { MemoryTidy = tidy };
        }
        catch
        {
            // The host may already have removed an outdated proposal before
            // rejecting its application. Re-read once; never replay the write.
            if (!Closed) { try { await Read(); } catch { } }
            throw;
        }
    });
    private async Task Request(Func<Task> request, bool form = false)
    {
        Busy = true; Changed?.Invoke();
        try { await request(); }
        catch (Exception ex) { if (!Closed) { if (form && Draft != null) FormError = ex.Message; else Error = ex.Message; } }
        finally { Busy = false; if (!Closed) Changed?.Invoke(); }
    }
    public string Cadence(AssistantSchedule schedule)
    {
        var at = TimeZoneInfo.ConvertTime(DateTimeOffset.FromUnixTimeSeconds(schedule.NextRunAt), Zone);
        return schedule.IntervalSecs switch {
            86400 => L("每天", "Daily") + " " + at.ToString("HH:mm"),
            604800 => Weekday((int)at.DayOfWeek, Chinese) + " " + at.ToString("HH:mm"),
            var n when n % 3600 == 0 => L($"每 {n / 3600} 小时", $"Every {n / 3600} h"),
            var n => L($"每 {n / 60} 分钟", $"Every {n / 60} min") };
    }
    public static string Weekday(int day, bool chinese) => (chinese ? new[] { "周日", "周一", "周二", "周三", "周四", "周五", "周六" } : new[] { "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat" })[day % 7];
    public void Dispose() { if (Closed) return; Closed = true; lifetime.Cancel(); lifetime.Dispose(); Snapshot = null; Draft = null; MandateDraft = null; replies.Clear(); }
}

public static class NativeAutomationTemplates
{
    public static readonly string[] Icons = ["book", "gauge", "doc"];
    public static NativeScheduleDraft Create(int index, bool chinese) => index switch
    {
        0 => new() { Name = chinese ? "文献追踪" : "Literature watch", Cadence = NativeScheduleCadence.Weekly, Weekday = 1, Time = "09:00",
            Prompt = chinese ? "检索过去 7 天内与本项目研究问题相关的新文献，按相关性列出最多 10 篇，每篇用一句话说明与本项目的关系，并注明所用检索式。只读，不修改项目文件。" : "Search for papers published in the past 7 days that bear on this project's research question. List at most 10, most relevant first, each with one sentence on how it relates to this project, and state the search queries you used. Read-only: do not change project files." },
        1 => new() { Name = chinese ? "运行巡检" : "Run check", Time = "18:00",
            Prompt = chinese ? "检查本项目的所有运行：列出仍在运行、失败、失去连接或已完成但尚未收取产出的任务，说明可能原因和建议的下一步。只读，不重新提交或取消任务。" : "Check this project's runs. List runs that are still running, failed, lost their connection, or finished without their outputs harvested, with the likely reason and a suggested next step for each. Read-only: do not resubmit or cancel runs." },
        2 => new() { Name = chinese ? "研究周报" : "Weekly report", Cadence = NativeScheduleCadence.Weekly, Weekday = 5, Time = "17:00",
            Prompt = chinese ? "汇总本项目本周的研究历程（含每日回顾），整理成一份简短周报：本周完成、关键发现与决定、遇到的问题、下周计划，并注明相关运行和产出。只读。" : "Summarize this week's research journey for this project, including its daily recaps, into a short weekly report: completed work, key findings and decisions, problems met, and next week's plan. Cite runs and outputs by name. Read-only." },
        _ => throw new ArgumentOutOfRangeException(nameof(index))
    };
}
