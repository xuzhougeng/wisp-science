using Wisp.ProjectBrowser.Contracts;

static class NativePanelTabsTests
{
    private static void Require(bool value) { if (!value) throw new InvalidOperationException("Native panel tab behavior drift"); }
    public static void Run()
    {
        var restored = new NativePanelTabs("[\"hosts\",\"files\",\"hosts\",\"future\",\"notebook\"]", "future");
        Require(restored.Open.SequenceEqual(new[] { "hosts", "files" }) && restored.Selected == "hosts");
        Require(new NativePanelTabs("broken").Open.Count == 0);
        Require(new NativePanelTabs(restored.Saved, restored.Selected).Open.SequenceEqual(restored.Open));
        foreach (var saved in new[] { "[]", "[\"agents\"]" })
        {
            var reveal = new NativePanelTabs(saved, "files", NativePanelTabs.All);
            reveal.Show("files");
            var reopened = new NativePanelTabs(reveal.Saved, reveal.Selected, NativePanelTabs.All);
            Require(reopened.Selected == "files" && reopened.Open.Count(id => id == "files") == 1);
        }
        var initial = "[\"artifacts\",\"agents\",\"files\",\"hosts\"]";
        var state = new NativePanelTabs(initial, "files");
        state.Remove("files"); Require(state.Selected == "agents");
        state.Remove("artifacts"); Require(state.Selected == "agents");
        state.Remove("agents"); Require(state.Selected == "hosts");
        state.Remove("hosts"); Require(state.Open.Count == 0);
        Require(new NativePanelTabs(state.Saved).Open.Count == 0);
        state.Reopen(); Require(state.Open.Count == 0 && state.Selected == NativePanelTabs.Home);
        state = new NativePanelTabs(initial);
        state.Show("agents"); state.Move("artifacts", "files");
        Require(state.Open.SequenceEqual(new[] { "agents", "files", "artifacts", "hosts" }) && state.Selected == "agents");
        state.Move("hosts", "agents"); state.Remove("files"); state.Show("files"); state.Show("files");
        Require(state.Open.SequenceEqual(new[] { "hosts", "agents", "artifacts", "files" }));
        state.Show("notebook"); state.Move("unknown", "files"); Require(state.Selected == "files" && state.Open.Count == 4);
        var full = new NativePanelTabs(initial, available: NativePanelTabs.All);
        foreach (var id in new[] { "notebook", "highlights", "provenance", "sidechat" }) full.Show(id);
        Require(full.Open.Count == 8 && new NativePanelTabs(full.Saved, full.Selected, NativePanelTabs.All).Open.SequenceEqual(full.Open));
        full.Show("memos"); full.Show("memos");
        Require(full.Open.Count == 9 && full.Selected == "memos");
        full.Show("runs"); full.Show("runs");
        Require(full.Open.Count == 10 && full.Selected == "runs");
        var restoredRuns = new NativePanelTabs(full.Saved, full.Selected, NativePanelTabs.All);
        Require(restoredRuns.Selected == "runs" && restoredRuns.Open.Count(id => id == "runs") == 1);
        restoredRuns.Remove("runs"); restoredRuns.Show("runs"); Require(restoredRuns.Selected == "runs");
        full.Show("memos");
        full.Remove("agents"); Require(full.Selected == "memos");
        full.Move("memos", "artifacts"); Require(full.Open[0] == "memos" && full.Selected == "memos");
        full.Remove("memos"); Require(full.Selected == "artifacts");
        var projectOnly = new NativePanelTabs("[]", "memos", ["files", "memos"]);
        projectOnly.Reopen(); Require(projectOnly.Open.Count == 0 && projectOnly.Selected == NativePanelTabs.Home);
        projectOnly.Show("files");
        projectOnly.Show("memos"); projectOnly.Remove("memos"); Require(projectOnly.Selected == "files");
        projectOnly.Remove("files"); projectOnly.Reopen(); Require(projectOnly.Open.Count == 0 && projectOnly.Selected == NativePanelTabs.Home);
        var home = new NativePanelTabs(available: NativePanelTabs.All);
        Require(home.Selected == NativePanelTabs.Home && home.Open.Count == 0);
        home.Show("notebook"); var open = home.Saved; home.Show(NativePanelTabs.Home);
        Require(home.Selected == NativePanelTabs.Home && home.Saved == open);
        Require(new NativePanelTabs(home.Saved, home.Selected, NativePanelTabs.All).Selected == NativePanelTabs.Home);
        home.Remove("files"); home.Move("notebook", "artifacts");
        Require(home.Selected == NativePanelTabs.Home && !home.Open.Contains(NativePanelTabs.Home));
        Console.WriteLine("Native panel tab restoration, close, reorder and optional registry tests passed.");
    }
}
