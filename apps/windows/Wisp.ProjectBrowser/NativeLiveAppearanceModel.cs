using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Immediate appearance and interaction edits share one ordered persistence queue.</summary>
public sealed class NativeLiveAppearanceModel(INativeSettingsClient client, string? projectId, Func<JsonObject>? local = null)
{
    private readonly Dictionary<string, (long Revision, JsonNode? Value)> pending = [];
    private long revision;
    private Task persistence = Task.CompletedTask;
    public JsonObject? Draft { get; private set; }
    public bool Saving { get; private set; }
    public bool HasChanges => pending.Count > 0;
    public string? Error { get; private set; }
    public event Action? Changed;

    public async Task LoadAsync(CancellationToken token = default)
    {
        if (HasChanges && !await WaitForPersistenceAsync()) return;
        var value = await client.InvokeAsync("get_appearance_prefs", new(), projectId, token) as JsonObject
            ?? throw new InvalidDataException("Missing appearance preferences.");
        token.ThrowIfCancellationRequested();
        Draft = NativeAppearanceSnapshot.Resolve(value, local);
        Error = null;
        Changed?.Invoke();
    }

    public void Set(string key, JsonNode? value)
    {
        if (!NativeAppearanceSnapshot.Fields.Contains(key)) throw new ArgumentException("Not an appearance field", nameof(key));
        if (Draft == null) throw new InvalidOperationException("Appearance has not loaded.");
        if (JsonNode.DeepEquals(Draft[key], value)) return;
        Draft[key] = value?.DeepClone();
        pending[key] = (++revision, value?.DeepClone());
        Changed?.Invoke();
        // A failed/uncertain write remains visible until the user explicitly retries.
        if (Error == null) StartPersistence();
    }

    public void Retry()
    {
        if (Saving || !HasChanges) return;
        Error = null;
        StartPersistence();
    }

    public async Task<bool> WaitForPersistenceAsync()
    {
        do { await persistence; } while (Saving);
        return !HasChanges && Error == null;
    }

    private void StartPersistence()
    {
        if (Saving || !HasChanges) return;
        var completion = new TaskCompletionSource(TaskCreationOptions.RunContinuationsAsynchronously);
        persistence = completion.Task;
        Saving = true;
        Changed?.Invoke();
        _ = PersistAsync(completion);
    }

    private async Task PersistAsync(TaskCompletionSource completion)
    {
        try
        {
            while (pending.Count > 0)
            {
                // This timeout is independent of the page lifetime: Escape/navigation
                // must not silently discard an edit already accepted by the UI.
                using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(15));
                var fresh = await client.InvokeAsync("get_appearance_prefs", new(), projectId, deadline.Token) as JsonObject
                    ?? throw new InvalidDataException("Missing appearance preferences.");
                fresh = NativeAppearanceSnapshot.Resolve(fresh, local);
                var patch = pending.ToArray();
                foreach (var (key, change) in patch) fresh[key] = change.Value?.DeepClone();
                var saved = await client.InvokeAsync("set_appearance_prefs", new() { ["prefs"] = fresh }, projectId, deadline.Token) as JsonObject
                    ?? throw new InvalidDataException("Missing saved appearance preferences.");
                foreach (var (key, change) in patch)
                    if (pending.TryGetValue(key, out var current) && current.Revision == change.Revision) pending.Remove(key);
                // Keep the object captured by live controls and any newer edit intact.
                foreach (var (key, value) in saved)
                    if (!pending.ContainsKey(key)) Draft![key] = value?.DeepClone();
            }
        }
        catch (Exception ex) { Error = ex.Message; }
        finally { Saving = false; completion.TrySetResult(); Changed?.Invoke(); }
    }
}
