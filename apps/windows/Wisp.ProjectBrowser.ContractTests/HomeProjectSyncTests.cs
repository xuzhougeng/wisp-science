using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class HomeProjectSyncTests
{
    private sealed class Host : INativeSettingsClient
    {
        public readonly List<(string Command, string Scope, string? Strategy)> Calls = [];
        public Func<Task<JsonNode?>> Reply = () => throw new InvalidOperationException("unexpected call");
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(projectId != null && args["id"]!.GetValue<string>() == projectId, "card id must match the host scope");
            Calls.Add((command, projectId!, args["strategy"]?.ToString())); return Reply();
        }
    }
    private static void Check(bool ok, string message) { if (!ok) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> { ["en"] = new()
        {
            ["projects.sync.last"] = "Synced {when}", ["time.hours"] = "{n}h",
            ["projects.sync.skipped"] = "Left local: {n}.", ["projects.sync.pulled"] = "Downloaded {n}."
        }});
        var now = DateTimeOffset.FromUnixTimeSeconds(1800000000);
        var card = new ProjectSummary("p","Study","","",false,0,0,0,0,0,true,now.ToUnixTimeSeconds()-3601);
        Check(HomeProjectSync.Badge(card, now, locale) == ("Synced 1h","ok"), "valid relay time uses the current locale");
        Check(HomeProjectSync.Badge(card with { SyncConfigured = false }, now, locale) == null, "no configured history has no badge");
        foreach (var (state,key,tone) in new[] { ("saved","saved","ok"),("remote-newer","remote_newer","warn"),("waiting","waiting","warn"),
            ("unpublished","unpublished","warn"),("conflict","conflict","err") })
        {
            var folder = card with { FolderSync = state, LastSyncedAt = null };
            Check(HomeProjectSync.Badge(folder,now,locale) == ("projects.folder."+key,tone), "folder state wins over relay history: "+state);
            Check(HomeProjectSync.Actions(folder,false) == (true,false) && HomeProjectSync.Actions(folder,true) == (true,false), "folder snapshots never share a device code");
        }
        bool Config(string json) => HomeProjectSync.BackendConfigured(JsonNode.Parse(json));
        Check(!Config("""{"sync_backend":"folder","sync_folder":" "}""") && Config("""{"sync_backend":"folder","sync_folder":"X:/sync"}"""), "an empty folder does not enable relay project actions");
        Check(!Config("""{"sync_backend":"relay","sync_relay_url":"https://relay.invalid"}""") && Config("""{"sync_backend":"relay","sync_relay_url":"https://relay.invalid","has_sync_relay_token":true}"""), "a relay needs a URL and an available token");
        Check(HomeProjectSync.Actions(card,false) == (false,false) && HomeProjectSync.Actions(card,true) == (true,true), "old relay history is not a configured backend");

        var sync = new HomeProjectSync(); var host = new Host();
        Task<INativeSettingsClient> Connect() => Task.FromResult<INativeSettingsClient>(host);
        var waiting = new TaskCompletionSource<JsonNode?>(); host.Reply = () => waiting.Task;
        var first = sync.RunAsync(Connect,"p",null,locale);
        Check(sync.Busy("p") && await sync.RunAsync(Connect,"p",null,locale) == null && host.Calls.Count == 1, "do not replay a sync while pending");
        waiting.SetException(new IOException("Sync conflict: divergent revisions")); await first;
        Check(sync.InConflict("p") && !sync.Busy("p") && host.Calls.Count == 1, "conflict opens a choice without overwriting either side");
        var homeOpen = true;
        NativeWindowEscapeTests.PressEscapeImmediately(() => Wisp.Science.Preview.NativeWindowEscape.DispatchPages(null,
            sync.InConflict("p") ? () => sync.Dismiss("p") : () => homeOpen = false));
        Check(homeOpen && !sync.InConflict("p"), "Escape closes only the new conflict without moving focus or leaving the project list");
        await sync.RunAsync(Connect,"p","remote",locale);
        Check(host.Calls.Count == 1, "dismissal and stale resolution never write");
        host.Reply = () => throw new IOException("Sync conflict: changed again"); await sync.RunAsync(Connect,"p",null,locale);
        await sync.RunAsync(Connect,"other","remote",locale); await sync.RunAsync(Connect,"p","invalid",locale);
        Check(host.Calls.Count == 2, "resolution is restricted to an outstanding conflict for that project");
        host.Reply = () => throw new IOException("delivery uncertain");
        try { await sync.RunAsync(Connect,"p","local",locale); throw new InvalidOperationException("failed resolution was accepted"); }
        catch (IOException) { Check(sync.InConflict("p") && host.Calls.Count == 3, "uncertain resolution is not replayed"); }
        host.Reply = () => Task.FromResult(JsonNode.Parse("""{"direction":"pull","uploaded_files":0,"downloaded_files":3,"skipped_paths":["example.tmp"]}"""));
        Check(await sync.RunAsync(Connect,"p","remote",locale) == "Downloaded 3." && !sync.InConflict("p")
            && host.Calls.Last() == ("resolve_project_sync","p","remote"), "explicit resolution uses the selected side and correct scope");
        Check(await sync.RunAsync(Connect,"p",null,locale) == "Downloaded 3. Left local: 1.", "regular sync reports skipped paths by count");
    }
}
