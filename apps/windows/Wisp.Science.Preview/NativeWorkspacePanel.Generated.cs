#nullable enable
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using Windows.Storage;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel
{
    private readonly NativeGeneratedArtifacts generated = new();
    private readonly ContentControl generatedHost = new() { Visibility = Visibility.Collapsed, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
    private NativeArtifactsPane artifactsPane = null!;
    private readonly Func<string> projectRoot;
    private void InitializeGenerated(Grid root)
    {
        generatedHost.Content = artifactsPane = new(design, model.ReadGeneratedOutputAsync, artifact => OpenArtifactDialogAsync(artifact.Id),
            PerformArtifactFileActionAsync, CanArtifactFileAction, model.ReadRegisteredOutputAsync,
            quoteDocument == null ? null : (path, selection) => !disposed && IsLoaded && quoteDocument(path, selection));
        Grid.SetRow(generatedHost, 1); Grid.SetRowSpan(generatedHost, 2); root.Children.Add(generatedHost);
    }
    private bool UpdateGenerated()
    {
        var changed = generated.Update(Transcript, design.Localization);
        return artifactsPane.Update(generated, model.Artifacts, projectRoot(), model.Loading, model.Error) || changed;
    }
    public async Task ShowGeneratedAsync(NativeGeneratedOutput output)
    {
        if (disposed) return;
        UpdateGenerated(); if (generated.Find(output.Id) is not { Superseded: false }) return;
        reviewNavigation++; runReview?.Close(); model.DismissPreview();
        model.Tabs.Show("artifacts"); tabChanged?.Invoke(model.Tabs.Selected, model.Tabs.Saved);
        Render(); await artifactsPane.OpenAsync(output, lifetime.Token);
    }
    private string? ArtifactWorkspacePath(NativeArtifactEntry entry)
    {
        var path = NativeArtifactCatalog.Identity(entry.DisplayPath ?? "", projectRoot());
        return NativeFilePathRules.Relative(path) ? path : null;
    }
    private bool CanArtifactFileAction(NativeArtifactEntry entry, NativeArtifactFileAction action)
    {
        if (disposed || entry.DisplayPath == null) return false;
        return action switch {
            NativeArtifactFileAction.Center => openCenter != null,
            NativeArtifactFileAction.Download => pickDownload != null,
            NativeArtifactFileAction.Attach => conversation?.Snapshot is { } snapshot && ArtifactWorkspacePath(entry) is { } path
                && conversation.CanReferenceFile(new(snapshot.ProjectId, snapshot.SessionId, path)),
            NativeArtifactFileAction.Reveal or NativeArtifactFileAction.FileManager => ArtifactWorkspacePath(entry) != null,
            NativeArtifactFileAction.Provenance => model.Tabs.Available.Contains("provenance"),
            _ => true
        };
    }
    private async Task PerformArtifactFileActionAsync(NativeArtifactEntry entry, NativeArtifactFileAction action)
    {
        if (!CanArtifactFileAction(entry, action)) return;
        try
        {
            if (action == NativeArtifactFileAction.Provenance) { await ShowTabAsync("provenance"); return; }
            if (action is NativeArtifactFileAction.Attach or NativeArtifactFileAction.Reveal or NativeArtifactFileAction.FileManager)
            {
                if (ArtifactWorkspacePath(entry) is not { } path) return;
                if (action == NativeArtifactFileAction.Attach && conversation?.Snapshot is { } snapshot)
                {
                    if (await conversation.ReferenceFileAsync(new(snapshot.ProjectId, snapshot.SessionId, path), lifetime.Token) && !disposed)
                        insertArtifactDraft?.Invoke(conversation.Draft);
                }
                else if (action == NativeArtifactFileAction.Reveal)
                {
                    syncingFilter = true; filter.Text = ""; syncingFilter = false;
                    await model.RevealArtifactInputAsync(path, lifetime.Token);
                    if (!disposed) { tabChanged?.Invoke(model.Tabs.Selected, model.Tabs.Saved); Render(); }
                }
                else if (action == NativeArtifactFileAction.FileManager)
                {
                    var source = await model.ResolveArtifactFileAsync(path, lifetime.Token);
                    if (disposed) return;
                    var file = await StorageFile.GetFileFromPathAsync(source.Path);
                    var folder = await file.GetParentAsync(); if (disposed || folder == null) return;
                    var options = new Windows.System.FolderLauncherOptions(); options.ItemsToSelect.Add(file);
                    await Windows.System.Launcher.LaunchFolderAsync(folder, options);
                }
                return;
            }
            var reader = entry.Registered != null ? model.CreateArtifactPreview() : model.CreateGeneratedFilePreview(
                NativeArtifactCatalog.Collect(generated.Items, model.Artifacts, projectRoot(), design.Localization)
                    .Where(item => item.Output?.Path != null).Select(item => item.Output!));
            var id = entry.Registered?.Id ?? entry.Id;
            if (action == NativeArtifactFileAction.Viewer) await OpenArtifactDialogAsync(id, reader);
            else if (action == NativeArtifactFileAction.Center) openCenter?.Invoke(reader, id);
            else
            {
                try
                {
                    await reader.ExportItemAsync(id, pickDownload!, NativeArtifactFileCopy.CopyAsync, lifetime.Token);
                    if (!disposed && reader.ExportError is { } error) await ShowErrorAsync(error);
                }
                finally { reader.Close(); }
            }
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (!disposed) await ShowErrorAsync(ex.Message); }
    }
}
