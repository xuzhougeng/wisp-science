using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed record NativeArtifactDocumentQuote(string ProjectId, string SessionId, string Path, NativeDocumentSelection Selection);

/// <summary>A modal read scope separate from the panel's selection and refresh cycle.</summary>
public sealed class WorkspaceArtifactModel(INativePanelClient client, string project, string session, IEnumerable<NativePanelArtifact> artifacts, bool allowLibrary = true)
{
    private readonly NativePanelArtifact[] artifacts = artifacts.DistinctBy(a => a.Id).ToArray();
    private readonly Dictionary<string, string> filePaths = [];
    private sealed record OutputScope(string Context, string RunId, string? Requester);
    private OutputScope? methodOutput;
    private int generation;
    public string Scope => project + ":" + session;
    public WorkspaceArtifactModel Fork()
    {
        var result = new WorkspaceArtifactModel(client, project, session, artifacts, allowLibrary);
        result.methodOutput = methodOutput;
        foreach (var pair in filePaths) result.filePaths.Add(pair.Key, pair.Value);
        return result;
    }
    public static WorkspaceArtifactModel ForGenerated(INativePanelClient client, string project, string session, IEnumerable<NativeGeneratedOutput> outputs)
    {
        var files = outputs.Where(item => item.Path != null && !item.Superseded).ToArray();
        if (files.Any(item => item.ProjectId != project || item.SessionId != session))
            throw new InvalidOperationException("Generated file scope mismatch");
        var reader = new WorkspaceArtifactModel(client, project, session, files.Select(item =>
            new NativePanelArtifact(item.Id, item.Name, item.Kind, item.Path!, null, 0, item.Path)));
        foreach (var file in files.DistinctBy(item => item.Id)) reader.filePaths.Add(file.Id, file.Path!);
        return reader;
    }
    public bool CanUseLibrary => allowLibrary && Selected is { } item && !filePaths.ContainsKey(item.Id) && IsImage(item);
    // A promoted version does not necessarily match the current workspace file's provenance.
    public bool CanReadProvenance => Selected != null && methodOutput == null;
    public static WorkspaceArtifactModel ForMethodOutputs(INativePanelClient client, string project, string? requester, string context, NativeMethodDetail detail)
    {
        var items = detail.Outputs.Select(o => new NativePanelArtifact(o.ArtifactVersionId, Path.GetFileName(o.SourcePath), "",
            "artifact-version:" + o.ArtifactVersionId, null, 0, null));
        return new WorkspaceArtifactModel(client, project, requester ?? "", items, false) { methodOutput = new(context, detail.Run.Id, requester) };
    }
    public NativePanelArtifact? Find(string id) => artifacts.FirstOrDefault(a => a.Id == id);
    public bool ExportBusy { get; private set; }
    public string? ExportError { get; private set; }
    public string? ExportedPath { get; private set; }
    public string? LibraryItemId { get; private set; }
    public bool LibraryBusy { get; private set; }
    public bool LibraryKnown { get; private set; }
    public string? LibraryError { get; private set; }
    public NativePanelArtifact? Selected { get; private set; }
    public NativePanelFileContent? Content { get; private set; }
    public NativeArtifactProvenance? Provenance { get; private set; }
    public bool LoadingContent { get; private set; }
    public bool LoadingProvenance { get; private set; }
    public string? ContentError { get; private set; }
    public string? ProvenanceError { get; private set; }
    public event Action? Changed;
    public NativeArtifactDocumentQuote? DocumentQuote(NativePanelArtifact item, NativePanelFileContent content, NativeDocumentSelection selection)
    {
        // The mounted reader owns identity. Neither the renderer's payload nor
        // a mutable workspace alias may replace a registered artifact snapshot.
        if (LoadingContent || !ReferenceEquals(Selected, item) || !ReferenceEquals(Content, content)
            || !NativeArtifactSelection.Allows(content, item.Name, item.Kind, selection)) return null;
        if (methodOutput != null && session.Length == 0) return null;
        var path = methodOutput != null ? item.Path : filePaths.TryGetValue(item.Id, out var file) ? file : "artifact:" + item.Id;
        if (string.IsNullOrWhiteSpace(path) || path.Length > 32768 || path.Any(char.IsControl)) return null;
        return new(project, session, path, selection);
    }
    public NativePanelArtifact? Adjacent(int direction)
    {
        if (Selected is not { } selected || !IsImage(selected)) return null;
        var images = artifacts.Where(IsImage).ToArray(); var index = Array.FindIndex(images, a => a.Id == selected.Id);
        var next = index + Math.Sign(direction);
        return index >= 0 && direction != 0 && next >= 0 && next < images.Length ? images[next] : null;
    }
    public static bool IsImage(NativePanelArtifact a) => a.Kind.StartsWith("image/", StringComparison.OrdinalIgnoreCase)
        || a.Kind == "image" || NativeAttachmentPresentation.Image(a.Name);
    public NativePanelArtifact? InputArtifact(string path) => artifacts.FirstOrDefault(a => a.Path == path || a.LogicalPath == path);
    public async Task OpenAsync(string id, CancellationToken token = default)
    {
        var selected = artifacts.FirstOrDefault(a => a.Id == id) ?? throw new InvalidOperationException("Artifact is no longer listed in this conversation");
        var current = ++generation; Selected = selected; Content = null; Provenance = null;
        ExportError = ExportedPath = null;
        LibraryItemId = LibraryError = null; LibraryKnown = false; LibraryBusy = CanUseLibrary;
        ContentError = ProvenanceError = null; LoadingContent = true; LoadingProvenance = CanReadProvenance; Changed?.Invoke();
        await Task.WhenAll(ReadContent(), ReadProvenance(), ReadLibrary());
        async Task ReadLibrary()
        {
            if (!CanUseLibrary) return;
            await LibraryAsync(null, current, selected, token);
        }
        async Task ReadContent()
        {
            try { var value = methodOutput is { } source
                    ? (await client.MethodOutputAsync<NativePanelFileContent>(project, source.Requester, source.Context, source.RunId, id, token)) with { Path = selected.Name }
                    : filePaths.TryGetValue(id, out var path) ? await client.ReadFileAsync(project, session, path, token)
                    : await client.ReadArtifactAsync(project, session, id, token); if (current == generation && !token.IsCancellationRequested) Content = value; }
            catch (OperationCanceledException) { }
            catch (Exception ex) { if (current == generation) ContentError = ex.Message; }
            finally { if (current == generation) { LoadingContent = false; Changed?.Invoke(); } }
        }
        async Task ReadProvenance()
        {
            if (!CanReadProvenance) return;
            try { var value = filePaths.TryGetValue(id, out var path) ? await client.FileProvenanceAsync(project, session, path, token)
                    : await client.ArtifactProvenanceAsync(project, session, id, token);
                if (current == generation && !token.IsCancellationRequested) Provenance = value; }
            catch (OperationCanceledException) { }
            catch (Exception ex) { if (current == generation && !token.IsCancellationRequested) ProvenanceError = ex.Message; }
            finally { if (current == generation) { LoadingProvenance = false; Changed?.Invoke(); } }
        }
    }
    public Task NavigateAsync(int direction, CancellationToken token = default) => !ExportBusy && Adjacent(direction) is { } next ? OpenAsync(next.Id, token) : Task.CompletedTask;
    // Tile downloads select a scoped source without fetching a bounded preview first.
    public Task ExportItemAsync(string id, Func<string, Task<string?>> choose, Func<NativePanelExport, string, CancellationToken, Task> copy, CancellationToken token = default)
    {
        if (ExportBusy || token.IsCancellationRequested) return Task.CompletedTask;
        Close(); Selected = Find(id) ?? throw new InvalidOperationException("Artifact is no longer listed");
        return ExportAsync(choose, copy, token);
    }
    public async Task ToggleLibraryAsync(CancellationToken token = default)
    {
        if (LibraryBusy || Selected is not { } item || !CanUseLibrary) return;
        // An uncertain write must be read back before another mutation is allowed.
        await LibraryAsync(LibraryKnown ? LibraryItemId == null : null, generation, item, token);
    }
    private async Task LibraryAsync(bool? enabled, int current, NativePanelArtifact item, CancellationToken token)
    {
        LibraryBusy = true; LibraryError = null; Changed?.Invoke();
        try
        {
            var id = await client.ArtifactLibraryAsync(project, session, item.Id, enabled, token);
            if (current == generation) { LibraryItemId = id; LibraryKnown = true; }
        }
        catch (OperationCanceledException) { if (current == generation) LibraryKnown = false; }
        catch (Exception ex) { if (current == generation) { LibraryError = ex.Message; LibraryKnown = false; } }
        finally { if (current == generation) { LibraryBusy = false; Changed?.Invoke(); } }
    }
    public async Task ExportAsync(Func<string, Task<string?>> choose, Func<NativePanelExport, string, CancellationToken, Task> copy, CancellationToken token = default)
    {
        if (ExportBusy || Selected is not { } item) return;
        var current = generation; ExportBusy = true; ExportError = ExportedPath = null; Changed?.Invoke();
        try
        {
            var source = methodOutput is { } output ? await client.MethodOutputAsync<NativePanelExport>(project, output.Requester, output.Context, output.RunId, item.Id, token)
                : filePaths.TryGetValue(item.Id, out var path) ? await client.ExportFileAsync(project, session, path, token)
                : await client.ExportArtifactAsync(project, session, item.Id, token);
            if (current != generation || token.IsCancellationRequested) return;
            var destination = await choose(source.Name);
            if (current != generation || token.IsCancellationRequested || string.IsNullOrEmpty(destination)) return;
            if (System.IO.Path.GetFullPath(source.Path).Equals(System.IO.Path.GetFullPath(destination), StringComparison.OrdinalIgnoreCase))
                throw new InvalidOperationException("Choose a destination different from the original file.");
            await copy(source, destination, token);
            if (current == generation) ExportedPath = destination;
        }
        catch (OperationCanceledException) { }
        catch (Exception ex) { if (current == generation) ExportError = ex.Message; }
        finally { ExportBusy = false; if (current == generation) Changed?.Invoke(); }
    }
    public void Close() { generation++; Selected = null; Content = null; Provenance = null; LibraryItemId = LibraryError = null; LibraryKnown = LibraryBusy = false; LoadingContent = LoadingProvenance = false; Changed?.Invoke(); }
}
