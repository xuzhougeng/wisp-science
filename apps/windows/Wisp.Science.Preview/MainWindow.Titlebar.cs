using System.Diagnostics;
using Microsoft.UI.Xaml;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private void RefreshWindowTitlebar()
    {
        if (windowClosed) return;
        design.Dark = root.ActualTheme == ElementTheme.Dark;
        design.LightPalette = settings.LightPalette; design.DarkPalette = settings.DarkPalette; design.RefreshPalette();
        var project = workspaceSheet is NativeAssistantPage ? design.T("assistant.title")
            : examplePage != null ? design.T("projects.example")
            : model.Projects.FirstOrDefault(p => p.Id == model.ActiveProjectId)?.Name;
        Title = NativeHomePresentation.WindowTitle(project);
        windowTitlebar.Update(project, hostVersion, model.ActiveProjectId == null && examplePage == null && workspaceSheet is not NativeAssistantPage,
            model.ActiveProjectId != null && examplePage == null);
    }
    private bool CanRunWindowCommand() => !windowClosed && !nativePickerOpen && !openingAction
        && settingsPage == null && searchOverlay == null && workspaceSheet == null
        && projectImport?.IsModal != true && projectExport?.IsModal != true && conversationPage?.HasRuntimeModal != true;
    private void RunWindowCommand(string command)
    {
        if (!CanRunWindowCommand()) return;
        // Mounted editors keep their own leave/confirmation flow.
        if (examplePage != null && command is "new-project" or "projects") CloseExampleProject();
        switch (command)
        {
            case "quit": Close(); break;
            case "commands": OpenSearch(true); break;
            case "privacy-mode": _ = OpenPrivacyAsync(); break;
            case "setup": _ = OpenQuickSetupAsync(); break;
            case "import-project": _ = OpenNativeAction("import"); break;
            case "export-current-project":
                if (model.Projects.FirstOrDefault(p => p.Id == model.ActiveProjectId) is { } project) _ = OpenProjectExportAsync(project);
                break;
            case "import-codex": _ = OpenNativeAction("import-codex"); break;
            case "import-claude": _ = OpenNativeAction("import-claude"); break;
            case "runs": ShowPanelTab("runs"); break;
            case "check-updates": _ = OpenUpdateCheckAsync(); break;
            case "star-us": OpenWindowLink("https://github.com/xuzhougeng/wisp-science"); break;
            case "issues": OpenWindowLink("https://github.com/xuzhougeng/wisp-science/issues"); break;
            default: RunPaletteCommand(command); break;
        }
    }
    private void OpenWindowLink(string url)
    {
        try { Process.Start(new ProcessStartInfo(url) { UseShellExecute = true }); }
        catch (Exception ex) { localError = ex.Message; Render(); }
    }
    private async Task ReadWindowVersionAsync(NativeSettingsClient host)
    {
        var database = model.DatabasePath;
        using var deadline = new CancellationTokenSource(TimeSpan.FromSeconds(10));
        try
        {
            var info = await host.InvokeAsync("get_bootstrap_status", new(), cancellationToken: deadline.Token);
            if (!windowClosed && database == model.DatabasePath && workspaceHost == host)
            { hostVersion = info?["app_version"]?.GetValue<string>(); RefreshWindowTitlebar(); }
        }
        catch { /* A missing version must not prevent opening or using the workbench. */ }
    }
    private async Task OpenUpdateCheckAsync()
    {
        var database = model.DatabasePath;
        var navigation = model.NavigationRevision;
        var host = await ConnectHostAsync();
        if (host == null || !CanRunWindowCommand() || database != model.DatabasePath || navigation != model.NavigationRevision) return;
        MountSheet(new NativeUpdateCheckPage(new(host), design, CloseSheet,
            url => Process.Start(new ProcessStartInfo(url) { UseShellExecute = true })));
    }

    private int quickSetupIntent;
    private async Task OpenQuickSetupAsync(bool firstLaunch = false)
    {
        var ticket = ++quickSetupIntent;
        var database = model.DatabasePath;
        var navigation = model.NavigationRevision;
        var settingsOwner = settingsPage;
        bool Current() => ticket == quickSetupIntent && database == model.DatabasePath && navigation == model.NavigationRevision
            && settingsPage == settingsOwner && (CanRunWindowCommand() || !firstLaunch && settingsPage != null && SetupCommandLayer
                && !windowClosed && !nativePickerOpen && !openingAction && searchOverlay == null) && !overlays.Any();
        var host = await ConnectHostAsync();
        if (host == null || !Current()) return;
        bool remember;
        try { remember = await NativeQuickSetupModel.ShouldShowAsync(host); }
        catch { remember = false; } // An older host can still run the manually opened wizard.
        if (!Current() || firstLaunch && !remember) return;
        var wizardDesign = settingsOwner?.OverlayDesign ?? design;
        var wizard = new NativeQuickSetupModel(host, wizardDesign.Localization);
        MountSheet(new NativeQuickSetupPage(wizard, wizardDesign,
            () => OpenWindowLink("https://platform.deepseek.com/api_keys"), () =>
            {
                CloseSheet();
                if (remember) _ = RememberDismissalAsync();
            }), nested: workspaceSheet != null);
        if (!firstLaunch) await wizard.Environment.DetectAsync();
        else
        {
            // Startup has already begun path discovery. Do not launch a second scan.
            try { wizard.Environment.AcceptSnapshot(await host.InvokeAsync("get_bootstrap_status", new())); wizard.Environment.Notify(); }
            catch { if (!wizard.Closed) wizard.Environment.Fail(design.T("env.check_again")); }
        }
        async Task RememberDismissalAsync()
        {
            try { await wizard.DismissAsync(); }
            catch { if (!windowClosed && database == model.DatabasePath) ShowChatSearchNotice(design.L("未能记住向导已关闭；下次启动时可能会再次显示。", "Could not remember dismissal; setup may appear again at startup.")); }
        }
    }
}
