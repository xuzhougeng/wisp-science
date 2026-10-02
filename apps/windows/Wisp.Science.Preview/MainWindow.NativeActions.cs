using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Windows.Storage.Pickers;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private WorkspaceSessionGroups? sessionGroups;
    private string? actionProject;
    private IWorkspaceSheet? projectPage;
    private bool openingAction;
    private readonly Stack<IWorkspaceSheet> parentSheets = new();

    private void SyncProjectActions()
    {
        if (actionProject == model.ActiveProjectId) return;
        actionProject = model.ActiveProjectId;
        sessionGroups?.Dispose(); sessionGroups = null;
        projectPage?.Dispose(); projectPage = null;
        if (actionProject is { } project) _ = LoadGroups(project);
    }
    private async Task LoadGroups(string project)
    {
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || actionProject != project) return;
        var groups = new WorkspaceSessionGroups(host, project);
        sessionGroups = groups;
        await groups.LoadAsync();
        if (!windowClosed && sessionGroups == groups) Render();
    }
    private void MountSheet(IWorkspaceSheet sheet, bool nested = false)
    {
        if (nested && workspaceSheet != null)
        {
            if (workspaceSheet is UserControl previous) root.Children.Remove(previous);
            parentSheets.Push(workspaceSheet);
        }
        workspaceSheet = sheet;
        if (pageContent != null) { pageContent.IsHitTestVisible = false; pageContent.Visibility = Visibility.Collapsed; }
        if (sheet is UserControl control) root.Children.Add(control);
    }
    private void ClearSheets()
    {
        while (parentSheets.TryPop(out var parent)) parent.Dispose();
        CloseSheet();
    }
    private async Task OpenNativeAction(string kind)
    {
        if (openingAction || nativePickerOpen || workspaceSheet != null || settingsPage != null || searchOverlay != null) return;
        openingAction = true;
        var project = model.ActiveProjectId; var session = model.ActiveSessionId; var database = model.DatabasePath;
        try
        {
            var host = await ConnectHostAsync();
            if (host == null || windowClosed || database != model.DatabasePath || project != model.ActiveProjectId || session != model.ActiveSessionId) return;
            async Task OpenCreated(ProjectSummary row)
            {
                CloseSheet(); await model.RefreshAsync();
                await model.OpenProjectAsync(row.Id);
            }
            IWorkspaceSheet? page = kind switch
            {
                "create" => new NativeNewProjectPage(new(new NativeProjectClient(host)), design, PickDirectory, OpenCreated, CloseSheet),
                "import" => new NativeImportProjectPage(new(new NativeProjectClient(host)), design, () => PickFile(".zip"), OpenCreated, CloseSheet),
                "library" => new NativeLibraryPage(new(new NativeLibraryClient(host)), design,
                    session == null ? null : item => { if (conversation?.Prefill(WorkspaceLibraryModel.ComposerText(item), append: true) == true) CloseSheet(); },
                    async item => { CloseSheet(); await model.OpenProjectAsync(item.SourceProjectId, item.SourceSessionId); }, CloseSheet),
                "calendar" => new NativeResearchCalendarPage(new(new NativeCalendarClient(host), new NativePrivacyClient(host), model.Projects), design,
                    (id, day) => MountSheet(new NativeJourneyPage(new NativeJourneyClient(host), id, design, day, CloseSheet,
                        frame => MountSheet(new NativeJourneyConversationPage(new NativeConversationClient(host), id, frame, design, CloseSheet), nested: true)), nested: true), CloseSheet),
                "journey" when project != null => new NativeJourneyPage(new NativeJourneyClient(host), project, design, null, CloseSheet,
                    frame => MountSheet(new NativeJourneyConversationPage(new NativeConversationClient(host), project, frame, design, CloseSheet), nested: true)),
                "capabilities" when project != null => new NativeCapabilitiesPage(host, project, design, section =>
                    { CloseSheet(); OpenSettingsSection(section); }, CloseSheet),
                _ => null
            };
            if (kind == "publication" && project != null)
            {
                projectPage?.Dispose();
                projectPage = new NativePublicationPage(new(new NativePublicationClient(host), project), design, CloseProjectPage);
                Render(); return;
            }
            if (kind == "scratch")
            {
                var scratch = new WorkspaceScratchModel(new NativeScratchClient(host));
                if (!await scratch.OpenAsync()) { localError = scratch.Error; scratch.Dispose(); Render(); return; }
                if (windowClosed || database != model.DatabasePath || project != model.ActiveProjectId || session != model.ActiveSessionId)
                {
                    await scratch.CloseAsync(); scratch.Dispose(); return;
                }
                page = new NativeScratchPage(scratch, host, design, () => PickFile("*"), CloseSheet);
            }
            if (page != null) MountSheet(page);
        }
        catch (Exception ex) { localError = ex.Message; Render(); }
        finally { openingAction = false; }
    }
    private void CloseProjectPage() { projectPage?.Dispose(); projectPage = null; Render(); }

    /// <summary>Run a session mutation on the groups model, then refresh the sidebar.
    /// Failed or ambiguous mutations stay on the session list without retry.</summary>
    private async Task SessionMutationAsync(Task<bool> mutation, string? refreshSession = null)
    {
        var ok = await mutation;
        if (!ok || model.ActiveProjectId is not { } current) return;
        await model.OpenProjectAsync(current, refreshSession ?? model.ActiveSessionId);
    }

    private async Task PromptSessionRenameAsync(BrowserSession session)
    {
        if (sessionGroups == null || sessionGroups.Busy) return;
        var name = new TextBox { Text = session.Title, PlaceholderText = "会话名称" };
        var dialog = new ContentDialog
        {
            Title = "重命名会话",
            Content = name,
            PrimaryButtonText = "保存",
            CloseButtonText = "取消",
            XamlRoot = root.XamlRoot
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        await SessionMutationAsync(sessionGroups.RenameAsync(session.Id, name.Text), session.Id == model.ActiveSessionId ? session.Id : null);
    }

    private async Task PromptSessionDeleteAsync(BrowserSession session)
    {
        if (sessionGroups == null || sessionGroups.Busy) return;
        var dialog = new ContentDialog
        {
            Title = "删除会话",
            Content = $"永久删除“{session.Title}”？这个会话的运行会先被停止，删除无法撤销。",
            PrimaryButtonText = "删除",
            CloseButtonText = "取消",
            XamlRoot = root.XamlRoot
        };
        if (await dialog.ShowAsync() != ContentDialogResult.Primary) return;
        await SessionMutationAsync(sessionGroups.DeleteAsync(session.Id), session.Id == model.ActiveSessionId ? null : model.ActiveSessionId);
    }

    private async Task EditGroup(string? id = null)
    {
        if (workspaceSheet != null || settingsPage != null || sessionGroups == null || sessionGroups.Busy) return;
        sessionGroups.RenamingId = id;
        sessionGroups.Draft = id == null ? "" : sessionGroups.Folders.FirstOrDefault(f => f.Id == id)?.Name ?? "";
        MountSheet(new NativeSessionGroupPage(sessionGroups, design, () => { CloseSheet(); Render(); }));
        await Task.CompletedTask;
    }
    private FrameworkElement SessionList()
    {
        var section = new Grid { RowSpacing = 6 };
        section.RowDefinitions.Add(new() { Height = GridLength.Auto });
        section.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var controls = Stack(4);
        controls.Children.Add(Text($"会话   {model.Sessions.Count}", 11, "text-faint"));
        if (sessionGroups is { } groups)
        {
            var options = Row(4);
            var arrange = ActionButton("分组和排序", "list", () => { }, quiet: true);
            var menu = new MenuFlyout();
            foreach (var (label, value, sort) in new[] { ("最近更新", "newest", true), ("按名称排序", "name", true), ("不分组", "none", false), ("按文件夹分组", "folder", false), ("按日期分组", "date", false) })
            {
                var choice = new ToggleMenuFlyoutItem { Text = label, IsChecked = (sort ? groups.Sort : groups.Group) == value };
                choice.Click += (_, _) => { if (sort) groups.Sort = value; else groups.Group = value; Render(); };
                menu.Items.Add(choice);
            }
            Register(menu); arrange.Flyout = menu; options.Children.Add(arrange);
            options.Children.Add(ActionButton(groups.Selecting ? "取消选择" : "选择会话", "check", () => { groups.Selecting = !groups.Selecting; groups.Selected.Clear(); Render(); }, true, quiet: true));
            if (groups.Selecting)
            {
                var move = ActionButton("移动", "folder", () => { }, quiet: true);
                move.IsEnabled = groups.Selected.Count > 0 && !groups.Busy;
                var destinations = new MenuFlyout();
                foreach (var folder in groups.Folders.Append(new ProjectFolder("", "未分组")))
                {
                    var option = new MenuFlyoutItem { Text = folder.Name };
                    option.Click += async (_, _) =>
                    {
                        await groups.MoveAsync(folder.Id.Length == 0 ? null : folder.Id);
                        if (sessionGroups == groups && model.ActiveProjectId is { } current) await model.OpenProjectAsync(current, model.ActiveSessionId);
                    };
                    destinations.Items.Add(option);
                }
                Register(destinations); move.Flyout = destinations; options.Children.Add(move);
            }
            controls.Children.Add(options);
            if (groups.Error != null) controls.Children.Add(Text(groups.Error, 11, "clay-strong"));
        }
        section.Children.Add(controls);
        var list = Stack(4);
        foreach (var group in sessionGroups?.Sections(model.Sessions) ?? [new SessionSection("会话", null, model.Sessions.ToArray())])
        {
            if (sessionGroups?.Group != "none" && sessionGroups != null)
            {
                var heading = Row(4); heading.Children.Add(Text(group.Title, 11, "text-muted"));
                if (group.FolderId != null) heading.Children.Add(ActionButton("重命名分组", "edit", () => _ = EditGroup(group.FolderId), quiet: true));
                list.Children.Add(heading);
            }
            foreach (var session in group.Sessions)
            {
                if (sessionGroups is { Selecting: true } selection)
                {
                    var check = new CheckBox { Content = SingleLine(session.Title, 12), IsChecked = selection.Selected.Contains(session.Id) };
                    check.Checked += (_, _) => { selection.Selected.Add(session.Id); Render(); };
                    check.Unchecked += (_, _) => { selection.Selected.Remove(session.Id); Render(); };
                    list.Children.Add(check);
                }
                else
                {
                    var row = new Grid();
                    row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
                    row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                    var label = Stack(3); label.Children.Add(SingleLine(session.Title, 12)); label.Children.Add(SessionMetadata(session));
                    var button = ContentButton(label, () => { CloseProjectPage(); _ = model.OpenSessionAsync(session.Id); }, "session-" + session.Id,
                        session.Title + " · " + NativeBrowserPresentation.Status(session.Status));
                    button.Padding = new Thickness(8, 6, 8, 6);
                    if (session.Id == model.ActiveSessionId) button.Background = design.Brush("surface-hover");
                    row.Children.Add(button);
                    if (sessionGroups is { } mutationGroups)
                    {
                        var more = ActionButton("会话操作", "more", () => { }, quiet: true);
                        var menu = new MenuFlyout();
                        var rename = new MenuFlyoutItem { Text = "重命名会话" };
                        rename.Click += (_, _) => _ = PromptSessionRenameAsync(session);
                        menu.Items.Add(rename);
                        var pin = new MenuFlyoutItem { Text = session.Pinned == true ? "取消置顶" : "置顶" };
                        pin.IsEnabled = session.Pinned != null;
                        pin.Click += (_, _) => _ = SessionMutationAsync(mutationGroups.PinAsync(session.Id, session.Pinned != true),
                            refreshSession: session.Id == model.ActiveSessionId ? session.Id : null);
                        menu.Items.Add(pin);
                        var delete = new MenuFlyoutItem { Text = "删除会话…" };
                        delete.Click += (_, _) => _ = PromptSessionDeleteAsync(session);
                        menu.Items.Add(delete);
                        Register(menu); more.Flyout = menu;
                        Grid.SetColumn(more, 1); row.Children.Add(more);
                    }
                    list.Children.Add(row);
                }
            }
        }
        var scroll = new ScrollViewer { Content = list, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        Grid.SetRow(scroll, 1); section.Children.Add(scroll); return section;
    }
    private void ShowFiles() => ShowPanelTab("files");

    /// <summary>Open the side panel on one tab; shared by the files shortcut
    /// and the composer slash/环境 entries.</summary>
    private void ShowPanelTab(string tab)
    {
        runNavigation++;
        var tabs = new NativePanelTabs(settings.PanelTabs, settings.PanelTab, NativePanelTabs.All);
        tabs.Show(tab);
        settings.PanelTabs = tabs.Saved; settings.PanelTab = tabs.Selected;
        panelVisible = true; settings.PanelVisible = true; SaveSettings();
        if (panelPage == null) _ = EnsurePanelAndTerminalAsync();
        else _ = panelPage.ShowTabAsync(tab);
        Render();
    }

    private async Task OpenRunAsync(string runId)
    {
        if (model.ActiveProjectId is not { } project || model.ActiveSessionId is not { } session) return;
        var navigation = ++runNavigation;
        var tabs = new NativePanelTabs(settings.PanelTabs, settings.PanelTab, NativePanelTabs.All);
        tabs.Show("hosts");
        settings.PanelTabs = tabs.Saved; settings.PanelTab = tabs.Selected;
        panelVisible = true; settings.PanelVisible = true; SaveSettings();
        await EnsurePanelAndTerminalAsync();
        if (windowClosed || navigation != runNavigation || !panelVisible
            || project != model.ActiveProjectId || session != model.ActiveSessionId || panelPage is null) return;
        Render();
        await panelPage.ShowRunAsync(runId);
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
            case "/scratch": _ = OpenNativeAction("scratch"); return true;
            default: return false;
        }
    }
    private async Task<string?> PickFile(string extension)
    {
        if (nativePickerOpen) return null;
        nativePickerOpen = true;
        try
        {
            var picker = new FileOpenPicker();
            WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
            picker.FileTypeFilter.Add(extension);
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
