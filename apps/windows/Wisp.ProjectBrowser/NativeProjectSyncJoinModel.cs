using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>The join dialog owns its code separately from the parent sync settings draft.</summary>
public sealed class NativeProjectSyncJoinModel(INativeSettingsClient client, string? projectId, NativeLocalization localization) : WorkspaceActionModel
{
    public bool ModalOpen { get; private set; }
    public string Code { get; private set; } = "";
    public string? JoinedProjectId { get; private set; }
    public bool CanJoin => ModalOpen && !Busy && !Closed && !string.IsNullOrWhiteSpace(Code);
    public string? DisplayError
    {
        get
        {
            if (Error == null) return null;
            const string prefix = "操作结果未能确认；不会自动重试。\n";
            var message = Error.StartsWith(prefix, StringComparison.Ordinal) ? Error[prefix.Length..] : Error;
            var key = message switch
            {
                "Configure a sync relay URL in Settings first." => "err.sync_relay_url",
                "Configure the relay token in Settings before synchronizing." => "err.sync_relay_token",
                "Choose a shared sync folder in Settings first." => "err.sync_folder",
                "The shared project is not available yet. Wait for the cloud folder or relay to finish updating." => "err.sync_not_ready",
                _ => null
            };
            return key == null ? message : localization.Text(key);
        }
    }
    public void Open()
    {
        if (Closed) return;
        ClearFailure(); ModalOpen = true; Notify();
    }
    // WebView's close button and window Escape can hide an in-flight join.
    // Hiding never cancels or replays the host command; a late reply is still owned here.
    public void Dismiss() { if (!Closed) { ModalOpen = false; Notify(); } }
    public void SetCode(string value) { if (!Closed) { Code = value; Notify(); } }
    public Task<bool> JoinAsync()
    {
        if (!CanJoin) return Task.FromResult(false);
        JoinedProjectId = null;
        var code = Code;
        return RunAsync(() => client.InvokeAsync("join_synced_project", new() { ["code"] = code }, projectId), result =>
        {
            // Null means that the destination picker was cancelled, not that a project was joined.
            if (result == null) return;
            if (result is not JsonObject row || row["id"] is not JsonValue value
                || !value.TryGetValue<string>(out var id) || string.IsNullOrWhiteSpace(id))
                throw new InvalidDataException(localization.Choose("未收到有效的项目，请检查项目列表。", "No valid project was returned. Check the project list."));
            JoinedProjectId = id; Code = ""; ModalOpen = false;
        });
    }
    public override void Dispose() { base.Dispose(); Code = ""; JoinedProjectId = null; ModalOpen = false; }
}
