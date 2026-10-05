using Microsoft.UI.Xaml;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private void RunPaletteCommand(string command)
    {
        switch (command)
        {
            case "new": _ = CreateSessionAsync(); break;
            case "new-window": ((App)Application.Current).OpenWindow(model.DatabasePath); break;
            case "search": OpenSearch(); break;
            case "settings": OpenSettings(); break;
            case "project-settings": OpenSettingsSection("project", model.ActiveProjectId); break;
            case "skills": OpenSettingsSection("skills"); break;
            case "projects": model.GoHome(); break;
            case "library": _ = OpenNativeAction("library"); break;
            case "calendar": _ = OpenNativeAction("calendar"); break;
            case "import-session": _ = OpenNativeAction("import-session"); break;
            case "import-cli": _ = OpenNativeAction("import-cli"); break;
            case "files" or "artifacts" or "notebook" or "provenance": ShowPanelTab(command); break;
            case "contexts": ShowPanelTab("hosts"); break;
            case "side-chat": ShowPanelTab("sidechat"); break;
            case "close-panel": if (panelVisible) TogglePanel(); break;
            case "toggle-sidebar":
                if (PreviewLayout.Workspace(root.ActualWidth, true, false, settings.PanelWidth).SidebarWidth == 0)
                    sidebarDrawerVisible = !sidebarDrawerVisible;
                else sidebarVisible = !sidebarVisible;
                Render(); break;
            case "theme-light": case "theme-dark": case "theme-system":
                _ = ChangeAppearanceAsync(command[6..]); break;
            case "terminal": ToggleTerminal(); break;
            case "docs": OpenTutorials(); break;
            case "issues": _ = conversation?.PrepareIssueReportAsync(); break;
        }
    }

    private async Task OpenSearchResultAsync(NativeSearchItem item, bool attach, bool newWindow, string database, string? sourceProject, string? sourceSession)
    {
        if (windowClosed || model.DatabasePath != database || model.ActiveProjectId != sourceProject || model.ActiveSessionId != sourceSession) return;
        if (newWindow && !attach && item.Kind is "project" or "session")
        {
            ((App)Application.Current).OpenWindow(database, item.ProjectId, item.SessionId);
            return;
        }
        if (attach)
        {
            if (item.Kind is not ("artifact" or "session") || conversation?.AddReference(new(new(item.Kind, Id: item.Id), item.Title, item.ProjectName)) != true)
                localError = "未能加入引用，请确认当前会话可编辑。";
            else conversationPage?.FocusComposer();
            Render(); return;
        }
        var opening = model.OpenProjectAsync(item.ProjectId, item.SessionId);
        var revision = model.NavigationRevision;
        await opening;
        if (windowClosed || model.NavigationRevision != revision || model.DatabasePath != database || model.ActiveProjectId != item.ProjectId) return;
        if (item.Kind == "artifact")
        {
            if (model.ActiveSessionId != item.SessionId || !model.Sessions.Any(s => s.Id == item.SessionId)) { localError = "产物所属会话当前不可打开，请在原项目中查看。"; Render(); return; }
            var navigation = conversationNavigation;
            panelVisible = true; settings.PanelVisible = true; settings.PanelTab = "artifacts"; SaveSettings();
            await EnsurePanelAndTerminalAsync();
            if (windowClosed || model.NavigationRevision != revision || navigation != conversationNavigation || model.DatabasePath != database || model.ActiveProjectId != item.ProjectId || model.ActiveSessionId != item.SessionId) return;
            if (panelPage != null) await panelPage.OpenSearchArtifactAsync(item.Id);
        }
    }
}
