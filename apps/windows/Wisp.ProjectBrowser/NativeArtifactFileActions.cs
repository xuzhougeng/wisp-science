namespace Wisp.ProjectBrowser;

public enum NativeArtifactFileAction { Viewer, Center, Attach, Reveal, FileManager, Provenance, Download }
public sealed record NativeArtifactFileActionItem(NativeArtifactFileAction Action, string Label);
public static class NativeArtifactFileActions
{
    // Keep the WebView ordering; publication evidence bindings are a separate surface.
    public static IReadOnlyList<NativeArtifactFileActionItem> Items { get; } = Array.AsReadOnly(new[] {
        new NativeArtifactFileActionItem(NativeArtifactFileAction.Viewer, "artifact.open_viewer"),
        new(NativeArtifactFileAction.Center, "center.open_file"),
        new(NativeArtifactFileAction.Attach, "ctx.attach_file"),
        new(NativeArtifactFileAction.Reveal, "artifact.reveal_in_files"),
        new(NativeArtifactFileAction.FileManager, "ctx.reveal_in_manager"),
        new(NativeArtifactFileAction.Provenance, "artifact.provenance"),
        new(NativeArtifactFileAction.Download, "artifact.download") });
    public static bool OpensInModal(NativeArtifactEntry entry) => entry.DisplayPath != null && entry.Kind is "image" or "pdf" or "csv";
}
