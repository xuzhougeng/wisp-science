using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record JourneyDay(DateOnly Day, JourneyEntry[] Entries);

public sealed class WorkspaceJourneyModel(INativeJourneyClient client, string projectId) : IDisposable
{
    public event Action? Changed;
    public bool Closed { get; private set; }
    public bool Busy { get; private set; }
    public string? Error { get; private set; }
    public DateTime Month { get; private set; } = new(DateTime.Today.Year, DateTime.Today.Month, 1);
    public DateTime? FocusDay { get; private set; }
    public DateTime SelectedDay { get; private set; } = DateTime.Today;
    public HashSet<DateOnly> MarkedDays { get; private set; } = [];
    public JourneyGraph? Graph { get; private set; }
    public bool GraphBusy { get; private set; }
    public string? GraphError { get; private set; }
    public bool NoteOpen { get; private set; }
    public DateTime NoteDate { get; set; } = DateTime.Today;
    public string NoteCategory { get; set; } = "progress";
    public string NoteTitle { get; set; } = "";
    public string NoteBody { get; set; } = "";
    public bool Saving { get; private set; }
    public string? SaveError { get; private set; }
    public bool CanSaveNote => !Closed && !Saving && NoteTitle.Trim().Length is > 0 and <= 200 && NoteBody.Length <= 10000;
    private int readGeneration, graphGeneration, noteGeneration;
    public void Notify() { if (!Closed) Changed?.Invoke(); }
    public JourneyPage? Page { get; private set; }
    public string Query { get; set; } = "";
    public string Kind { get; set; } = "";
    public JourneyEntry? Selected { get; private set; }
    public JourneyArtifact? Artifact { get; private set; }
    public NativeRun? Run { get; private set; }
    public string? SourceRunId => Artifact != null ? Artifact.Source.RunId : Selected?.RunId;
    public string? SourceError { get; private set; }
    public bool SourceLoading { get; private set; }
    public bool HasParentSource => sources.Count > 0;
    private readonly Stack<JourneyArtifact> sources = new();
    private int sourceGeneration;

    public async Task<bool> LoadAsync(long from, long until)
    {
        if (Closed) return false;
        if (from >= until) { Error = "Invalid date range"; Notify(); return false; }
        var current = ++readGeneration; Busy = true; Error = null; ClearSelection(); Notify();
        try
        {
            var value = await client.ReadAsync(projectId, from, until);
            if (Closed || current != readGeneration) return false;
            Page = value;
            if (FocusDay == null) MarkedDays = value.Entries.Select(e => DateOnly.FromDateTime(DateTimeOffset.FromUnixTimeSeconds(e.OccurredAt).LocalDateTime)).ToHashSet();
            return true;
        }
        catch (Exception ex) { if (!Closed && current == readGeneration) Error = ex.Message; return false; }
        finally { if (!Closed && current == readGeneration) { Busy = false; Notify(); } }
    }

    public Task<bool> ReloadAsync() => LoadAsync(WorkspaceCalendarModel.Unix(FocusDay ?? Month), WorkspaceCalendarModel.Unix(FocusDay?.AddDays(1) ?? Month.AddMonths(1)));
    public Task<bool> OpenAsync(DateTime? day)
    {
        var date = day?.Date ?? DateTime.Today;
        Month = new(date.Year, date.Month, 1); SelectedDay = date; FocusDay = day?.Date;
        return ReloadAsync();
    }
    public Task<bool> ShiftMonthAsync(int delta)
    { Month = Month.AddMonths(delta); FocusDay = null; Query = ""; Page = null; MarkedDays.Clear(); return ReloadAsync(); }
    public Task<bool> SelectDayAsync(DateTime day)
    { SelectedDay = day.Date; FocusDay = day.Date; Query = ""; Page = null; return ReloadAsync(); }
    public Task<bool> FullMonthAsync() { FocusDay = null; Page = null; return ReloadAsync(); }
    public Task<bool> TodayAsync()
    { Month = new(DateTime.Today.Year, DateTime.Today.Month, 1); FocusDay = null; SelectedDay = DateTime.Today; Query = ""; Page = null; MarkedDays.Clear(); return ReloadAsync(); }
    public async Task LoadGraphAsync()
    {
        if (Closed) return;
        var current = ++graphGeneration; GraphBusy = true; GraphError = null; Notify();
        try { var graph = await client.GraphAsync(projectId); if (!Closed && current == graphGeneration) Graph = graph; }
        catch (Exception ex) { if (!Closed && current == graphGeneration) GraphError = ex.Message; }
        finally { if (!Closed && current == graphGeneration) { GraphBusy = false; Notify(); } }
    }
    public void OpenNote()
    {
        if (Closed || Saving) return;
        noteGeneration++; NoteOpen = true; NoteDate = SelectedDay; NoteTitle = NoteBody = ""; NoteCategory = "progress"; SaveError = null; Notify();
    }
    public void CloseNote() { noteGeneration++; NoteOpen = false; SaveError = null; Notify(); }
    public async Task<bool> SaveNoteAsync()
    {
        if (!NoteOpen || !CanSaveNote) return false;
        var current = noteGeneration; var date = NoteDate.Date;
        var input = new JourneyNote(NoteTitle.Trim(), NoteBody, NoteCategory, WorkspaceCalendarModel.Unix(date));
        Saving = true; SaveError = null; Notify(); var saved = false;
        try
        {
            var id = await client.AddAsync(projectId, input);
            if (string.IsNullOrWhiteSpace(id)) throw new InvalidDataException("Missing research entry identity");
            if (Closed || current != noteGeneration) return false;
            NoteOpen = false; NoteTitle = NoteBody = ""; saved = true;
        }
        catch (Exception ex) { if (!Closed && current == noteGeneration) SaveError = ex.Message; }
        finally { if (!Closed) { Saving = false; Notify(); } }
        if (saved) { await OpenAsync(date); await LoadGraphAsync(); }
        return saved;
    }
    public async Task<bool> SaveRecapAsync(JourneyRecapEdit edit)
    {
        if (Closed || Saving) return false;
        Saving = true; SaveError = null; Notify();
        try
        {
            var recap = await client.EditRecapAsync(projectId, edit);
            if (Closed) return false;
            if (recap.Id != edit.Id) throw new InvalidDataException("Recap identity mismatch");
            if (Page != null) Page = Page with { Recaps = Page.Recaps.Select(r => r.Id == recap.Id ? recap : r).ToArray() };
            return true;
        }
        catch (Exception ex) { if (!Closed) SaveError = ex.Message; return false; }
        finally { if (!Closed) { Saving = false; Notify(); } }
    }

    public JourneyDay[] Days(TimeZoneInfo? zone = null) => (Page?.Entries ?? [])
        .Where(row => (Kind.Length == 0 || row.Kind == Kind) && (row.Title.Contains(Query.Trim(), StringComparison.CurrentCultureIgnoreCase)
            || row.Summary.Contains(Query.Trim(), StringComparison.CurrentCultureIgnoreCase)))
        .OrderByDescending(row => row.OccurredAt).ThenBy(row => row.Id, StringComparer.Ordinal)
        .GroupBy(row => DateOnly.FromDateTime(TimeZoneInfo.ConvertTime(DateTimeOffset.FromUnixTimeSeconds(row.OccurredAt), zone ?? TimeZoneInfo.Local).DateTime))
        .Select(group => new JourneyDay(group.Key, group.ToArray())).ToArray();

    public async Task SelectAsync(JourneyEntry entry)
    {
        if (Closed) return;
        ClearSelection(); Selected = entry; Notify();
        if (entry.Kind == "artifact" && !entry.SourceDiscarded && !string.IsNullOrEmpty(entry.SourceId)) await OpenArtifactAsync(entry.SourceId);
    }

    public async Task OpenArtifactAsync(string version)
    {
        if (Closed || Selected == null || SourceLoading || string.IsNullOrWhiteSpace(version)) return;
        var current = ++sourceGeneration;
        var previous = Artifact;
        SourceLoading = true; SourceError = null; Notify();
        try
        {
            var result = await client.ArtifactAsync(projectId, version);
            if (Closed || current != sourceGeneration) return;
            if (result.VersionId != version) throw new InvalidDataException("产物版本不匹配。");
            if (previous != null) sources.Push(previous);
            Artifact = result;
        }
        catch (Exception ex) { if (!Closed && current == sourceGeneration) SourceError = ex.Message; }
        finally { if (!Closed && current == sourceGeneration) { SourceLoading = false; Notify(); } }
    }

    public async Task OpenRunAsync()
    {
        if (Closed || SourceLoading || SourceRunId is not { Length: > 0 } id) return;
        var current = ++sourceGeneration;
        SourceLoading = true; SourceError = null; Notify();
        try
        {
            var result = await client.RunAsync(projectId, id);
            if (Closed || current != sourceGeneration) return;
            if (result.Id != id) throw new InvalidDataException("运行来源不匹配。");
            Run = result;
        }
        catch (Exception ex) { if (!Closed && current == sourceGeneration) SourceError = ex.Message; }
        finally { if (!Closed && current == sourceGeneration) { SourceLoading = false; Notify(); } }
    }

    public void Back()
    {
        sourceGeneration++; SourceLoading = false; SourceError = null;
        if (Run != null) Run = null;
        else if (sources.TryPop(out var previous)) Artifact = previous;
        else ClearSelection();
        Notify();
    }
    private void ClearSelection() { sourceGeneration++; Selected = null; Artifact = null; Run = null; SourceLoading = false; SourceError = null; sources.Clear(); }
    public void Dispose() { Closed = true; readGeneration++; graphGeneration++; noteGeneration++; Busy = GraphBusy = false; ClearSelection(); }
    public static string KindLabel(string kind) => kind switch
    {
        "artifact" => "产物", "run" => "运行", "session" => "会话", "archive" => "归档", "finding" => "发现",
        "decision" => "决策", "progress" => "进展", "next" => "下一步", "data" => "数据", "paper" => "文献",
        _ => string.IsNullOrEmpty(kind) ? "记录" : kind
    };
}
