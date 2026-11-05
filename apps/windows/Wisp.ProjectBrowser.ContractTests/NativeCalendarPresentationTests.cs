using System.Text.Json;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeCalendarPresentationTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var february = NativeCalendarPresentation.Cells(new(2024, 2, 13));
        Check(february.Count == 35 && february.Take(3).All(d => d == null) && february[3] == new DateTime(2024, 2, 1)
            && february.Count(d => d.HasValue) == 29 && february.Skip(32).All(d => d == null), "leap months fill Monday-first weeks with blank edges");
        var sunday = NativeCalendarPresentation.Cells(new(2026, 11, 1));
        Check(sunday.Count == 42 && sunday.Take(6).All(d => d == null) && sunday[6]?.Day == 1, "Sunday starts in the seventh column, not the first");
        Check(NativeCalendarPresentation.SelectedDay(new(2026, 10, 1), new(2026, 10, 30)) == new DateTime(2026, 10, 30)
            && NativeCalendarPresentation.SelectedDay(new(2026, 9, 1), new(2026, 10, 30)) == new DateTime(2026, 9, 1), "returning to the current month selects today");
        Check(NativeCalendarLayout.ForViewport(1707, 980).Desktop && !NativeCalendarLayout.ForViewport(1707, 650).Desktop
            && NativeCalendarLayout.ForViewport(1103, 763).DetailsWidth == 0 && NativeCalendarLayout.ForViewport(650, 700).HorizontalProjects
            && !NativeCalendarLayout.ForViewport(651, 700).HorizontalProjects, "short and narrow windows retain page scrolling and responsive project navigation");
        Check(NativeCalendarPresentation.ColorIndex("a") == 1 && NativeCalendarPresentation.ColorIndex("b") == 2, "project colors use the WebView's stable byte hash");
        var run = new CalendarEntry("run:r1", "run", "A run", 100, "succeeded", false) { SourceId = "r1" };
        var output = new CalendarEntry("artifact:o1", "artifact", "A result", 101, "recorded", false) { RunId = "r1", VersionNumber = 2 };
        var orphan = output with { Id = "artifact:o2", RunId = "other" };
        var folded = NativeCalendarPresentation.FoldOutputs([run, output, orphan]);
        Check(folded.Entries.Select(e => e.Id).SequenceEqual(["artifact:o2", "run:r1"]) && folded.Outputs["r1"] == 1,
            "only artifacts with a visible producing run fold into that run; orphan versions remain readable");
        var history = JsonSerializer.Deserialize<CalendarHistory>("""{"entries":[{"id":"r","kind":"run","title":"run","occurred_at":100,"status":"succeeded","manual":false,"source_id":"r1","run_id":"r1","version_number":3}],"truncated":true,"recaps":[{"status":"confirmed","headline":"Reviewed","done":[{"text":"Result recorded","refs":[0]}]}]}""")!;
        Check(history.Entries[0].SourceId == "r1" && history.Entries[0].RunId == "r1" && history.Entries[0].VersionNumber == 3
            && history.Recaps.Single().Done.Single().Text == "Result recorded", "existing shared DTO fields survive native projection");
        var old = JsonSerializer.Deserialize<CalendarHistory>("""{"entries":[],"truncated":false}""")!;
        Check(old.Recaps.Count == 0, "older hosts may omit recaps");

        var client = new CalendarFake(); var privacy = new PrivacyFake();
        var project = new ProjectSummary("p", "Project", "", "C:/research", false, 0, 0, 0, 0, 0, false, null);
        using var model = new WorkspaceCalendarModel(client, privacy, [project]);
        await model.OpenAsync(); await model.ShiftMonthAsync(-1); await model.ShiftMonthAsync(1);
        Check(model.Day == DateTime.Today, "previous and next return to today's selected day");
        await model.SelectDayAsync(model.Month.AddDays(2)); await model.TodayAsync(); Check(model.Day == DateTime.Today, "Today returns selection as well as the visible month");
        Check(!model.Filter([new CalendarProject("unexpected", new([], false), null)]).Any(), "unlisted project responses cannot appear in the calendar");
        var pending = new TaskCompletionSource<IReadOnlyList<CalendarProject>>(); client.Pending = pending;
        var stale = model.SelectDayAsync(model.Month.AddDays(4));
        Check(model.Busy && model.MonthRows.Count == 0 && model.DayRows.Count == 0, "changing day hides stale records while reads are pending");
        client.Pending = null; var newest = model.Month.AddDays(7); await model.SelectDayAsync(newest);
        pending.SetResult([new CalendarProject("p", new([run], false), null)]); await stale;
        Check(model.Day == newest && !model.Busy && model.DayRows.Count == 0, "late month reply cannot overwrite newer selection or start its day request");
        client.Fail = true; await model.SelectDayAsync(model.Month.AddDays(8));
        Check(model.Error != null && model.DayRows.Count == 0 && !model.Busy, "failed reads show an error rather than stale successful records");
        client.Fail = false; await model.OpenAsync(); Check(model.Error == null && model.PrivacyReady, "explicit retry rechecks privacy and restores reads");
        model.Dispose(); var calls = client.Calls; await model.TodayAsync(); Check(client.Calls == calls, "closed calendar navigation performs no reads");
    }
    private sealed class PrivacyFake : INativePrivacyClient
    { public Task<PrivacyMode> GetAsync(CancellationToken cancellationToken = default) => Task.FromResult(new PrivacyMode(false, [])); }
    private sealed class CalendarFake : INativeCalendarClient
    {
        public TaskCompletionSource<IReadOnlyList<CalendarProject>>? Pending; public bool Fail; public int Calls;
        public Task<IReadOnlyList<CalendarProject>> ReadAsync(IReadOnlyList<string> ids, long from, long until, CancellationToken cancellationToken = default)
        { Calls++; if (Fail) throw new IOException("unavailable"); return Pending?.Task ?? Task.FromResult<IReadOnlyList<CalendarProject>>([]); }
    }
}
