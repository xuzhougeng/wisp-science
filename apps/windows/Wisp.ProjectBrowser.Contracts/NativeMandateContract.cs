namespace Wisp.ProjectBrowser.Contracts;

// Projections of wisp_dto::mandate. The shared Rust contract and the existing
// mandate handlers own persistence, normalization, scheduling and execution.
public sealed record AssistantMandateKpi(string Name, string Definition, string Target, string Period, string? Current);
public sealed record AssistantMandateConstraints(string Autonomous, string ReviewFirst, string AskForHelp, bool ReviewMutations,
    long MinIntervalSecs, long MaxIntervalSecs, int MaxRoundsPerDay, string[] Notes);
public sealed record AssistantMandate(string Id, string ProjectId, string? FrameId, string Name, string Goal,
    AssistantMandateKpi[] Kpis, AssistantMandateConstraints Constraints, long? EndsAt, long IntervalSecs,
    long ReportIntervalSecs, long? NextReportAt, string Status, long NextRunAt, long? LastRunAt, string? WaitRunId, long CreatedAt, long UpdatedAt);
public sealed record AssistantMandateRound(string Id, string MandateId, long Seq, string Done, string Blockers, string NextStep, string Source, long CreatedAt);
public sealed record AssistantMandateRequest(string Id, string MandateId, string Kind, string What, string Why, string ThenWhat, string Status, long CreatedAt);
public sealed record AssistantMandateReport(string Id, string MandateId, long PeriodFrom, long PeriodUntil, int Rounds, JourneyRecap Body);
public sealed record AssistantMandateOverview(AssistantMandate Mandate, AssistantMandateRound? LastRound, AssistantMandateRequest? Request, AssistantMandateReport[] Reports);

internal static class NativeMandateContract
{
    internal static void Validate(AssistantMandateOverview[]? rows, AssistantProject[] projects)
    {
        if (rows == null) throw new InvalidDataException("Missing research mandates. Use the matching native desktop host.");
        var ids = new HashSet<string>(StringComparer.Ordinal);
        foreach (var row in rows)
        {
            var m = row?.Mandate;
            if (m == null || string.IsNullOrWhiteSpace(m.Id) || !ids.Add(m.Id) || !projects.Any(p => p.Id == m.ProjectId)
                || m.Status is not ("active" or "waiting" or "paused" or "done") || m.Name == null || m.Goal == null
                || m.Kpis == null || m.Kpis.Any(k => k == null || k.Name == null || k.Definition == null || k.Target == null || k.Period == null)
                || m.Constraints?.Notes == null || m.Constraints.Notes.Any(n => n == null)
                || m.Constraints.Autonomous == null || m.Constraints.ReviewFirst == null || m.Constraints.AskForHelp == null
                || row!.Reports == null || row.Reports.Any(r => r == null || r.MandateId != m.Id || !ValidBody(r.Body))
                || row.LastRound is { } round && (round.MandateId != m.Id || round.Done == null || round.Blockers == null || round.NextStep == null)
                || row.Request is { } request && (request.MandateId != m.Id || request.Status != "open" || string.IsNullOrWhiteSpace(request.Id)
                    || request.What == null || request.Why == null || request.ThenWhat == null))
                throw new InvalidDataException("Invalid research mandate snapshot or project visibility.");
        }
    }
    private static bool ValidBody(JourneyRecap? body) => body != null && body.Headline != null && body.Model != null
        && body.Sources != null && body.Sources.All(s => s != null && s.Title != null)
        && new[] { body.Done, body.Findings, body.Issues, body.Next }.All(items => items != null && items.All(i => i != null && i.Text != null && i.Refs != null));
}
