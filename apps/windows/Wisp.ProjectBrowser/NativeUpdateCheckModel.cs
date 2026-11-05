using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class NativeUpdateCheckModel(INativeSettingsClient client,
    Func<Task<bool>>? disableReminders = null) : WorkspaceActionModel
{
    public const string Releases = "https://github.com/xuzhougeng/wisp-science/releases";
    private readonly CancellationTokenSource lifetime = new();
    public JsonObject? Result { get; private set; }
    public bool DismissAttempted { get; private set; }
    public bool RemindersDisabled { get; private set; }
    public bool Available => Result?["update_available"]?.GetValue<bool>() == true;
    public string Version => Result?[Available ? "latest_version" : "current_version"]?.GetValue<string>() ?? "";
    public string Notes => NativeUpdateNotes.ForDisplay(Result?["notes"]?.GetValue<string>() ?? "");
    public string ReleaseUrl => Uri.TryCreate(Result?["release_url"]?.GetValue<string>(), UriKind.Absolute, out var url)
        && url.Scheme == Uri.UriSchemeHttps && url.UserInfo.Length == 0 ? url.AbsoluteUri : Releases;
    public string TitleKey => Error != null ? "update_modal.failed_title" : Result == null ? "update_modal.checking_title"
        : Available ? "update_modal.available_title" : "update_modal.up_to_date_title";
    public string BodyKey => Result == null ? "update_modal.checking_body" : Available ? "update_modal.available_body" : "update_modal.up_to_date_body";
    protected override string UnconfirmedFailure(Exception exception) => exception.Message;
    public Task<bool> LoadAsync()
    {
        if (Busy || Closed || DismissAttempted) return Task.FromResult(false);
        Result = null;
        return RunAsync(async () =>
        {
            var result = await client.InvokeAsync("check_for_updates", new(), cancellationToken: lifetime.Token) as JsonObject
                ?? throw new InvalidDataException("Missing update status.");
            var available = result["update_available"]?.GetValue<bool>() ?? throw new InvalidDataException("Missing update availability.");
            if (string.IsNullOrWhiteSpace(result[available ? "latest_version" : "current_version"]?.GetValue<string>()))
                throw new InvalidDataException("Missing update version.");
            // Windows follows the WebView release-page flow; in-app installation is macOS-only.
            _ = result["notes"]?.GetValue<string>(); _ = result["release_url"]?.GetValue<string>();
            return result;
        }, result => Result = result);
    }
    public Task<bool> DisableRemindersAsync()
    {
        if (Busy || Closed || Error != null || !Available || DismissAttempted) return Task.FromResult(false);
        DismissAttempted = true;
        return RunAsync(async () =>
        {
            if (disableReminders != null)
            {
                if (!await disableReminders()) throw new InvalidOperationException("Could not confirm the update preference.");
            }
            else
            {
                var reply = await client.InvokeAsync("set_update_check_enabled", new() { ["enabled"] = false }, cancellationToken: lifetime.Token);
                if (reply is not JsonValue value || !value.TryGetValue<bool>(out var enabled) || enabled)
                    throw new InvalidDataException("Could not confirm the update preference.");
            }
            return true;
        }, _ => RemindersDisabled = true);
    }
    public override void Dispose() { if (Closed) return; base.Dispose(); lifetime.Cancel(); lifetime.Dispose(); }
}
