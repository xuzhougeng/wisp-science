using System.Text.Json;

namespace Wisp.ProjectBrowser.Contracts;

/// Client-local layout. Optional data-backed surfaces opt into Available when implemented.
public sealed class NativePanelTabs
{
    public static IReadOnlyList<string> Defaults { get; } = Array.AsReadOnly(new[] { "artifacts", "agents", "files", "hosts" });
    public static IReadOnlyList<string> All { get; } = Array.AsReadOnly(new[] { "artifacts", "agents", "runs", "notebook", "highlights", "files", "provenance", "hosts", "sidechat", "memos" });
    private readonly List<string> open;
    public IReadOnlyList<string> Open => open.AsReadOnly();
    public IReadOnlyList<string> Available { get; }
    public string Selected { get; private set; }
    public string Saved => JsonSerializer.Serialize(open);
    public NativePanelTabs(string? saved = null, string selected = "artifacts", IEnumerable<string>? available = null)
    {
        Available = Array.AsReadOnly((available ?? Defaults).Where(All.Contains).Distinct().ToArray());
        string[]? decoded = null;
        try { if (saved is not null) decoded = JsonSerializer.Deserialize<string[]>(saved); } catch (JsonException) { }
        open = (decoded ?? Defaults.ToArray()).Where(Available.Contains).Distinct().ToList();
        Selected = open.Contains(selected) ? selected : open.FirstOrDefault() ?? "artifacts";
    }
    public void Show(string id)
    {
        if (!Available.Contains(id)) return;
        if (!open.Contains(id)) open.Add(id);
        Selected = id;
    }
    public void Remove(string id)
    {
        var index = open.IndexOf(id);
        if (index < 0) return;
        open.RemoveAt(index);
        if (Selected == id && open.Count > 0) Selected = open[Math.Max(0, index - 1)];
    }
    public void Move(string id, string target)
    {
        var from = open.IndexOf(id); var to = open.IndexOf(target);
        if (from < 0 || to < 0 || from == to) return;
        open.RemoveAt(from); open.Insert(to, id);
    }
    public void Reopen()
    {
        if (open.Count > 0) return;
        open.AddRange(Defaults.Where(Available.Contains));
        if (open.Count == 0 && Available.Count > 0) open.Add(Available[0]);
        Selected = open.FirstOrDefault() ?? "artifacts";
    }
}
