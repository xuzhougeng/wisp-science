using System.Diagnostics;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Windows.Storage.Pickers;
using Windows.ApplicationModel.DataTransfer;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private WorkspaceSessionGroups? sessionGroups;
    private string? actionProject;
    private IWorkspaceSheet? projectPage;
    private bool openingAction;
    private (string Database, ProjectCreationDraft Draft)? newProjectDraft;
    private NativeAssistantModel? assistant;
    private INativeSettingsClient? assistantTransport;
    private NativeAssistantPage AssistantPage(INativeSettingsClient host)
    {
        if (!ReferenceEquals(assistantTransport, host))
        {
            assistant?.Close();
            assistant = new(host, new(settings.AssistantProjectsVisible, settings.AssistantCalendarVisible, (projects, calendar) =>
            { settings.AssistantProjectsVisible = projects; settings.AssistantCalendarVisible = calendar; SaveSettings(); }));
            assistantTransport = host;
        }
        assistant!.Conversation.ApplyInputPreferences(settings.InputPreferences ?? new());
        return new(assistant, design, model.Projects, CloseSheet, () => OpenSettingsSection("specialists", preserveAssistant: true), PickAttachments,
            () => OpenSettingsSection("skills", NativeAssistantClient.ProjectId, preserveAssistant: true), SaveSharedFileAsync, PickFileDownload,
            () => OpenSettingsSection("hooks", NativeAssistantClient.ProjectId, preserveAssistant: true),
            () => OpenSettingsSection("environments", NativeAssistantClient.ProjectId, preserveAssistant: true),
            () => OpenComputeHostDialog(NativeAssistantClient.ProjectId),
            () => OpenSettingsSection("models", NativeAssistantClient.ProjectId, preserveAssistant: true),
            () => settings.ModelSwitchWarningDisabled, () => { settings.ModelSwitchWarningDisabled = true; SaveSettings(); });
    }
    private void OpenComputeHostDialog(string? project)
    {
        if (windowClosed || workspaceHost == null || project == null || settingsPage != null || nativePickerOpen
            || workspaceSheet != null && workspaceSheet is not NativeAssistantPage) return;
        MountSheet(NativeSettingsSectionPage.CreateEnvironmentHostDialog(workspaceHost, project, design, CloseSheet), nested: workspaceSheet != null);
    }
    private async Task<string?> PickFileDownload(string name)
    {
        if (nativePickerOpen) return null;
        nativePickerOpen = true;
        try
        {
            return await NativeWorkspaceFiles.PickDownloadAsync(this, name, design.L("文件", "File"));
        }
        finally { nativePickerOpen = false; }
    }
    private async Task<string?> SaveSharedFileAsync(NativeShareFile file)
    {
        if (nativePickerOpen) return null;
        nativePickerOpen = true;
        try
        {
            var path = await NativeWorkspaceFiles.SaveShareAsync(this, file);
            if (!string.IsNullOrEmpty(path)) _ = ShowShareSavedAsync(path);
            return path;
        }
        finally { nativePickerOpen = false; }
    }
    private async Task ShowShareSavedAsync(string path)
    {
        var notice = new InfoBar { IsOpen = true, IsClosable = false, Severity = InfoBarSeverity.Success,
            Message = design.T("share.saved").Replace("{path}", path), MaxWidth = 600,
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(20) };
        root.Children.Add(notice); Canvas.SetZIndex(notice, 100);
        await Task.Delay(TimeSpan.FromSeconds(5)); root.Children.Remove(notice);
    }
    private readonly Stack<(IWorkspaceSheet Sheet, Control? Focus, bool Retained)> parentSheets = new();
    private Control? modalPreviousFocus;
    private NativeProjectImportPage? projectImport;
    private Control? importPreviousFocus;
    private NativeProjectExportPage? projectExport;
    private Control? exportPreviousFocus;
    private string? exportProjectId, restoreExportFocus;
    private string? restoreDeleteFocus;

    private async Task OpenProjectDeleteAsync(ProjectSummary project)
    {
        if (openingAction || nativePickerOpen || workspaceSheet != null || settingsPage != null || searchOverlay != null
            || projectExport?.Busy == true || projectExport?.IsModal == true || projectImport?.Busy == true || projectImport?.IsModal == true) return;
        openingAction = true; var database = model.DatabasePath; var revision = model.NavigationRevision;
        try
        {
            var host = await ConnectHostAsync();
            if (windowClosed || database != model.DatabasePath || revision != model.NavigationRevision || workspaceSheet != null) return;
            if (host == null) { localError ??= design.L("无法连接桌面服务。", "Could not connect to the desktop service."); Render(); return; }
            var deletion = new NativeProjectDeleteModel(host, project.Id);
            NativeProjectDeletePage? page = null;
            page = new(deletion, design, project.WorkspaceDirectory, async () =>
            {
                if (windowClosed || !ReferenceEquals(workspaceSheet, page) || deletion.Busy) return;
                var attempted = deletion.Attempted;
                restoreDeleteFocus = deletion.Deleted ? null : project.Id;
                CloseSheet(); Render();
                if (attempted && database == model.DatabasePath) await model.RefreshAsync();
            });
            MountSheet(page);
        }
        catch (Exception ex) { if (!windowClosed) { localError = ex.Message; Render(); } }
        finally { openingAction = false; }
    }

    private async Task OpenProjectExportAsync(ProjectSummary project)
    {
        if (projectExport is { Busy: false, IsModal: false }) CloseProjectExport();
        if (projectExport != null || projectImport?.Busy == true || projectImport?.IsModal == true || openingAction || nativePickerOpen || workspaceSheet != null || settingsPage != null || searchOverlay != null) return;
        openingAction = true; var database = model.DatabasePath; var revision = model.NavigationRevision;
        exportPreviousFocus = FocusManager.GetFocusedElement(root.XamlRoot) as Control;
        try
        {
            var host = await ConnectHostAsync();
            if (windowClosed || database != model.DatabasePath || revision != model.NavigationRevision) return;
            if (host == null) { localError ??= design.L("无法连接桌面服务。", "Could not connect to the desktop service."); Render(); return; }
            exportProjectId = project.Id;
            projectExport = new(new NativeProjectExportModel(host, project.Id), design, directory => PickExportDestination(project.Name, directory), CloseProjectExport);
            projectExport.PresentationChanged += Render; Render();
        }
        catch (Exception ex) { if (!windowClosed) { localError = ex.Message; Render(); } }
        finally { openingAction = false; }
    }
    private void CloseProjectExport()
    {
        if (projectExport == null || projectExport.Busy) return;
        root.Children.Remove(projectExport); projectExport.PresentationChanged -= Render; projectExport.Dispose(); projectExport = null;
        restoreExportFocus = exportProjectId; exportProjectId = null; Render();
        if (exportPreviousFocus is { IsLoaded: true } previous) previous.Focus(FocusState.Programmatic);
        exportPreviousFocus = null;
    }
    private async Task<string?> PickExportDestination(string name, bool directory)
    {
        if (directory)
        {
            var parent = await PickDirectory();
            return parent == null ? null : NativeProjectExportModel.DirectoryDestination(parent, name);
        }
        if (nativePickerOpen) return null;
        nativePickerOpen = true;
        try
        {
            var picker = new FileSavePicker { SuggestedFileName = NativeProjectExportModel.ArchiveName(name) };
            picker.FileTypeChoices.Add("Wisp project", [".zip"]);
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
            return (await picker.PickSaveFileAsync())?.Path;
        }
        finally { nativePickerOpen = false; }
    }

    private async Task OpenProjectImportAsync()
    {
        if (projectImport is { Busy: false, IsModal: false }) CloseProjectImport();
        if (projectImport != null || projectExport?.Busy == true || projectExport?.IsModal == true || openingAction || nativePickerOpen || workspaceSheet != null || settingsPage != null || searchOverlay != null) return;
        openingAction = true; var database = model.DatabasePath; var revision = model.NavigationRevision;
        importPreviousFocus = FocusManager.GetFocusedElement(root.XamlRoot) as Control;
        try
        {
            var host = await ConnectHostAsync();
            if (windowClosed || database != model.DatabasePath || revision != model.NavigationRevision) return;
            if (host == null) { localError ??= design.L("无法连接桌面服务。", "Could not connect to the desktop service."); Render(); return; }
            var state = new NativeProjectImportModel(host);
            projectImport = new NativeProjectImportPage(state, design, PickDirectory, () => PickFile(".zip"), async (id, open) =>
            {
                if (windowClosed || state.Closed || database != model.DatabasePath) return;
                open &= revision == model.NavigationRevision;
                if (open) CloseProjectImport();
                await model.RefreshAsync();
                if (open && !windowClosed && database == model.DatabasePath && revision == model.NavigationRevision)
                    await model.OpenProjectAsync(id);
            }, CloseProjectImport, Register);
            projectImport.PresentationChanged += UpdateImportModality; root.Children.Add(projectImport); UpdateImportModality();
        }
        catch (Exception ex) { if (!windowClosed) { localError = ex.Message; Render(); } }
        finally { openingAction = false; }
    }
    private void UpdateImportModality()
    {
        if (pageContent != null) pageContent.IsHitTestVisible = projectImport?.IsModal != true && projectExport?.IsModal != true && searchOverlay == null && workspaceSheet == null;
    }
    private void CloseProjectImport()
    {
        if (projectImport == null) return;
        root.Children.Remove(projectImport); projectImport.Dispose(); projectImport = null; UpdateImportModality();
        if (importPreviousFocus is { IsLoaded: true } previous) previous.Focus(FocusState.Programmatic);
        importPreviousFocus = null;
    }

    private int explorationNavigation = -1;
    private void SyncProjectActions()
    {
        if (actionProject == model.ActiveProjectId)
        {
            if (!model.SessionsLoading && explorationNavigation != model.NavigationRevision && sessionGroups is { } current)
            {
                explorationNavigation = model.NavigationRevision;
                _ = RefreshExplorationsAsync(current);
            }
            return;
        }
        actionProject = model.ActiveProjectId;
        explorationNavigation = model.NavigationRevision;
        sessionGroups?.Dispose(); sessionGroups = null;
        projectPage?.Dispose(); projectPage = null;
        if (actionProject is { } project) _ = LoadGroups(project);
    }
    private async Task RefreshExplorationsAsync(WorkspaceSessionGroups groups)
    {
        await groups.LoadExplorationsAsync();
        if (!windowClosed && sessionGroups == groups) Render();
    }
    private async Task LoadGroups(string project)
    {
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || actionProject != project) return;
        var groups = new WorkspaceSessionGroups(host, project) { Sort = settings.SessionSort, Group = settings.SessionGroup };
        sessionGroups = groups;
        await groups.LoadAsync();
        await groups.LoadExplorationsAsync();
        if (!windowClosed && sessionGroups == groups) Render();
    }
    private int explorationOpenTicket;
    private int explorationPreviewTicket;
    private int explorationStartTicket;
    private async Task StartExplorationAsync(NativeHistoryTarget target)
    {
        var groups = sessionGroups; var source = conversation;
        var intent = model.SessionNavigationRevision; var ticket = ++explorationStartTicket;
        if (groups == null || source == null || !source.CanStartExploration(target)) return;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || sessionGroups != groups || conversation != source
            || model.SessionNavigationRevision != intent || ticket != explorationStartTicket || !source.CanStartExploration(target)) return;
        ClearSheets();
        var creation = new WorkspaceExplorationStart(host, target,
            WorkspaceExplorationStart.DefaultName(design.Localization, source.Snapshot?.Exploration?.CandidateCount ?? 0));
        NativeExplorationStartPage? page = null;
        page = new(creation, design, async result =>
        {
            if (windowClosed || !ReferenceEquals(workspaceSheet, page) || sessionGroups != groups
                || model.ActiveProjectId != target.ProjectId || model.ActiveSessionId != target.SessionId) return;
            CloseSheet(); await RefreshExplorationsAsync(groups);
            if (!windowClosed && sessionGroups == groups && model.ActiveProjectId == target.ProjectId
                && model.ActiveSessionId == target.SessionId)
                await model.OpenExplorationAsync(host, target.ProjectId, result.Exploration.Id, result.Exploration.FrameId);
        }, () => { CloseSheet(); if (!windowClosed && sessionGroups == groups) _ = RefreshExplorationsAsync(groups); });
        MountSheet(page);
    }
    private async Task ReviewExplorationAsync(NativeExplorationSummary summary)
    {
        var project = model.ActiveProjectId; var groups = sessionGroups;
        var intent = model.SessionNavigationRevision; var ticket = ++explorationPreviewTicket;
        if (project == null || groups == null) return;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || model.ActiveProjectId != project || sessionGroups != groups
            || model.SessionNavigationRevision != intent || ticket != explorationPreviewTicket) return;
        ClearSheets();
        var review = new WorkspaceExplorationPreview(host, project, summary);
        NativeExplorationPreviewPage? page = null;
        page = new(review, design, async result =>
        {
            if (windowClosed || !ReferenceEquals(workspaceSheet, page) || sessionGroups != groups || model.ActiveProjectId != project) return;
            CloseSheet();
            var target = result.Action == "discard" && model.ActiveSessionId != result.SessionId ? model.ActiveSessionId : result.SourceSessionId;
            await model.OpenProjectAsync(project, target, selectFirst: false);
            if (!windowClosed && sessionGroups == groups) await RefreshExplorationsAsync(groups);
        }, CloseSheet);
        MountSheet(page); await review.LoadAsync();
    }
    private NativeSessionMenu ExplorationMenu(NativeExplorationSummary summary) => new(design,
        NativeSessionMenuEntries.Exploration(summary), entry =>
        {
            if (entry.Id == "open") _ = OpenExplorationAsync(summary);
            else _ = ReviewExplorationAsync(summary);
        }, Register);
    private async Task OpenExplorationAsync(NativeExplorationSummary summary)
    {
        var project = model.ActiveProjectId; var groups = sessionGroups; var navigation = model.NavigationRevision;
        var intent = model.SessionNavigationRevision; var ticket = ++explorationOpenTicket;
        if (project == null || groups == null) return;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || sessionGroups != groups || model.ActiveProjectId != project || model.NavigationRevision != navigation
            || model.SessionNavigationRevision != intent || ticket != explorationOpenTicket) return;
        CloseProjectPage();
        await model.OpenExplorationAsync(host, project, summary.Exploration.Id, summary.Exploration.FrameId);
    }
    private void MountSheet(IWorkspaceSheet sheet, bool nested = false)
    {
        if (sheet.IsModal && workspaceSheet == null)
            modalPreviousFocus = FocusManager.GetFocusedElement(root.XamlRoot) as Control;
        if (nested && workspaceSheet != null)
        {
            var focus = FocusManager.GetFocusedElement(root.XamlRoot) as Control;
            if (workspaceSheet is UserControl previous)
            {
                if (sheet.IsModal) previous.IsHitTestVisible = false;
                else SheetHost(workspaceSheet).Children.Remove(previous);
            }
            parentSheets.Push((workspaceSheet, focus, sheet.IsModal));
        }
        workspaceSheet = sheet;
        if (sheet is UserControl control) SheetHost(sheet).Children.Add(control);
        UpdateSheetPresentation();
    }
    private void UpdateSheetPresentation()
    {
        RefreshWindowTitlebar();
        refreshSidebarNavigation?.Invoke();
        if (settingsPage != null)
        {
            settingsPage.IsHitTestVisible = searchOverlay == null && workspaceSheet is not (NativeQuickSetupPage or NativePrivacyPage);
            return; // Keep the retained workspace hidden beneath Settings.
        }
        var keepsSidebar = workspaceSheet?.KeepsProjectSidebar == true;
        if (pageContent != null)
        {
            pageContent.IsHitTestVisible = searchOverlay == null && projectImport?.IsModal != true && projectExport?.IsModal != true && (workspaceSheet == null || keepsSidebar && !workspaceSheet.IsModal);
            pageContent.Visibility = workspaceSheet == null || workspaceSheet.IsModal || keepsSidebar ? Visibility.Visible : Visibility.Collapsed;
        }
        if (workspaceMain != null) workspaceMain.Visibility = keepsSidebar ? Visibility.Collapsed : Visibility.Visible;
        if (workspaceSheet is NativeJourneyPage journey)
            journey.SetWorkspaceInset(keepsSidebar ? PreviewLayout.Workspace(root.ActualWidth, sidebarVisible, false, settings.PanelWidth).SidebarWidth : 0);
    }
    private void ClearSheets()
    {
        while (parentSheets.TryPop(out var parent))
        {
            if (parent.Sheet is UserControl control) SheetHost(parent.Sheet).Children.Remove(control);
            parent.Sheet.Dispose();
        }
        CloseSheet();
    }
    private async Task OpenProjectSettingsAsync(string? projectId)
    {
        if (projectId == null || openingAction || nativePickerOpen || workspaceSheet != null || settingsPage != null || searchOverlay != null) return;
        openingAction = true;
        var database = model.DatabasePath; var openedProject = model.ActiveProjectId;
        var loading = new NativeConnectionPage(design, design.T("proj_settings.title"), CloseSheet,
            () => { CloseSheet(); _ = OpenProjectSettingsAsync(projectId); }, modal: true);
        MountSheet(loading);
        try
        {
            var host = await ConnectHostAsync();
            if (windowClosed || !ReferenceEquals(workspaceSheet, loading) || database != model.DatabasePath || openedProject != model.ActiveProjectId) return;
            if (host == null) { loading.Failed(localError ?? design.L("无法连接桌面服务。", "Could not connect to the desktop service.")); return; }
            var page = new NativeProjectSettingsPage(new(host, projectId, openedProject != null, host.ReconnectAsync, design.Localization), design,
                async () => { CloseSheet(); await model.RefreshAsync(); }, async () => await model.RefreshAsync(), CloseSheet, Register);
            CloseSheet(); MountSheet(page);
        }
        catch (Exception ex) { if (ReferenceEquals(workspaceSheet, loading)) loading.Failed(ex.Message); }
        finally { openingAction = false; }
    }
    private async Task OpenNativeAction(string kind)
    {
        if (workspaceSheet?.KeepsProjectSidebar == true && !workspaceSheet.IsModal) ClearSheets();
        if (kind == "import") { await OpenProjectImportAsync(); return; }
        if (projectImport?.IsModal == true) return;
        if (openingAction || nativePickerOpen || workspaceSheet != null || settingsPage != null || searchOverlay != null) return;
        openingAction = true;
        var project = model.ActiveProjectId; var session = model.ActiveSessionId; var database = model.DatabasePath;
        var title = kind switch { "shelved" => design.T("session.shelved"), "create" => design.T("projects.new"), "import" => "导入项目", "import-session" => "导入会话归档", "library" => design.T("library.title"), "calendar" => design.L("研究日历", "Research calendar"), "journey" => "研究历程", "publication" => "论文整理", "capabilities" => design.T("caps.title"), _ => "工作区" };
        if (kind is "import-cli" or "import-codex" or "import-claude")
            title = kind == "import-claude" ? design.T("claude.title") : kind == "import-codex" ? design.T("codex.title") : design.L("导入外部会话", "Import external conversations");
        var loading = new NativeConnectionPage(design, title, CloseSheet, () => { CloseSheet(); _ = OpenNativeAction(kind); }, modal: kind is "capabilities" or "create" or "shelved" or "import-cli" or "import-codex" or "import-claude");
        MountSheet(loading);
        try
        {
            var host = await ConnectHostAsync();
            if (windowClosed || !ReferenceEquals(workspaceSheet, loading) || database != model.DatabasePath || project != model.ActiveProjectId || session != model.ActiveSessionId) return;
            if (host == null) { loading.Failed(localError ?? "连接失败，请重试。"); return; }
            async Task OpenCreated(ProjectSummary row)
            {
                newProjectDraft = null;
                CloseSheet(); await model.RefreshAsync();
                await model.OpenProjectAsync(row.Id);
            }
            NativeNewProjectPage CreateProjectDialog()
            {
                var seed = newProjectDraft is { } saved && saved.Database == database ? saved.Draft : null;
                var form = new WorkspaceProjectCreation(new NativeProjectClient(host), design.T("projects.layout_context"), seed);
                if (seed != null) form.SetStandardLayout(seed.StandardLayout);
                return new(form, design, PickDirectory, OpenCreated, () =>
                {
                    if (model.DatabasePath == database && model.ActiveProjectId == null) newProjectDraft = (database, form.CaptureDraft());
                    CloseSheet();
                }, Register);
            }
            IWorkspaceSheet? page = kind switch
            {
                "assistant" => AssistantPage(host),
                "import-cli" or "import-codex" or "import-claude" when project != null => new NativeExternalSessionImportPage(new(new NativeExternalImportClient(host), project, design.Localization), model.Projects,
                    design, async (destination, imported) =>
                    {
                        if (windowClosed || database != model.DatabasePath || project != model.ActiveProjectId || session != model.ActiveSessionId) return;
                        CloseSheet(); await model.OpenProjectAsync(destination, imported);
                    }, CloseSheet, kind == "import-claude" ? "claude" : "codex", chooseDestination: kind == "import-cli"),
                "create" => CreateProjectDialog(),
                "shelved" when project != null => new NativeShelvedSessionsPage(new(host, project), design,
                    async item => { CloseSheet(); await model.OpenShelvedSessionAsync(project, item); },
                    async () => { if (!windowClosed && model.ActiveProjectId == project) await model.OpenProjectAsync(project, model.ActiveSessionId); }, CloseSheet, Register),
                "import-session" when project != null => new NativeSessionArchiveImportPage(new(new NativeSessionImportClient(host), project), model.Projects,
                    design, () => PickFile(".zip"), async (destination, imported) =>
                    {
                        if (windowClosed || database != model.DatabasePath || project != model.ActiveProjectId || session != model.ActiveSessionId) return;
                        // Both destinations already came from the project list.
                        // Open directly so a late refresh cannot override newer navigation.
                        CloseSheet(); await model.OpenProjectAsync(destination, imported);
                    }, CloseSheet),
                "library" => new NativeLibraryPage(new(new NativeLibraryClient(host)), design,
                    session == null ? null : text => { if (conversation?.Prefill(text, append: true) == true) CloseSheet(); },
                    async item => { CloseSheet(); await model.OpenProjectAsync(item.SourceProjectId, item.SourceSessionId); }, CloseSheet, Register),
                "calendar" => new NativeResearchCalendarPage(new(new NativeCalendarClient(host), new NativePrivacyClient(host), model.Projects), design,
                    (id, day) => MountSheet(new NativeJourneyPage(new NativeJourneyClient(host), id, model.Projects.FirstOrDefault(p => p.Id == id)?.Name ?? id, design, day, CloseSheet,
                        frame => MountSheet(new NativeJourneyConversationPage(new NativeConversationClient(host), id, frame, design, CloseSheet), nested: true), Register, false, UpdateSheetPresentation), nested: true), CloseSheet),
                "journey" when project != null => new NativeJourneyPage(new NativeJourneyClient(host), project, model.Projects.FirstOrDefault(p => p.Id == project)?.Name ?? project, design, null, CloseSheet,
                    frame => MountSheet(new NativeJourneyConversationPage(new NativeConversationClient(host), project, frame, design, CloseSheet), nested: true), Register, true, UpdateSheetPresentation),
                "capabilities" when project != null => new NativeCapabilitiesPage(host, project, design, section =>
                    { CloseSheet(); OpenSettingsSection(section, project); }, conversation, () => _ = StartEnvironmentSetupAsync(project), CloseSheet),
                _ => null
            };
            if (kind == "publication" && project != null)
            {
                CloseSheet();
                ShowPublication(host, project); return;
            }
            if (page != null) { CloseSheet(); MountSheet(page); }
        }
        catch (Exception ex) { if (ReferenceEquals(workspaceSheet, loading)) loading.Failed(ex.Message); }
        finally { openingAction = false; }
    }
    private void ShowPublication(NativeSettingsClient host, string project, PublicationRunSource? run = null)
    {
        var publication = new WorkspacePublicationModel(new NativePublicationClient(host), project);
        if (run != null) publication.BeginRunBinding(run);
        projectPage?.Dispose();
        projectPage = new NativePublicationPage(publication, design, PickDirectory, CloseProjectPage);
        Render();
    }
    private void CloseProjectPage() { projectPage?.Dispose(); projectPage = null; Render(); }

    /// <summary>Run a session mutation on the groups model, then refresh the sidebar.
    /// Failed or ambiguous mutations stay on the session list without retry.</summary>
    private async Task SessionMutationAsync(Task<bool> mutation, string? refreshSession = null)
    {
        var project = model.ActiveProjectId; var groups = sessionGroups;
        var active = refreshSession ?? model.ActiveSessionId;
        var ok = await mutation;
        if (project == null || model.ActiveProjectId != project || sessionGroups != groups) return;
        if (ok) await model.OpenProjectAsync(project, active, selectFirst: false); else Render();
    }

    private async Task PromptSessionRenameAsync(BrowserSession session)
    {
        if (workspaceSheet != null || sessionGroups is not { Busy: false } groups || model.ActiveProjectId != session.ProjectId) return;
        var host = await ConnectHostAsync();
        if (host == null || workspaceSheet != null || sessionGroups != groups || model.ActiveProjectId != session.ProjectId) return;
        var rename = new WorkspaceSessionRename(host, session.ProjectId, session.Id, NativeSessionTree.Title(session, design.Localization));
        void Close(bool refresh)
        {
            CloseSheet();
            if (refresh && model.ActiveProjectId == session.ProjectId) _ = model.OpenProjectAsync(session.ProjectId, model.ActiveSessionId, selectFirst: false);
        }
        MountSheet(new NativeSessionRenamePage(rename, design, () => Close(true), () => Close(rename.MutationUnconfirmed), Register));
    }

    private async Task ShelveSessionAsync(BrowserSession session, WorkspaceSessionGroups groups)
    {
        if (session.ProjectId != model.ActiveProjectId || groups != sessionGroups) return;
        var ok = await groups.ShelveAsync(session.Id);
        if (session.ProjectId != model.ActiveProjectId || groups != sessionGroups) return;
        if (ok) await model.RefreshAfterShelvingAsync(session); else Render();
    }

    private NativeSessionMenu SessionMenu(BrowserSession session, WorkspaceSessionGroups groups)
    {
        var menu = new NativeSessionMenu(design, NativeSessionMenuEntries.Conversation(session, groups, stalePrompt: null), entry =>
        {
            if (session.ProjectId != model.ActiveProjectId || groups != sessionGroups) return;
            switch (entry.Id)
            {
                case "copy":
                    try { var data = new DataPackage(); data.SetText(NativeSessionTree.Title(session, design.Localization)); Clipboard.SetContent(data); }
                    catch (Exception error) { groups.Fail(error.Message); Render(); }
                    break;
                case "open": CloseProjectPage(); _ = model.OpenSessionAsync(session.Id); break;
                case "reload_rules": _ = PromptSessionRulesAsync(session, groups); break;
                case "merge_branch": _ = OpenBranchMergeAsync(session); break;
                case "pin": _ = SessionMutationAsync(groups.PinAsync(session.Id, session.Pinned != true)); break;
                case "shelve": _ = ShelveSessionAsync(session, groups); break;
                case "rename": _ = PromptSessionRenameAsync(session); break;
                case "move": _ = SessionMutationAsync(groups.MoveSessionAsync(session.Id, entry.Payload)); break;
                case "copy_project": _ = PromptSessionTransferAsync(session, "copy"); break;
                case "move_project": _ = PromptSessionTransferAsync(session, "move"); break;
                case "export": _ = ExportSessionAsync(session); break;
                case "export_debug": _ = ExportDebugRequestAsync(session); break;
                case "delete": _ = PromptSessionDeleteAsync(session); break;
            }
        }, Register);
        var generation = 0;
        menu.Opened += async (_, _) =>
        {
            var current = ++generation;
            menu.SetEntryVisible("reload_rules", false);
            try
            {
                var host = await ConnectHostAsync();
                if (host == null || current != generation || windowClosed || groups != sessionGroups || model.ActiveProjectId != session.ProjectId) return;
                var stale = await WorkspaceSessionRules.ReadStaleAsync(host, session.ProjectId, session.Id);
                if (current == generation && !windowClosed && groups == sessionGroups && model.ActiveProjectId == session.ProjectId)
                    menu.SetEntryVisible("reload_rules", stale);
            }
            catch (Exception error) { if (current == generation && groups == sessionGroups) groups.Fail(error.Message); }
        };
        menu.Closed += (_, _) => generation++;
        return menu;
    }

    private async Task PromptSessionRulesAsync(BrowserSession session, WorkspaceSessionGroups groups)
    {
        if (windowClosed || workspaceSheet != null || nativePickerOpen || settingsPage != null || groups.Busy || groups != sessionGroups || model.ActiveProjectId != session.ProjectId) return;
        var revision = model.NavigationRevision;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || workspaceSheet != null || nativePickerOpen || settingsPage != null || revision != model.NavigationRevision || groups != sessionGroups || model.ActiveProjectId != session.ProjectId) return;
        var state = new WorkspaceSessionRules(host, session.ProjectId, session.Id);
        MountSheet(new NativeSessionRulesPage(state, design, () =>
        {
            if (!state.Closed && !windowClosed && groups == sessionGroups && model.ActiveProjectId == session.ProjectId) CloseSheet();
        }, CloseSheet));
    }

    private async Task PromptSessionTransferAsync(BrowserSession session, string mode)
    {
        if (workspaceSheet != null || nativePickerOpen || sessionGroups is not { Busy: false } groups || model.ActiveProjectId != session.ProjectId) return;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || workspaceSheet != null || sessionGroups != groups || model.ActiveProjectId != session.ProjectId) return;
        var transfer = new WorkspaceSessionTransfer(host, session, mode, model.Projects);
        void Close(bool saved)
        {
            var uncertain = transfer.MutationUnconfirmed; var result = transfer.Result;
            var targetName = transfer.Targets.FirstOrDefault(p => p.Id == result?.TargetProjectId)?.Name ?? "";
            CloseSheet();
            if (model.ActiveProjectId != session.ProjectId) return;
            if (saved) { _ = model.RefreshAfterTransferAsync(session, mode == "move"); _ = ShowSessionNoticeAsync(design.T(mode == "move" ? "session.move_success" : "session.copy_success").Replace("{project}", targetName)); }
            else if (uncertain) _ = model.RefreshAsync(selectFirst: false);
        }
        MountSheet(new NativeSessionTransferPage(transfer, design, () => Close(true), () => Close(false)));
    }

    private bool sessionExporting;
    private async Task ExportDebugRequestAsync(BrowserSession session)
    {
        if (sessionExporting || nativePickerOpen || workspaceSheet != null || model.ActiveProjectId != session.ProjectId) return;
        sessionExporting = true;
        var navigation = model.NavigationRevision;
        bool Current() => !windowClosed && model.ActiveProjectId == session.ProjectId && model.NavigationRevision == navigation && settingsPage == null && workspaceSheet == null;
        try
        {
            var host = await ConnectHostAsync(); if (host == null || !Current()) return;
            using var export = new WorkspaceDebugRequestExport(host, session);
            await export.ExportAsync(PickFileDownload, (path, bytes) => System.IO.File.WriteAllBytesAsync(path, bytes), Current);
            if (!Current()) return;
            if (export.SavedPath is { } path) _ = ShowShareSavedAsync(path);
            else if (export.Error is { } error) _ = ShowSessionNoticeAsync(error, failure: true);
        }
        catch (Exception error) { if (Current()) _ = ShowSessionNoticeAsync(error.Message, failure: true); }
        finally { sessionExporting = false; }
    }

    private async Task ExportSessionAsync(BrowserSession session)
    {
        if (sessionExporting || nativePickerOpen || workspaceSheet != null || model.ActiveProjectId != session.ProjectId) return;
        sessionExporting = true;
        var navigation = model.NavigationRevision;
        bool Current() => !windowClosed && model.ActiveProjectId == session.ProjectId && model.NavigationRevision == navigation && settingsPage == null && workspaceSheet == null;
        try
        {
            var host = await ConnectHostAsync(); if (host == null || !Current()) return;
            using var export = new WorkspaceSessionExport(host, session);
            await export.ExportAsync(PickFileDownload, Current);
            if (!Current()) return;
            if (export.Result is { } result) _ = ShowShareSavedAsync(result.DestinationPath);
            else if (export.Error is { } error) _ = ShowSessionNoticeAsync(error, failure: true);
        }
        catch (Exception error) { if (Current()) _ = ShowSessionNoticeAsync(error.Message, failure: true); }
        finally { sessionExporting = false; }
    }

    private async Task ShowSessionNoticeAsync(string message, bool failure = false)
    {
        var notice = new InfoBar { IsOpen = true, IsClosable = false, Severity = failure ? InfoBarSeverity.Error : InfoBarSeverity.Success,
            Message = message, MaxWidth = 600, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(20) };
        root.Children.Add(notice); Canvas.SetZIndex(notice, 100);
        await Task.Delay(TimeSpan.FromSeconds(5)); root.Children.Remove(notice);
    }

    private Task PromptSessionDeleteAsync(BrowserSession session) => PromptSessionDeleteAsync([session.Id]);
    private async Task PromptSessionDeleteAsync(string[] ids)
    {
        if (workspaceSheet != null || sessionGroups is not { Busy: false } groups || model.ActiveProjectId is not { } project) return;
        var host = await ConnectHostAsync();
        if (host == null || workspaceSheet != null || sessionGroups != groups || model.ActiveProjectId != project) return;
        var deletion = new WorkspaceSessionDeletion(host, project, ids);
        void Close()
        {
            var changed = deletion.Removed.Count > 0 || deletion.MutationUnconfirmed;
            var active = model.ActiveSessionId;
            if (active != null && deletion.Removed.Contains(active)) active = null;
            groups.Selected.ExceptWith(deletion.Removed);
            if (groups.Selected.Count == 0) groups.Selecting = false;
            CloseSheet();
            if (changed && model.ActiveProjectId == project) _ = model.OpenProjectAsync(project, active, selectFirst: false);
        }
        MountSheet(new NativeSessionDeletePage(deletion, design, Close));
        await deletion.LoadAsync();
    }

    private async Task EditGroup(string? id = null)
    {
        if (workspaceSheet != null || settingsPage != null || sessionGroups == null || sessionGroups.Busy) return;
        sessionGroups.BeginFolderEdit(id);
        var originalFolders = sessionGroups.Folders;
        MountSheet(new NativeSessionGroupPage(sessionGroups, design, () =>
        {
            var changed = !ReferenceEquals(originalFolders, sessionGroups?.Folders);
            CloseSheet();
            if (changed) Render(); // Cancellation preserves the invoking control and keyboard focus.
        }, Register));
        await Task.CompletedTask;
    }
    private NativeSessionMenu FolderMenu(string folder, WorkspaceSessionGroups groups)
    {
        var project = model.ActiveProjectId;
        return new NativeSessionMenu(design, NativeSessionMenuEntries.Folder(folder, groups), entry =>
        {
            if (windowClosed || workspaceSheet != null || groups != sessionGroups || project != model.ActiveProjectId) return;
            if (entry.Id == "rename") _ = EditGroup(folder);
            else if (entry.Id == "delete" && groups.CanDeleteFolder(folder))
            {
                groups.BeginFolderDelete();
                MountSheet(new NativeFolderDeletePage(groups, folder, design, async () =>
                {
                    if (groups != sessionGroups || project != model.ActiveProjectId) return;
                    CloseSheet();
                    if (project != null) await RefreshSessionGrouping(groups, project);
                }, CloseSheet));
            }
        }, Register);
    }
    private async Task RefreshSessionGrouping(WorkspaceSessionGroups groups, string project)
    {
        if (windowClosed || groups != sessionGroups || project != model.ActiveProjectId) return;
        await model.OpenProjectAsync(project, model.ActiveSessionId, selectFirst: false);
        if (!windowClosed && groups == sessionGroups && project == model.ActiveProjectId) Render();
    }
    private FrameworkElement SessionList()
    {
        var rail = layout.CompactWorkspace;
        if (sessionGroups is { Selecting: true } selection)
            return new NativeSessionSelection(selection, model.Sessions, design,
                () => _ = PromptSessionDeleteAsync(selection.Selected.ToArray()),
                () => { selection.Selecting = false; selection.Selected.Clear(); Render(); },
                async folder =>
                {
                    await selection.MoveAsync(folder);
                    if (sessionGroups == selection && model.ActiveProjectId is { } project)
                        await model.OpenProjectAsync(project, model.ActiveSessionId, selectFirst: false);
                }, Register, model.ActiveSessionId, rail);
        var section = new Grid { RowSpacing = 6 };
        section.RowDefinitions.Add(new() { Height = GridLength.Auto });
        section.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var controls = Stack(4);
        var sectionHeader = new Grid { Padding = new Thickness(10, 10, 10, 4) };
        sectionHeader.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        sectionHeader.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var count = Text(design.T("sidebar.sessions"), 11, "text-faint");
        count.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        count.VerticalAlignment = VerticalAlignment.Center;
        sectionHeader.Children.Add(count); controls.Children.Add(sectionHeader);
        if (sessionGroups is { } groups)
        {
            var options = Row(3);
            var select = new Button { Content = Text(design.T(groups.Selecting ? "settings.cancel" : "sidebar.select_sessions"), 11, "text-faint"), IsEnabled = groups.Selecting || model.Sessions.Count > 0 };
            design.QuietButton(select); select.Padding = new Thickness(4, 6, 4, 6); select.MinWidth = select.MinHeight = 0;
            select.Opacity = select.IsEnabled ? 1 : .45;
            select.Click += (_, _) => { groups.Selecting = !groups.Selecting; groups.Selected.Clear(); Render(); }; options.Children.Add(select);
            if (!groups.Selecting)
            {
                var shelved = SessionToolbarButton(design.T("session.shelved"), "eye-off");
                shelved.Click += (_, _) => _ = OpenNativeAction("shelved"); options.Children.Add(shelved);
                var arrange = SessionToolbarButton(design.T("sidebar.sort_group"), "adjustments");
                arrange.Flyout = SessionArrangementMenu(groups); options.Children.Add(arrange);
            }
            Grid.SetColumn(options, 1); sectionHeader.Children.Add(options);
            if (groups.Error != null) controls.Children.Add(Text(groups.Error, 11, "clay-strong"));
            if (groups.ExplorationError != null) controls.Children.Add(Text(groups.ExplorationError, 11, "clay-strong"));
        }
        if (rail) sectionHeader.Visibility = Visibility.Collapsed;
        section.Children.Add(controls);
        var list = Stack(4);
        if (!rail && model.Sessions.Count == 0 && sessionGroups?.Folders.Length is not > 0 && !model.Loading)
        {
            var empty = Text(design.T("sidebar.no_sessions"), 12, "text-faint"); empty.Margin = new Thickness(10, 12, 10, 0);
            list.Children.Add(empty);
        }
        var sidebarRows = model.Sessions.ToArray();
        NativeSessionSidebarDrag? sidebarDrag = null;
        if (sessionGroups is { } dragGroups && model.ActiveProjectId is { } dragProject)
            sidebarDrag = new(dragProject, dragGroups, sidebarRows, design,
                () => !windowClosed && dragGroups == sessionGroups && model.ActiveProjectId == dragProject,
                () => RefreshSessionGrouping(dragGroups, dragProject));
        sessionTree = new NativeSessionTreeView(sidebarRows, sessionGroups, design, model.ActiveSessionId,
            session => { CloseProjectPage(); _ = model.OpenSessionAsync(session.Id); },
            session => sessionGroups is { } mutationGroups ? SessionMenu(session, mutationGroups) : null,
            (source, kind, id) => { if (sidebarDrag != null) sidebarDrag.Source(source, kind, id); else EnableSessionReferenceDrag(source, kind, id); },
            folder => _ = EditGroup(folder), folder => sessionGroups is { } folderGroups ? FolderMenu(folder, folderGroups) : null,
            (target, folder) => sidebarDrag?.Target(target, folder),
            exploration => _ = OpenExplorationAsync(exploration), ExplorationMenu, rail);
        list.Children.Add(sessionTree);
        var scroll = new ScrollViewer { Content = list, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollBarVisibility = rail ? ScrollBarVisibility.Hidden : ScrollBarVisibility.Auto };
        Grid.SetRow(scroll, 1); section.Children.Add(scroll); return section;
    }
    private void EnableSessionReferenceDrag(FrameworkElement source, string kind, string id)
    {
        var project = model.ActiveProjectId;
        source.CanDrag = project != null;
        if (source is Panel panel) panel.Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
        source.DragStarting += (_, e) =>
        {
            if (windowClosed || project == null || model.ActiveProjectId != project || model.Loading)
            { e.Cancel = true; return; }
            e.Data.RequestedOperation = DataPackageOperation.Copy;
            e.Data.SetData(NativeSessionDrag.Format, JsonSerializer.Serialize(new NativeSessionDrag(project, kind, id)));
        };
    }
    private void ShowFiles() => ShowPanelTab("files");

    /// <summary>Open the side panel on one tab; shared by the files shortcut
    /// and the composer slash/环境 entries.</summary>
    private void ShowPanelTab(string tab)
    {
        if (tab == "memos") { ClearSheets(); CloseProjectPage(); }
        runNavigation++;
        var tabs = new NativePanelTabs(settings.PanelTabs, settings.PanelTab, NativePanelTabs.All);
        tabs.Show(tab);
        settings.PanelTabs = tabs.Saved; settings.PanelTab = tabs.Selected;
        panelVisible = true; settings.PanelVisible = true; SaveSettings();
        if (panelPage == null) _ = EnsurePanelAndTerminalAsync();
        else _ = panelPage.ShowTabAsync(tab);
        Render();
    }

    private async Task OpenRunAsync(string runId, bool review = false)
    {
        if (model.ActiveProjectId is not { } project || model.ActiveSessionId is not { } session) return;
        var navigation = ++runNavigation;
        var tabs = new NativePanelTabs(settings.PanelTabs, settings.PanelTab, NativePanelTabs.All);
        tabs.Show("runs");
        settings.PanelTabs = tabs.Saved; settings.PanelTab = tabs.Selected;
        panelVisible = true; settings.PanelVisible = true; SaveSettings();
        await EnsurePanelAndTerminalAsync();
        if (windowClosed || navigation != runNavigation || !panelVisible
            || project != model.ActiveProjectId || session != model.ActiveSessionId || panelPage is null) return;
        Render();
        if (review) await panelPage.ShowRunReviewAsync(runId); else await panelPage.ShowRunAsync(runId);
    }

    private async Task OpenGeneratedAsync(NativeGeneratedOutput output)
    {
        if (model.ActiveProjectId != output.ProjectId || model.ActiveSessionId != output.SessionId) return;
        var navigation = ++runNavigation;
        var tabs = new NativePanelTabs(settings.PanelTabs, settings.PanelTab, NativePanelTabs.All); tabs.Show("artifacts");
        settings.PanelTabs = tabs.Saved; settings.PanelTab = tabs.Selected; panelVisible = true; settings.PanelVisible = true; SaveSettings();
        await EnsurePanelAndTerminalAsync();
        if (windowClosed || navigation != runNavigation || !panelVisible || model.ActiveProjectId != output.ProjectId
            || model.ActiveSessionId != output.SessionId || panelPage == null) return;
        Render(); await panelPage.ShowGeneratedAsync(output);
    }

    /// <summary>Composer slash mapping. Only commands with a working native
    /// surface are routed; anything else falls back to the hint text.</summary>
    private bool RouteSlashCommand(string draft)
    {
        var command = draft.Split([' ', '\n'], 2)[0].ToLowerInvariant();
        switch (command)
        {
            case "/files": ShowPanelTab("files"); return true;
            case "/outline": _ = OpenSheet("outline"); return true;
            case "/share": _ = OpenSheet("share"); return true;
            case "/trajectory": _ = OpenSheet("trajectory"); return true;
            case "/archive": _ = OpenSheet("archive"); return true;
            case "/library": _ = OpenNativeAction("library"); return true;
            case "/calendar": _ = OpenNativeAction("calendar"); return true;
            case "/journey": _ = OpenNativeAction("journey"); return true;
            case "/publication": _ = OpenNativeAction("publication"); return true;
            case "/settings": OpenSettings(); return true;
            default: return false;
        }
    }
    private async Task<IReadOnlyList<string>> PickAttachments()
    {
        if (nativePickerOpen) return [];
        nativePickerOpen = true;
        try
        {
            var picker = new FileOpenPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
            picker.FileTypeFilter.Add("*");
            return (await picker.PickMultipleFilesAsync()).Select(file => file.Path).ToArray();
        }
        finally { nativePickerOpen = false; }
    }
    private async Task<string?> PickFile(params string[] extensions)
    {
        if (nativePickerOpen) return null;
        nativePickerOpen = true;
        try
        {
            var picker = new FileOpenPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
            foreach (var extension in extensions) picker.FileTypeFilter.Add(extension);
            return (await picker.PickSingleFileAsync())?.Path;
        }
        catch (Exception ex) { localError = ex.Message; return null; }
        finally { nativePickerOpen = false; }
    }
    private async Task<string?> PickDirectory()
    {
        if (nativePickerOpen) return null;
        nativePickerOpen = true;
        try
        {
            var picker = new FolderPicker(); picker.FileTypeFilter.Add("*");
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
            return (await picker.PickSingleFolderAsync())?.Path;
        }
        catch (Exception ex) { localError = ex.Message; return null; }
        finally { nativePickerOpen = false; }
    }
    private void OpenTutorials()
    {
        try { Process.Start(new ProcessStartInfo("https://wispscience.com/tutorials.html") { UseShellExecute = true }); }
        catch (Exception ex) { localError = ex.Message; Render(); }
    }
}
