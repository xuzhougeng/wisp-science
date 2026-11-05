using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeRunReviewPrompt(string ProjectId, string SessionId, string RunId, string Title);
public sealed partial class WorkspaceConversationModel
{
    private readonly HashSet<(string Project, string Session, string Run)> hiddenRunCards = [], pendingRunReviews = [], checkingRunReviews = [], runMutations = [];
    private readonly Dictionary<(string Project, string Session, string Run), string> runActionErrors = [], previousRunStates = [];
    private readonly Dictionary<(string Project, string Session, string Run), string> runReadErrors = [];
    private readonly Dictionary<(string Project, string Session, string Run), NativeRun> observedRuns = [];
    private int historicalRunRead;
    public NativeRunReviewPrompt? RunReviewPrompt { get; private set; }
    private (string, string, string) RunKey(string id) => (projectId ?? "", sessionId ?? "", id);
    public bool IsRunHidden(string id) => hiddenRunCards.Contains(RunKey(id));
    public bool RunBusy(string id) => runMutations.Contains(RunKey(id));
    public string? RunError(string id) => runActionErrors.GetValueOrDefault(RunKey(id));
    public string? RunReadError(string id) => runReadErrors.GetValueOrDefault(RunKey(id));
    public static bool RunTerminal(string status) => status is "succeeded" or "failed" or "cancelled" or "timed_out" or "lost";
    public void DismissRunCard(string id)
    {
        if ((ShowingHistory ? History : Snapshot)?.RunCards?.FirstOrDefault(row => row.Id == id && row.FrameId == sessionId) is { } run
            && RunTerminal(run.Status)) { hiddenRunCards.Add(RunKey(id)); Notify(); }
    }
    public bool CanCancelRun(NativeRun run) => settings != null && ConnectionError == null && Snapshot is { ReadOnly: false }
        && !RunBusy(run.Id) && RunError(run.Id) == null && RunReadError(run.Id) == null && run.FrameId == sessionId
        && (ShowingHistory ? History : Snapshot) is { ReadOnly: false } page
        && page.RunCards?.Any(row => row.Id == run.Id && row.Status == run.Status) == true
        && run.Status is "submitted" or "running" or "cancelling";
    public async Task CancelInlineRunAsync(NativeRun run, CancellationToken token = default)
    {
        if (!CanCancelRun(run) || projectId is not { } project || sessionId is not { } session) return;
        var key = (project, session, run.Id); var current = generation; runMutations.Add(key); Notify();
        try
        {
            var result = await new NativeContextActivityClient(settings!).CancelRunAsync(project, session, run.Id, token);
            if (result.Id != run.Id || result.FrameId != session) throw new InvalidDataException("运行取消响应身份不匹配。");
        }
        catch (Exception ex) { runActionErrors[key] = "取消结果未确认；请核对状态后继续，不会自动重试。" + ex.Message; }
        finally { runMutations.Remove(key); if (current == generation) Notify(); }
        if (current == generation && !token.IsCancellationRequested) await RefreshAsync(token);
    }
    public void AcknowledgeRunAction(string id) { if (!RunBusy(id) && ConnectionError == null) { runActionErrors.Remove(RunKey(id)); Notify(); } }

    // Refresh only the Run records on the pinned historical page. Re-reading a
    // transcript cursor would move an outline selection as new turns arrive.
    private async Task RefreshHistoricalRunsAsync(CancellationToken token)
    {
        if (!active || settings == null || ConnectionError != null || !ShowingHistory || History is not { } page
            || projectId is not { } project || sessionId is not { } session || Snapshot is not { } latest) return;
        var ids = page.Items.Where(item => item.Run != null).Select(item => item.Run!.Id).Distinct().ToArray();
        if (ids.Length == 0) return;
        var current = generation; var history = historyGeneration; var request = ++historicalRunRead;
        bool Current() => active && current == generation && history == historyGeneration && request == historicalRunRead
            && ShowingHistory && ReferenceEquals(History, page) && Snapshot?.Epoch == latest.Epoch;
        var reads = new Dictionary<string, NativeRun>();
        var errors = new Dictionary<string, string>();
        var reader = new NativeContextActivityClient(settings);
        async Task<(string Id, NativeRun? Run, string? Error)> Read(string id)
        {
            try
            {
                var run = await reader.ReadRunAsync(project, session, id, token);
                if (run.Id != id || run.FrameId != session || string.IsNullOrEmpty(run.ContextId)
                    || run.Status is not ("draft" or "submitted" or "running" or "paused" or "cancelling" or "succeeded" or "failed" or "cancelled" or "timed_out" or "lost"))
                    throw new InvalidDataException("运行记录与当前历史会话不符。");
                return (id, run, null);
            }
            catch (OperationCanceledException) { throw; }
            catch (Exception ex) { return (id, null, "运行状态暂未更新，显示上次确认的记录：" + ex.Message); }
        }
        try
        {
            // Bound outstanding requests even when a historical turn contains
            // many runs. Replies are committed together after scope validation.
            foreach (var group in ids.Chunk(4))
            {
                if (!Current()) return;
                foreach (var result in await Task.WhenAll(group.Select(Read)))
                {
                    if (result.Run != null) reads[result.Id] = result.Run;
                    else errors[result.Id] = result.Error!;
                }
            }
            if (!Current() || token.IsCancellationRequested) return;
            foreach (var id in ids)
            {
                var key = (project, session, id);
                if (errors.TryGetValue(id, out var error)) runReadErrors[key] = error;
                else runReadErrors.Remove(key);
            }
            var cards = (page.RunCards ?? []).ToDictionary(run => run.Id);
            foreach (var (id, run) in reads) cards[id] = run;
            var items = page.Items.Select(item => item.Run is { } link && reads.TryGetValue(link.Id, out var run)
                ? item with { Run = link with { Status = run.Status, NeedsReview = RunTerminal(run.Status) && run.Kind == "ssh_direct" && run.CleanedAt == null } }
                : item).ToArray();
            History = page with { Items = items, RunCards = cards.Values.ToArray() };
            ObserveRunReviews(History, token);
            Notify();
        }
        catch (OperationCanceledException) { }
    }

    private void ObserveRunReviews(ConversationSnapshot value, CancellationToken token)
    {
        if (value.RunReviewSupported != true || settings == null) { RunReviewPrompt = null; return; }
        foreach (var run in value.RunCards ?? [])
        {
            var key = (value.ProjectId, value.SessionId, run.Id);
            if (run.FrameId != value.SessionId || !value.Items.Any(item => item.Run?.Id == run.Id)) continue;
            observedRuns[key] = run;
            if (previousRunStates.GetValueOrDefault(key) is "submitted" or "running" or "cancelling"
                && run.Status == "succeeded" && run.Kind == "ssh_direct" && run.CleanedAt == null) pendingRunReviews.Add(key);
            previousRunStates[key] = run.Status;
        }
        if (RunReviewPrompt is { } prompt && (prompt.ProjectId != value.ProjectId || prompt.SessionId != value.SessionId
            || value.ReadOnly || !ObservedReviewEligible(value, prompt.RunId))) RunReviewPrompt = null;
        if (value.Running || value.Stopping || value.ReadOnly || ShowingHistory || RunReviewPrompt != null) return;
        var candidate = observedRuns.Where(pair => pair.Key.Project == value.ProjectId && pair.Key.Session == value.SessionId
                && pendingRunReviews.Contains(pair.Key)).Select(pair => pair.Value).Where(run => RunReviewEligible(run, value.SessionId))
            .OrderByDescending(run => run.EndedAt ?? run.CreatedAt).FirstOrDefault();
        if (candidate != null) _ = CheckRunReviewAsync(value.ProjectId, value.SessionId, candidate, token);
    }
    private static bool RunReviewEligible(NativeRun run, string session) => run.FrameId == session && run.Status == "succeeded"
        && run.Kind == "ssh_direct" && run.CleanedAt == null;
    private bool ObservedReviewEligible(ConversationSnapshot snapshot, string id) => snapshot.RunReviewSupported == true
        && observedRuns.TryGetValue((snapshot.ProjectId, snapshot.SessionId, id), out var run) && RunReviewEligible(run, snapshot.SessionId);
    private async Task CheckRunReviewAsync(string project, string session, NativeRun candidate, CancellationToken token)
    {
        var key = (project, session, candidate.Id);
        if (!checkingRunReviews.Add(key)) return;
        var current = generation;
        try
        {
            // Historical submissions may no longer be on the latest page.
            // Verify their current exact record before asking to open results.
            var verified = await new NativeContextActivityClient(settings!).ReadRunAsync(project, session, candidate.Id, token);
            if (current != generation) return;
            if (observedRuns.GetValueOrDefault(key) != candidate) return;
            if (verified.Id != candidate.Id || !RunReviewEligible(verified, session)) { pendingRunReviews.Remove(key); return; }
            observedRuns[key] = verified;
            var reply = await new NativeRunReviewClient(settings!).InvokeAsync(project, session, candidate.Id, new() { ["action"] = "check_prompt" }, token);
            if (current != generation) return;
            if (reply.ShouldPrompt == null) throw new InvalidDataException("缺少运行审阅提示状态。");
            if (Snapshot is not { Running: false, Stopping: false, ReadOnly: false } snapshot || ShowingHistory) return;
            pendingRunReviews.Remove(key);
            if (!reply.ReadOnly && !reply.Cleaned && reply.ShouldPrompt == true && ObservedReviewEligible(snapshot, candidate.Id) && RunReviewPrompt == null)
                RunReviewPrompt = new(project, session, candidate.Id, candidate.Title);
        }
        catch (Exception ex) when (ex is not OperationCanceledException)
        {
            // Suppress this automatic attempt; the manual review remains available.
            pendingRunReviews.Remove(key);
            if (current == generation) OperationError = "运行结果提示检查失败，可从运行卡片手动打开审阅。" + ex.Message;
        }
        catch (OperationCanceledException) { }
        finally { checkingRunReviews.Remove(key); if (current == generation) Notify(); }
    }
    public bool TakeRunReviewPrompt(NativeRunReviewPrompt prompt)
    {
        if (RunReviewPrompt != prompt || ShowingHistory || Snapshot is not { Running: false, Stopping: false, ReadOnly: false } snapshot
            || snapshot.ProjectId != prompt.ProjectId || snapshot.SessionId != prompt.SessionId || !ObservedReviewEligible(snapshot, prompt.RunId)) return false;
        RunReviewPrompt = null; Notify();
        return true;
    }
}

public abstract record NativeRunProgress(double? Percent, string Label);
public sealed record NativeTransferProgress(string Phase, string? File, string[] Details, double? Completion)
    : NativeRunProgress(Completion, string.Join(" · ", new[] { Phase, File }.Concat(Details).Where(s => !string.IsNullOrEmpty(s))));
public sealed record NativeMethodProgress(string Phase, string Counts, string Best)
    : NativeRunProgress(null, $"{Phase} · {Counts} · {Best}");

public static class NativeRunPresentation
{
    public static string MonitorMetadata(NativeRun run, long now, long? heartbeat, NativeLocalization locale)
    {
        static string Duration(long seconds) => seconds < 60 ? $"{seconds}s" :
            seconds < 3600 ? $"{seconds / 60}m {seconds % 60}s" : $"{seconds / 3600}h {seconds % 3600 / 60}m";
        string Time(string key, long seconds) => locale.Format(key, ("time", Duration(Math.Max(0, seconds))));
        var elapsed = Time("runs.elapsed", (run.EndedAt ?? now) - (run.StartedAt ?? run.CreatedAt));
        var text = $"{run.ContextId} · {run.Kind} · {elapsed}";
        if (run.Status is "submitted" or "running" or "cancelling")
        {
            if (heartbeat is { } last) text += " · " + Time("runs.heartbeat", now - last);
            if (run.TimeoutSecs is > 0) text += " · " + Time("runs.timeout", run.TimeoutSecs.Value);
        }
        return run.ExitCode is { } exit ? text + " · exit " + exit : text;
    }
    public static string Output(NativeRun run)
    {
        static string Fold(string? text) => string.Join("\n", (text ?? "").Replace("\r\n", "\n").Split('\n').Select(line => line.Split('\r').Last()));
        var stdout = Fold(run.StdoutTail); var stderr = Fold(run.StderrTail);
        var output = stdout.Length == 0 ? stderr : stderr.Length == 0 ? stdout : stdout + "\n[stderr]\n" + stderr;
        return string.Join("\n", output.TrimEnd('\n').Split('\n').TakeLast(8));
    }
    public static NativeRunProgress? Progress(NativeRun run, NativeLocalization locale)
    {
        if (string.IsNullOrWhiteSpace(run.ProgressJson)) return null;
        try
        {
            var value = JsonNode.Parse(run.ProgressJson);
            if (value is not JsonObject) return null;
            var phase = value["phase"]?.GetValue<string>();
            if (run.Kind == "method_search")
            {
                var best = value["best_primary"]?.GetValue<double>().ToString("F6", System.Globalization.CultureInfo.InvariantCulture) ?? "—";
                return new NativeMethodProgress(phase ?? "", $"{value["candidate_count"]?.GetValue<ulong>() ?? 0} / {value["successful_count"]?.GetValue<ulong>() ?? 0}", best);
            }
            if (string.IsNullOrEmpty(phase) || value["total_bytes"] == null || value["completed_bytes"] == null) return null;
            var done = value["completed_bytes"]?.GetValue<ulong>() ?? 0;
            var total = value["total_bytes"]?.GetValue<ulong>() ?? 0;
            double? percent = phase is ("uploading" or "downloading") && (value["indeterminate"]?.GetValue<bool>() == true || total == 0)
                ? null : total == 0 ? 0 : (double)Math.Min(100m, Math.Ceiling(done * 100m / total));
            var key = phase is "uploading" or "uploaded" or "downloading" or "downloaded" or "cancelled" or "failed" ? phase : "transferring";
            var label = locale.Text("transfer." + key);
            var file = value["current_file"]?.GetValue<string>();
            static string Size(ulong bytes)
            {
                string[] units = ["B", "KB", "MB", "GB", "TB"]; int i = 0; double n = bytes;
                while (n >= 1024 && i + 1 < units.Length) { n /= 1024; i++; }
                return i == 0 ? $"{bytes} B" : FormattableString.Invariant($"{n:F2} {units[i]}");
            }
            if (!percent.HasValue) return new NativeTransferProgress(label, file, [total == 0 ? locale.Text("transfer.byte_progress_unavailable")
                : locale.Format("transfer.total", ("size", Size(total)))], null);
            var details = new List<string> { $"{Size(done)} / {Size(total)} · {percent}%" };
            if (value["bytes_per_second"]?.GetValue<ulong>() is { } speed) details.Add($"{Size(speed)}/s");
            if (value["eta_seconds"]?.GetValue<ulong>() is { } eta)
            {
                var time = eta < 60 ? $"{eta}s" : eta < 3600 ? $"{eta / 60}m {eta % 60}s" : $"{eta / 3600}h {eta % 3600 / 60}m";
                details.Add(locale.Format("transfer.eta", ("time", time)));
            }
            if (value["files_total"]?.GetValue<ulong>() is > 1 and var count)
                details.Add(locale.Format("transfer.files", ("done", (value["files_completed"]?.GetValue<ulong>() ?? 0).ToString()), ("total", count.ToString())));
            return new NativeTransferProgress(label, file, details.ToArray(), percent);
        }
        catch (Exception ex) when (ex is JsonException or InvalidOperationException or FormatException) { return null; }
    }
}
