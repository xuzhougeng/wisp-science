using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private void OpenBranchResult(string title, string summary)
    {
        if (windowClosed || workspaceSheet != null || settingsPage != null || nativePickerOpen) return;
        MountSheet(new NativeBranchResultPage(title, summary, design, CloseSheet, Register));
    }
    private void OpenTranscriptBranch(string id)
    {
        if (windowClosed || workspaceSheet != null || settingsPage != null || nativePickerOpen
            || model.ActiveProjectId is not { } project || conversation?.CurrentProjectId != project) return;
        if (conversation.Snapshot?.Branches?.Any(branch => branch.Id == id && branch.SourceSessionId == model.ActiveSessionId) != true) return;
        _ = model.OpenProjectAsync(project, id, selectFirst: false);
    }
    private async Task OpenBranchMergeAsync(BrowserSession session)
    {
        if (windowClosed || workspaceSheet != null || settingsPage != null || nativePickerOpen || sessionGroups?.Busy != false || model.ActiveProjectId != session.ProjectId) return;
        var groups = sessionGroups; var revision = model.NavigationRevision;
        var host = await ConnectHostAsync();
        if (host == null || windowClosed || workspaceSheet != null || settingsPage != null || groups != sessionGroups || revision != model.NavigationRevision || model.ActiveProjectId != session.ProjectId) return;
        var state = new WorkspaceBranchMerge(host, session.ProjectId, session.Id);
        MountSheet(new NativeBranchMergePage(state, design, async result =>
        {
            if (windowClosed || state.Closed || groups != sessionGroups || model.ActiveProjectId != session.ProjectId) return;
            CloseSheet();
            await model.OpenProjectAsync(session.ProjectId, result.MainSessionId, selectFirst: false);
            if (!windowClosed && model.ActiveProjectId == session.ProjectId) _ = ShowSessionNoticeAsync(design.T("branch.merge_success"));
        }, CloseSheet, Register));
        await state.LoadAsync();
    }
}
