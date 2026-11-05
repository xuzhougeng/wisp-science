using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private WorkspaceArtifactTabs? artifactTabs;
    private NativeCenterFiles? centerFiles;
    private async void OpenCenterArtifact(WorkspaceArtifactModel reader, string id)
    {
        artifactTabs ??= new();
        centerFiles ??= new(artifactTabs, design, () => HandleWindowEscape(), PickFileDownload, QuoteArtifactDocument);
        var opening = artifactTabs.OpenAsync(reader, id);
        Render(); await opening;
    }
    private bool QuoteArtifactDocument(NativeArtifactDocumentQuote request)
    {
        var project = request.ProjectId; var session = request.SessionId; var owner = conversation; var source = artifactTabs?.Selected;
        if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session
            || owner?.AddDocumentQuote(project, session, request.Path, request.Selection) != true) return false;
        if (request.Selection.Jump) DispatcherQueue.TryEnqueue(() =>
        {
            if (windowClosed || conversation != owner || model.ActiveProjectId != project || model.ActiveSessionId != session
                || !ReferenceEquals(artifactTabs?.Selected, source)) return;
            artifactTabs?.Select(null); Render(); conversationPage?.FocusComposer();
        });
        return true;
    }
    private bool InsertArtifactDraft(string project, string? session, string text)
    {
        if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session || conversation?.Prefill(text) != true) return false;
        artifactTabs?.Select(null);
        DispatcherQueue.TryEnqueue(() =>
        {
            if (windowClosed || model.ActiveProjectId != project || model.ActiveSessionId != session) return;
            Render(); conversationPage?.FocusComposer();
        });
        return true;
    }
    private void ClearCenterArtifacts()
    {
        centerFiles?.Dispose(); centerFiles = null; artifactTabs?.Dispose(); artifactTabs = null;
    }
}
