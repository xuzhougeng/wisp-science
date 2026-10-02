using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Fail-closed privacy gate. Navigation never invalidates an outstanding privacy read.</summary>
public sealed class WorkspaceCalendarModel(INativeCalendarClient client, INativePrivacyClient privacy, IReadOnlyList<ProjectSummary> projects) : IDisposable
{
    public event Action? Changed;
    public DateTime Month { get; private set; } = new(DateTime.Today.Year, DateTime.Today.Month, 1);
    public DateTime Day { get; private set; } = DateTime.Today;
    public bool PrivacyReady { get; private set; }
    public bool Busy { get; private set; }
    public string? Error { get; private set; }
    public string? ProjectFilter { get; set; }
    public IReadOnlyList<ProjectSummary> VisibleProjects { get; private set; } = [];
    public IReadOnlyList<CalendarProject> MonthRows { get; private set; } = [];
    public IReadOnlyList<CalendarProject> DayRows { get; private set; } = [];
    private int privacyGeneration, readGeneration;
    private bool closed, privacyLoading;
    public async Task OpenAsync()
    {
        if (closed || privacyLoading) return;
        var current = ++privacyGeneration;
        readGeneration++; privacyLoading = true; PrivacyReady = false; Busy = true;
        Error = null; MonthRows = []; DayRows = []; VisibleProjects = []; Changed?.Invoke();
        try
        {
            var mode = await privacy.GetAsync();
            if (closed || current != privacyGeneration) return;
            VisibleProjects = projects.Where(p => !mode.Active || !mode.ProjectIds.Contains(p.Id)).DistinctBy(p => p.Id).ToArray();
            if (!VisibleProjects.Any(p => p.Id == ProjectFilter)) ProjectFilter = null;
            PrivacyReady = true;
        }
        catch (Exception ex)
        {
            if (!closed && current == privacyGeneration) Error = "隐私模式未能确认读取；不会读取日历或自动重试。\n" + ex.Message;
        }
        finally
        {
            if (!closed && current == privacyGeneration) { privacyLoading = false; Busy = false; Changed?.Invoke(); }
        }
        if (!closed && current == privacyGeneration && PrivacyReady) await ReadAsync();
    }
    public Task ShiftMonthAsync(int delta)
    {
        Month = Month.AddMonths(delta); Day = Month;
        return ReadAsync();
    }
    public Task SelectDayAsync(DateTime day)
    {
        Day = day.Date; Month = new(day.Year, day.Month, 1);
        return ReadAsync();
    }
    public static long Unix(DateTime local) => new DateTimeOffset(DateTime.SpecifyKind(local, DateTimeKind.Local)).ToUnixTimeSeconds();
    private async Task ReadAsync()
    {
        if (closed || !PrivacyReady) return;
        var current = ++readGeneration;
        var privacyCurrent = privacyGeneration;
        var month = Month; var day = Day;
        var ids = VisibleProjects.Select(p => p.Id).ToArray();
        Busy = true; Error = null; Changed?.Invoke();
        try
        {
            var monthRows = ids.Length == 0 ? [] : await client.ReadAsync(ids, Unix(month), Unix(month.AddMonths(1)));
            if (closed || current != readGeneration || privacyCurrent != privacyGeneration) return;
            var dayRows = ids.Length == 0 ? [] : await client.ReadAsync(ids, Unix(day), Unix(day.AddDays(1)));
            if (closed || current != readGeneration || privacyCurrent != privacyGeneration) return;
            MonthRows = monthRows; DayRows = dayRows;
        }
        catch (Exception ex) { if (!closed && current == readGeneration) Error = ex.Message; }
        finally { if (!closed && current == readGeneration) { Busy = false; Changed?.Invoke(); } }
    }
    public IEnumerable<CalendarProject> Filter(IEnumerable<CalendarProject> rows) => rows.Where(p => ProjectFilter == null || p.ProjectId == ProjectFilter);
    public void Dispose() { closed = true; privacyGeneration++; readGeneration++; }
}
