using Wisp.ProjectBrowser;

internal static class NativePanelViewStateTests
{
    public static void Run()
    {
        var panel = new NativePanelViewState();
        var files = panel.For("files", "results");
        files.SetFilter("图");
        files.SetOffset(480);
        var artifacts = panel.For("artifacts", "results");
        Check(artifacts.Filter == "" && artifacts.Offset == 0, "file filter does not hide artifact results");
        artifacts.SetFilter("pdf"); artifacts.SetOffset(200);
        Check(panel.For("files", "results").Filter == "图" && panel.For("files", "results").Offset == 480,
            "returning to files restores its filter and reading position");
        Check(panel.For("files", "data").Filter == "" && panel.For("files", "data").Offset == 0,
            "another directory starts with independent reading state");
        Check(ReferenceEquals(artifacts, panel.For("artifacts", "data")),
            "file navigation does not reset another tab's reading state");
        files.SetFilter("图");
        Check(files.Offset == 480, "synchronizing unchanged filter keeps the reading position");
        files.SetFilter("table");
        Check(files.Offset == 0, "a new filter starts at its first result");
        Check(new NativePanelViewState().For("files", "results").Filter == "",
            "new session panel does not inherit previous session filters");
        foreach (var invalid in new[] { double.NaN, double.PositiveInfinity, -1d })
        {
            files.SetOffset(invalid);
            Check(files.Offset == 0, "invalid layout offsets cannot poison subsequent restoration");
        }
        Console.WriteLine("Native panel per-tab and per-directory reading state passed.");
    }
    private static void Check(bool value, string message)
    {
        if (!value) throw new InvalidOperationException(message);
    }
}
