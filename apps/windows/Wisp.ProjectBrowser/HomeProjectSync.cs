using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// One home's manual sync operations. No writes are issued by loading or dismissing a card.
public sealed class HomeProjectSync
{
    private readonly HashSet<string> pending = [];
    private readonly HashSet<string> conflicts = [];
    public bool Busy(string project) => pending.Contains(project);
    public bool InConflict(string project) => conflicts.Contains(project);
    public void Dismiss(string project) => conflicts.Remove(project);

    public static bool BackendConfigured(JsonNode? prefs) => prefs?["sync_backend"]?.ToString() switch
    {
        "folder" => !string.IsNullOrWhiteSpace(prefs["sync_folder"]?.ToString()),
        "relay" => !string.IsNullOrWhiteSpace(prefs["sync_relay_url"]?.ToString())
            && (prefs["has_sync_relay_token"]?.GetValue<bool>() == true || !string.IsNullOrWhiteSpace(prefs["sync_relay_token"]?.ToString())),
        _ => false
    };
    public static (bool Sync, bool Code) Actions(ProjectSummary p, bool configured)
        => (configured || p.FolderSync != null, configured && p.FolderSync == null);
    public static (string Label, string Color)? Badge(ProjectSummary p, DateTimeOffset now, NativeLocalization locale)
    {
        if (p.FolderSync == null && !p.SyncConfigured) return null;
        var when = NativeBrowserPresentation.RelativeTime(p.LastSyncedAt ?? 0, now, locale);
        var key = p.FolderSync switch
        {
            null => when.Length == 0 ? "projects.sync.enabled" : "projects.sync.last",
            "saved" => when.Length == 0 ? "projects.folder.saved" : "projects.folder.saved_at",
            "unpublished" => "projects.folder.unpublished", "waiting" => "projects.folder.waiting",
            "remote-newer" => "projects.folder.remote_newer", _ => "projects.folder.conflict"
        };
        return (locale.Format(key, ("when", when)), p.FolderSync switch
        { "unpublished" or "remote-newer" or "waiting" => "warn", "conflict" => "err", _ => "ok" });
    }

    public async Task<string?> RunAsync(Func<Task<INativeSettingsClient>> connect, string id, string? strategy, NativeLocalization locale)
    {
        if (strategy is not null && (strategy is not ("remote" or "local") || !InConflict(id))) return null;
        if (!pending.Add(id)) return null;
        if (strategy == null) conflicts.Remove(id);
        try
        {
            var host = await connect();
            var args = new JsonObject { ["id"] = id };
            if (strategy != null) args["strategy"] = strategy;
            var value = await host.InvokeAsync(strategy == null ? "sync_project" : "resolve_project_sync", args, id);
            if (value is not JsonObject result || result["direction"] is not JsonValue d || !d.TryGetValue<string>(out var direction)
                || result["uploaded_files"]?.GetValue<long>() is not >= 0 || result["downloaded_files"]?.GetValue<long>() is not >= 0
                || result["skipped_paths"] is not JsonArray skipped)
                throw new InvalidDataException(locale.Choose("未收到有效的同步结果。", "The service did not return a valid sync result."));
            conflicts.Remove(id);
            var notice = direction switch
            {
                "pull" => locale.Format("projects.sync.pulled", ("n", result["downloaded_files"]!.ToString())),
                "push" => locale.Format("projects.sync.pushed", ("n", result["uploaded_files"]!.ToString())),
                _ => locale.Text("projects.sync.current")
            };
            return strategy == null && skipped.Count > 0 ? notice + " " + locale.Format("projects.sync.skipped", ("n", skipped.Count.ToString())) : notice;
        }
        catch (Exception ex) when (strategy == null && ex.Message.Contains("Sync conflict", StringComparison.Ordinal))
        { conflicts.Add(id); return null; }
        finally { pending.Remove(id); }
    }
}
