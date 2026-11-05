using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal static class NativeArtifactFileCopy
{
    // Copy the host-validated original, never a bounded/transcoded preview.
    public static Task CopyAsync(NativePanelExport source, string destination, CancellationToken token) => Task.Run(() =>
    {
        token.ThrowIfCancellationRequested();
        File.Copy(source.Path, destination, overwrite: true);
    }, token);
}
