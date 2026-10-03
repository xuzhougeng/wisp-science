using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeFileListPresentationTests
{
    public static void Run()
    {
        NativePanelFile[] files = [new("zeta", true, 0, null), new("B.csv", false, 1536, null),
            new("analysis", true, 0, null), new("a.txt", false, 0, null)];
        Check(NativeFileListPresentation.VisibleFiles(files, "").Select(f => f.Name)
            .SequenceEqual(new[] { "analysis", "zeta", "a.txt", "B.csv" }), "folders first, names sorted within each kind");
        Check(NativeFileListPresentation.VisibleFiles(files, " B.CSV ").Single().Name == "B.csv", "filter trims whitespace and ignores case");
        Check(NativeFileListPresentation.VisibleFiles(files, "missing").Length == 0, "no-match state does not retain unrelated entries");
        Check(files[0].Name == "zeta", "presentation never reorders the source model");
        Check(NativeFileListPresentation.Size(0) == "0 B", "zero-byte files stay explicit");
        Check(NativeFileListPresentation.Size(1024) == "1 KiB", "unit boundary");
        Check(NativeFileListPresentation.Size(1536) == "1.5 KiB", "fractional size");
        Check(NativeFileListPresentation.Size(1048576) == "1 MiB", "megabyte unit");
        Check(NativeFileListPresentation.Size(ulong.MaxValue).EndsWith(" EiB"), "large sizes do not overflow");
        Console.WriteLine("Native file list sorting, filtering and readable sizes passed.");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
