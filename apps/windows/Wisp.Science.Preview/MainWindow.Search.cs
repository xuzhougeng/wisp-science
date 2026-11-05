using Microsoft.UI.Xaml;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private FrameworkElement? savedChatRow;
    private long? savedChatSequence;
    private bool scrollToChatHit;
    private int chatSearchIntent;
    private readonly NativeTransientNotice chatSearchNotice = new();

    private bool OpenChatHistory()
    {
        if (searchOverlay != null && workspaceSheet is NativeProjectChatSearch) { CloseSearch(); return true; }
        if (model.ActiveProjectId is not { } project || examplePage != null || nativePickerOpen || settingsPage != null
            || workspaceSheet is not (null or NativeShelvedSessionsPage) || overlays.Count != 0 || projectImport?.IsModal == true || projectExport?.IsModal == true || conversationPage?.HasRuntimeModal == true) return false;
        CloseSearch();
        var database = model.DatabasePath;
        var name = model.Projects.FirstOrDefault(p => p.Id == project)?.Name ?? project;
        var intent = ++chatSearchIntent;
        MountSheet(new NativeProjectChatSearch(new(browserClient, database, project), name, design, CloseSheet,
            hit =>
            {
                CloseSheet();
                if (workspaceSheet is NativeShelvedSessionsPage) CloseSheet();
                _ = OpenChatHitAsync(database, project, hit, intent);
            }, Register), nested: workspaceSheet != null);
        return true;
    }

    private async Task OpenChatHitAsync(string database, string project, ProjectChatHit hit, int intent)
    {
        if (model.DatabasePath != database || model.ActiveProjectId != project || windowClosed) return;
        artifactTabs?.Select(null);
        if (projectPage != null) CloseProjectPage();
        var opening = model.ActiveSessionId == hit.SessionId ? Task.CompletedTask :
            model.Sessions.Any(s => s.Id == hit.SessionId) ? model.OpenSessionAsync(hit.SessionId) :
            model.OpenShelvedSessionAsync(project, new(hit.SessionId, hit.SessionTitle, hit.Ts));
        var revision = model.SessionNavigationRevision;
        bool Current() => !windowClosed && model.DatabasePath == database && model.ActiveProjectId == project && model.ActiveSessionId == hit.SessionId
            && revision == model.SessionNavigationRevision && chatSearchIntent == intent;
        try
        {
            await opening;
            if (!Current()) return;
            await conversationReady;
            if (!Current()) return;
            if (conversationPage == null)
            {
                // The persisted-only reader has exact message sequences, even without a desktop host.
                if (!model.Messages.Any(m => m.Sequence == hit.MessageSeq))
                {
                    var page = model.OpenSessionAsync(hit.SessionId, beforeSeq: checked(hit.MessageSeq + 1));
                    revision = model.SessionNavigationRevision; await page;
                }
                if (!Current()) return;
                if (model.Messages.Any(m => m.Sequence == hit.MessageSeq))
                {
                    savedChatSequence = hit.MessageSeq; scrollToChatHit = true;
                }
                else ShowChatSearchNotice(design.T("chat_search.location_unavailable"));
                Render(); return;
            }
            var target = await browserClient.LocateChatAsync(database, project, hit, CancellationToken.None);
            if (!Current() || conversation is not { } live || live.CurrentSessionId != hit.SessionId || live.CurrentProjectId != project) return;
            if (target == null) ShowChatSearchNotice(design.T("chat_search.location_unavailable"));
            else if (!await live.RevealChatQuestionAsync(target) && Current())
                ShowChatSearchNotice(live.Snapshot?.Running == true ? design.T("chat_search.wait_for_turn") :
                    live.OperationError ?? design.T("chat_search.location_unavailable"));
        }
        catch (Exception e) { if (Current()) ShowChatSearchNotice(e.Message); }
    }

    private void ShowChatSearchNotice(string message)
    {
        chatSearchNotice.Show(windowFrame, design, message);
    }

    private void RunPaletteCommand(string command)
    {
        switch (command)
        {
            case "new-project": _ = OpenNativeAction("create"); break;
            case "new": _ = CreateSessionAsync(); break;
            case "new-window": ((App)Application.Current).OpenWindow(model.DatabasePath); break;
            case "search": OpenSearch(); break;
            case "search-project-chats": OpenChatHistory(); break;
            case "settings": OpenSettingsSection("models"); break;
            case "project-settings": OpenSettingsSection("project", model.ActiveProjectId); break;
            case "skills": OpenSettingsSection("skills"); break;
            case "projects": model.GoHome(); break;
            case "library": _ = OpenNativeAction("library"); break;
            case "calendar": _ = OpenNativeAction("calendar"); break;
            case "import-session": _ = OpenNativeAction("import-session"); break;
            case "files" or "artifacts" or "notebook" or "provenance" or "runs": ShowPanelTab(command); break;
            case "contexts": ShowPanelTab("hosts"); break;
            case "side-chat": ShowPanelTab("sidechat"); break;
            case "close-panel": if (panelVisible) TogglePanel(); break;
            case "toggle-sidebar":
                sidebarVisible = !sidebarVisible;
                Render(); break;
            case "theme-light": case "theme-dark": case "theme-system":
                _ = ChangeAppearanceAsync(command[6..]); break;
            case "font-ui-increase" or "font-ui-decrease" or "font-code-increase" or "font-code-decrease":
                _ = ChangeAppearanceAsync((host, token) => AppearanceSettingsModel.StepFontAsync(host,
                    command.StartsWith("font-code-"), command.EndsWith("-increase") ? 1 : -1, token, settings.ReadAppearanceCache)); break;
            case "terminal": ToggleTerminal(); break;
            case "docs": OpenTutorials(); break;
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
                localError = design.L("未能加入引用，请确认当前会话可编辑。", "Could not attach the reference. Check that the conversation is editable.");
            else conversationPage?.FocusComposer();
            Render(); return;
        }
        var opening = model.OpenProjectAsync(item.ProjectId, item.SessionId);
        var revision = model.NavigationRevision;
        await opening;
        if (windowClosed || model.NavigationRevision != revision || model.DatabasePath != database || model.ActiveProjectId != item.ProjectId) return;
        if (item.Kind == "artifact")
        {
            if (model.ActiveSessionId != item.SessionId || !model.Sessions.Any(s => s.Id == item.SessionId)) { localError = design.L("产物所属会话当前不可打开，请在原项目中查看。", "This artifact’s conversation is unavailable. Open it from its original project."); Render(); return; }
            var navigation = conversationNavigation;
            panelVisible = true; settings.PanelVisible = true; settings.PanelTab = "artifacts"; SaveSettings();
            await EnsurePanelAndTerminalAsync();
            if (windowClosed || model.NavigationRevision != revision || navigation != conversationNavigation || model.DatabasePath != database || model.ActiveProjectId != item.ProjectId || model.ActiveSessionId != item.SessionId) return;
            if (panelPage != null) await panelPage.OpenSearchArtifactAsync(item.Id);
        }
    }
}
