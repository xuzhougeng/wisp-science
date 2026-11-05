using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeComputeMenuTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static void Run()
    {
        NativePanelContext Context(string id, string kind, string name, string config = "{}") => new(id, kind, name, config, "{}", null, null);
        var contexts = new NativePanelContexts([
            Context("local", "local", "Local"), Context("ssh:gpu", "ssh", "GPU display name"),
            Context("wsl:Ubuntu", "wsl", "Ubuntu", "{\"is_default\":true}"), Context("wsl:blank", "wsl", " ", "bad json"),
            Context("future", "cluster", "Cluster"), Context("ssh:orphan", "ssh", "Orphan")], ["wsl:Ubuntu"], false, new(null));
        var rows = NativeComputeMenu.Resources(contexts, ["gpu", "登録済み"], "ssh:gpu", "");
        Check(NativeComputeMenu.AttachCandidates(contexts, ["gpu"], "").Select(r => r.Id).SequenceEqual(["ssh:gpu", "wsl:blank"]),
            "the attach list excludes local, attached resources, orphan aliases and unsupported kinds");
        var named = contexts with { Contexts = [Context("wsl:distro-id", "wsl", "Research box")] };
        Check(NativeComputeMenu.AttachCandidates(named, [], " DISTRO-ID ").Single().Name == "Research box", "WSL search also matches its underlying ID");
        var probed = Context("local", "local", "Local") with { CapabilitiesJson = """{"os":"Linux","arch":"x86_64","scheduler":"Slurm","probe_skill":"v1","privilege":"user"}""" };
        Check(NativeComputeMenu.CapabilitySummary(probed) == "Linux/x86_64 · Slurm · No GPU · user"
            && NativeComputeMenu.CapabilitySummary(probed with { CapabilitiesJson = "malformed", LastProbeStatus = "error" }) == "error",
            "machine card and detail use the same capability ordering and probe fallback as WebView");
        Check(rows.Select(r => r.Id).SequenceEqual(["ssh:gpu", "ssh:登録済み", "wsl:Ubuntu", "wsl:blank"]), "registered SSH aliases precede WSL, without orphan contexts or unsupported kinds");
        Check(rows[0].Name == "gpu" && rows[0].Default && rows[0].StateKey == "compute.auto_attaches", "SSH alias is literal and inherited default auto-attaches");
        Check(rows[1].StateKey == "compute.not_attached" && rows[2].Attached && rows[2].WslDefault && rows[2].StateKey == "compute.attached", "membership and default badges are independent");
        Check(rows[3].Name == "wsl:blank" && !rows[3].WslDefault, "blank WSL label falls back to ID and malformed config is harmless");
        Check(NativeComputeMenu.Resources(contexts, ["gpu"], null, "  UBUNtu ").Single().Id == "wsl:Ubuntu", "search trims input and ignores case");
        Check(NativeComputeMenu.Resources(contexts, ["gpu"], null, "absent").Length == 0, "unmatched search has an empty resource list");
        Check(NativeComputeMenu.DefaultId("local", "ssh:gpu") == "local" && NativeComputeMenu.DefaultId(" ", " ssh:gpu ") == "ssh:gpu", "explicit local overrides global while blank storage inherits");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> { ["en"] = new() { ["compute.default_local"] = "Local by default" } });
        var choices = NativeComputeMenu.DefaultChoices(contexts, locale);
        Check(choices[0] == new ComposerOptionChoice("local", "Local by default") && choices[1].Label == "GPU display name" && choices.All(c => c.Id != "future"), "default selector uses context labels and only supported remote kinds");
        Console.WriteLine("Compute menu defaults, membership states, SSH/WSL resources and search passed.");
    }
}
