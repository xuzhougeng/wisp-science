using Microsoft.UI.Xaml;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private int privacyRead;
    private async Task RefreshHomePrivacyAsync()
    {
        var ticket = ++privacyRead;
        var revision = model.PrivacyRevision;
        var database = model.DatabasePath;
        if (workspaceHost is not { } host || model.LastLoaded == null) return;
        PrivacyMode? mode;
        try { mode = await new NativePrivacyClient(host).GetAsync(); } catch { mode = null; }
        if (!windowClosed && database == model.DatabasePath && ticket == privacyRead && revision == model.PrivacyRevision)
            model.ApplyPrivacy(mode);
    }

    private async Task OpenPrivacyAsync()
    {
        var owner = settingsPage;
        if (workspaceSheet is NativeQuickSetupPage || !CanRunWindowCommand() &&
            !(owner != null && SetupCommandLayer && searchOverlay == null && !windowClosed && !openingAction && !nativePickerOpen)
            || overlays.Any()) return;
        var database = model.DatabasePath;
        var page = new NativePrivacyPage(owner?.OverlayDesign ?? design, CloseSheet, async mode =>
        {
            var host = await ConnectHostAsync() ?? throw new InvalidOperationException();
            if (database != model.DatabasePath || windowClosed) throw new OperationCanceledException();
            return await new NativePrivacyClient(host).SaveAsync(mode);
        }, saved =>
        {
            if (database == model.DatabasePath && !windowClosed) model.ApplyPrivacy(saved);
        });
        MountSheet(page, nested: workspaceSheet != null);
        await page.LoadAsync(model.PrivacyChoicesAsync);
    }
}
