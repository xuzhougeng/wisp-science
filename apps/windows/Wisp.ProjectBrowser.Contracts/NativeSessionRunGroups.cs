namespace Wisp.ProjectBrowser.Contracts;

/// <summary>Same conversation scope, active statuses and stable ordering as WebView session_run_groups.</summary>
public sealed record NativeSessionRunGroups(NativeRun[] Active, NativeRun[] Completed)
{
    public bool Empty => Active.Length == 0 && Completed.Length == 0;
    public static bool IsActive(string status) => status is "submitted" or "running" or "cancelling" or "paused";
    public bool RenderEquivalent(NativeSessionRunGroups other)
    {
        static bool Same(NativeRun[] before, NativeRun[] after)
        {
            if (before.Length != after.Length) return false;
            for (var i = 0; i < before.Length; i++)
                if (before[i] != after[i] && before[i] with { LastPolledAt = after[i].LastPolledAt } != after[i]) return false;
            return true;
        }
        return Same(Active, other.Active) && Same(Completed, other.Completed);
    }
    public static NativeSessionRunGroups For(IEnumerable<NativeRun> runs, string? session)
    {
        if (session == null) return new([], []);
        var scoped = runs.Where(run => run.FrameId == session).ToArray();
        return new(
            scoped.Where(run => IsActive(run.Status)).OrderByDescending(run => run.CreatedAt)
                .ThenByDescending(run => run.Id, StringComparer.Ordinal).ToArray(),
            scoped.Where(run => !IsActive(run.Status)).OrderByDescending(run => run.EndedAt ?? run.CreatedAt)
                .ThenByDescending(run => run.CreatedAt).ThenByDescending(run => run.Id, StringComparer.Ordinal).ToArray());
    }
}
