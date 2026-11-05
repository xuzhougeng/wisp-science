using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class NativeExampleCopy(INativeSettingsClient client, NativeExampleInfo source, IEnumerable<ProjectSummary> projects) : IDisposable
{
    public NativeExampleInfo Source { get; } = source;
    public ProjectSummary[] Targets { get; } = projects.Where(p => !string.IsNullOrWhiteSpace(p.Id) && !p.Id.StartsWith("assistant:", StringComparison.Ordinal)).DistinctBy(p => p.Id).ToArray();
    public bool Busy { get; private set; }
    public bool Unconfirmed { get; private set; }
    public string? Error { get; private set; }
    public string? Result { get; private set; }
    public event Action? Changed;
    private bool disposed;
    public async Task<bool> CopyAsync(string target)
    {
        if (disposed || Busy || Unconfirmed || Result != null || !Targets.Any(p => p.Id == target)
            || !NativeExampleCatalog.Entries.Any(e => e.Id == Source.Id)) return false;
        Busy = true; Error = null; Changed?.Invoke();
        try
        {
            var result = (await client.InvokeAsync("copy_demo_to_project", new() { ["id"] = Source.Id, ["targetProjectId"] = target }))?.GetValue<string>();
            if (disposed) return false;
            if (string.IsNullOrWhiteSpace(result)) throw new InvalidDataException("Missing copied conversation acknowledgement");
            Result = result; return true;
        }
        catch (Exception ex) { if (!disposed) { Error = ex.Message; Unconfirmed = true; } return false; }
        finally { if (!disposed) { Busy = false; Changed?.Invoke(); } }
    }
    public void Dispose() { disposed = true; Changed = null; }
}
