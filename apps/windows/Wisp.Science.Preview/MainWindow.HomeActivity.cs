using Microsoft.UI.Xaml;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private readonly DispatcherTimer homePoll = new() { Interval = TimeSpan.FromSeconds(2) };
    private CancellationTokenSource? homeRead;
    private bool homeForeground;
    private Action? updateHomeRows;
    private string? homeRevision;
    private DateTimeOffset homeFullRead, homeRetryAt;
    private DateTimeOffset? homeDiskRead;
    private NativeSettingsClient? homeSupportedHost;

    private void UpdateHomeActivityRows() => updateHomeRows?.Invoke();
    private bool OnLiveHome => homeForeground && !windowClosed && model.ActiveProjectId == null &&
        examplePage == null && model.LastLoaded != null && workspaceHost != null;
    private bool CanReadHome => OnLiveHome && HomeUncovered && !overlays.Any() && !model.Loading;

    private void UpdateHomePolling()
    {
        if (!OnLiveHome)
        {
            homePoll.Stop(); homeRead?.Cancel(); homeRevision = null; return;
        }
        if (homePoll.IsEnabled) return;
        homePoll.Start();
        DispatcherQueue.TryEnqueue(() => _ = ReadHomeActivityAsync());
    }

    private async Task ReadHomeActivityAsync()
    {
        if (!CanReadHome) { homeRevision = null; UpdateHomePolling(); return; }
        if (homeRead != null || DateTimeOffset.UtcNow < homeRetryAt || workspaceHost is not { } host) return;
        var nav = model.NavigationRevision; var privacy = model.PrivacyRevision; var loaded = model.LastLoaded;
        using var read = new CancellationTokenSource(TimeSpan.FromSeconds(8)); homeRead = read;
        bool Current() => CanReadHome && workspaceHost == host && nav == model.NavigationRevision &&
            privacy == model.PrivacyRevision && loaded == model.LastLoaded;
        try
        {
            if (homeSupportedHost != host)
            {
                var caps = await host.InvokeAsync("native_settings_capabilities", new(), cancellationToken: read.Token);
                if (!Current() || read.IsCancellationRequested) return;
                if (!NativeHomeActivityClient.Supported(caps))
                {
                    homeRetryAt = DateTimeOffset.UtcNow.AddSeconds(30); model.ApplyHomeActivity(null); return;
                }
                homeSupportedHost = host; homeRevision = null;
            }
            var needFull = homeDiskRead != loaded || DateTimeOffset.UtcNow - homeFullRead >= TimeSpan.FromSeconds(30);
            var snap = await NativeHomeActivityClient.ReadAsync(host, needFull ? null : homeRevision, read.Token);
            if (!Current() || read.IsCancellationRequested || snap == null) return;
            homeRevision = snap.Revision; homeFullRead = DateTimeOffset.UtcNow; homeDiskRead = loaded;
            model.ApplyHomeActivity(snap);
        }
        catch (Exception)
        {
            if (!Current()) return;
            homeRevision = null;
            if (read.IsCancellationRequested && !homePoll.IsEnabled) return;
            homeSupportedHost = null;
            homeRetryAt = DateTimeOffset.UtcNow.AddSeconds(8); model.ApplyHomeActivity(null);
        }
        finally { homeRead = null; }
    }
}
