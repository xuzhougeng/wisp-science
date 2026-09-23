using Wisp.ProjectBrowser.Contracts;

static class NativePanelTabsTests
{
    private static void Require(bool value) { if (!value) throw new InvalidOperationException("Native panel tab behavior drift"); }
    public static void Run()
    {
        var restored = new NativePanelTabs("[\"hosts\",\"files\",\"hosts\",\"future\",\"notebook\"]", "future");
        Require(restored.Open.SequenceEqual(new[] { "hosts", "files" }) && restored.Selected == "hosts");
        Require(new NativePanelTabs("broken").Open.SequenceEqual(NativePanelTabs.Defaults));
        Require(new NativePanelTabs(restored.Saved, restored.Selected).Open.SequenceEqual(restored.Open));
        foreach (var saved in new[] { "[]", "[\"agents\"]" })
        {
            var reveal = new NativePanelTabs(saved, "files", NativePanelTabs.All);
            reveal.Show("files");
            var reopened = new NativePanelTabs(reveal.Saved, reveal.Selected, NativePanelTabs.All);
            Require(reopened.Selected == "files" && reopened.Open.Count(id => id == "files") == 1);
        }
        var state = new NativePanelTabs(selected: "files");
        state.Remove("files"); Require(state.Selected == "agents");
        state.Remove("artifacts"); Require(state.Selected == "agents");
        state.Remove("agents"); Require(state.Selected == "hosts");
        state.Remove("hosts"); Require(state.Open.Count == 0);
        Require(new NativePanelTabs(state.Saved).Open.Count == 0);
        state.Reopen(); Require(state.Open.SequenceEqual(NativePanelTabs.Defaults) && state.Selected == "artifacts");
        state.Show("agents"); state.Move("artifacts", "files");
        Require(state.Open.SequenceEqual(new[] { "agents", "files", "artifacts", "hosts" }) && state.Selected == "agents");
        state.Move("hosts", "agents"); state.Remove("files"); state.Show("files"); state.Show("files");
        Require(state.Open.SequenceEqual(new[] { "hosts", "agents", "artifacts", "files" }));
        state.Show("notebook"); state.Move("unknown", "files"); Require(state.Selected == "files" && state.Open.Count == 4);
        var full = new NativePanelTabs(available: NativePanelTabs.All);
        foreach (var id in new[] { "notebook", "highlights", "provenance", "sidechat" }) full.Show(id);
        Require(full.Open.Count == 8 && new NativePanelTabs(full.Saved, full.Selected, NativePanelTabs.All).Open.SequenceEqual(full.Open));
        Console.WriteLine("Native panel tab restoration, close, reorder and optional registry tests passed.");
    }
}
