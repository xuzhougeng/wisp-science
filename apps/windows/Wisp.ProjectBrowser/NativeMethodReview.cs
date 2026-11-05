using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class NativeMethodReview(Func<string, CancellationToken, Task<NativeMethodDetail>> request)
{
    private string? lastStatus;
    private int revision;
    public bool Expanded { get; set; }
    public bool ShowCandidates { get; set; }
    public bool ShowStrategies { get; set; }
    public bool Busy { get; private set; }
    public bool Stale { get; private set; } = true;
    public bool Unconfirmed { get; private set; }
    public bool Writable { get; private set; }
    public string? Error { get; private set; }
    public NativeMethodDetail? Detail { get; private set; }
    public event Action? Changed;
    public void Observe(NativeRun run)
    {
        Writable = run.CanManage == true;
        if (lastStatus == run.Status) return;
        lastStatus = run.Status; revision++;
        if (Detail?.Run.Status != run.Status) Stale = true;
    }
    public static string[] Actions(string? status) => status switch {
        "draft" => ["start", "cancel"], "submitted" or "running" => ["pause", "cancel"],
        "paused" => ["resume", "cancel"], _ => []
    };
    public Task ExecuteAsync(string op, CancellationToken token) => ExecuteAsync(op, token, reconcile: true);
    private async Task ExecuteAsync(string op, CancellationToken token, bool reconcile)
    {
        bool write = op != "inspect";
        if (Busy || token.IsCancellationRequested ||
            write && (!Writable || Stale || Unconfirmed || !Actions(Detail?.Run.Status).Contains(op))) return;
        var atStart = revision; Busy = true; Error = null; Changed?.Invoke();
        try
        {
            var value = await request(op, token);
            token.ThrowIfCancellationRequested();
            Detail = value; Stale = atStart != revision && lastStatus != value.Run.Status;
            Unconfirmed = false;
        }
        catch (Exception e)
        {
            // A dropped acknowledgement is not evidence that the transition failed.
            // Only inspection can unlock the buttons; never queue a duplicate write.
            Stale = true; Unconfirmed |= write;
            if (!token.IsCancellationRequested) Error = e.Message;
        }
        finally { Busy = false; Changed?.Invoke(); }
        // A poll can observe a newer status while the detail request is in flight.
        // Reconcile that race once, by reading. Errors or lost writes require manual inspection.
        if (reconcile && Expanded && Stale && Error == null && !Unconfirmed && !token.IsCancellationRequested)
            await ExecuteAsync("inspect", token, reconcile: false);
    }
}
