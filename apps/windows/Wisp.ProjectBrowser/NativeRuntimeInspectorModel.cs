using System.Text.Json;
using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// Read-only object snapshots, retained by context/language within one project/frame.
public sealed class NativeRuntimeInspectorModel(INativeSettingsClient? transport)
{
    private sealed class State { public bool Loading; public string? Error; public NativeRuntimeObjects? Snapshot; }
    private readonly Dictionary<string, State> states = [];
    private string? project, session; private int binding;
    private NativeRuntimeSlot[] slots = [];
    private string Key => Selected == null ? "" : Selected.Context.Id + "/" + Selected.Language;
    private State? Current => states.GetValueOrDefault(Key);
    public event Action? Changed;
    public NativeRuntimeSlot? Selected { get; private set; }
    public bool Pinned { get; set; }
    public string Filter { get; private set; } = "";
    public bool Loading => Current?.Loading == true;
    public string? Error => Current?.Error;
    public NativeRuntimeObjects? Snapshot => Current?.Snapshot;
    public bool CanRefresh => transport != null && Selected?.Status == "ready" && !Loading;
    public NativeRuntimeSlot[] Siblings => slots.Where(s => s.Context.Id == Selected?.Context.Id).ToArray();
    public NativeRuntimeObject[] Rows => Snapshot?.Objects.Where(row => Matches(row, Filter)).ToArray() ?? [];
    public void Bind(string? projectId, string? sessionId)
    {
        if (project == projectId && session == sessionId) return;
        binding++; project = projectId; session = sessionId; states.Clear(); slots = []; Close();
    }
    public void Observe(NativeRuntimeSlot[] value)
    {
        slots = value;
        if (Selected == null) return;
        var next = slots.FirstOrDefault(s => s.Context.Id == Selected.Context.Id && s.Language == Selected.Language);
        if (next == null) { Close(); return; }
        if (next == Selected) return;
        Selected = next; Changed?.Invoke();
    }
    public async Task OpenAsync(string context, string language)
    {
        var slot = slots.FirstOrDefault(s => s.Context.Id == context && s.Language == language);
        if (slot == null || project == null) return;
        Selected = slot; Filter = ""; Changed?.Invoke();
        if (slot.Status is "ready" or "busy") await InspectAsync(true);
    }
    public void Close() { Selected = null; Filter = ""; Pinned = false; Changed?.Invoke(); }
    public void SetFilter(string value) { if (Filter == value) return; Filter = value; Changed?.Invoke(); }
    public async Task InspectAsync(bool opening = false)
    {
        if (transport == null || project == null || Selected?.Info is not { } info || Loading
            || !(CanRefresh || opening && Selected.Status == "busy")) return;
        var key = Key; var current = binding; var p = project; var s = session;
        if (!states.TryGetValue(key, out var state)) states[key] = state = new();
        state.Loading = true; state.Error = null; Changed?.Invoke();
        try
        {
            var value = await new NativeRuntimesClient(transport).CallAsync("inspect", p, s,
                new JsonObject { ["runtime_id"] = info.RuntimeId, ["runtime_generation"] = info.Generation });
            if (current != binding || !SameWorker()) return;
            var snapshot = value?.Deserialize<NativeRuntimeObjects>(ConversationSnapshot.JsonOptions);
            if (snapshot?.Objects == null || snapshot.TotalCount < 0) throw new InvalidDataException("Missing runtime object snapshot");
            state.Snapshot = snapshot;
        }
        catch (Exception ex) { if (current == binding && SameWorker()) state.Error = ex.Message; }
        finally { if (current == binding) { state.Loading = false; Changed?.Invoke(); } }
        bool SameWorker() => slots.Any(slot => slot.Info?.RuntimeId == info.RuntimeId && slot.Info.Generation == info.Generation);
    }
    public string EmptyKey => Selected?.Info == null && Snapshot == null && Error == null ? "runtime.environment_unavailable"
        : Loading && Snapshot == null ? "runtime.objects_loading" : Snapshot == null ? "runtime.objects_hint"
        : Snapshot.Objects.Length == 0 ? "runtime.objects_empty" : "runtime.objects_none_match";
    public static bool Matches(NativeRuntimeObject row, string query)
    {
        var text = query.Trim(); return text.Length == 0 || row.Name.Contains(text, StringComparison.OrdinalIgnoreCase)
            || row.TypeName.Contains(text, StringComparison.OrdinalIgnoreCase) || row.Summary.Contains(text, StringComparison.OrdinalIgnoreCase);
    }
    public static string Quote(string language, NativeRuntimeObject row) => $"[{language} runtime] {row.Name}: {row.TypeName}"
        + (row.Summary.Length == 0 || row.Summary == "—" ? "" : " = " + row.Summary)
        + (row.SizeBytes is { } size ? " (" + NativeStoragePresentation.FormatBytes(size) + ")" : "");
    public static (double X, double Y) ClampPosition(double x, double y, double width, double height) =>
        (Math.Clamp(x, 16, Math.Max(16, width - Math.Min(620, Math.Max(0, width - 32)) - 16)),
         Math.Clamp(y, 16, Math.Max(16, height - Math.Min(560, Math.Max(0, height - 32)) - 16)));
}
