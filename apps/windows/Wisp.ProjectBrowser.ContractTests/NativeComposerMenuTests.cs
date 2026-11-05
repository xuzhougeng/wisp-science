using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeComposerMenuTests
{
    public static void Run()
    {
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        Check(NativeComposerMenu.Width(1100) == 320 && NativeComposerMenu.Width(300) == 268,
            "agent menu includes its padding within the WebView cap and keeps window-edge clearance");
        Check(NativeComposerMenu.Groups.Select(g => g.LabelKey).SequenceEqual(["composer.group.mode", "composer.group.agent", "composer.group.resources"]), "WebView group order");
        Check(NativeComposerMenu.Groups.SelectMany(g => g.Items).Select(i => i.Id).SequenceEqual(["plan", "full_permission", "specialist", "auto_review", "reviewer", "compute"]),
            "only conversation controls belong in this menu; global settings and obsolete delegation switch are absent");
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>> {
            ["en"] = new() { ["composer.reviewer.default_http"] = "Default HTTP model", ["composer.reviewer.follow_session"] = "Follow session backend", ["composer.specialist.none"] = "None", ["compute.default_local"] = "Local by default", ["compute.default_named"] = "Default {name}", ["composer.compute_count"] = "{n} remote" },
            ["zh"] = new() { ["composer.reviewer.default_http"] = "默认 HTTP 模型", ["composer.reviewer.follow_session"] = "跟随会话后端", ["composer.specialist.none"] = "无", ["compute.default_local"] = "默认本机", ["compute.default_named"] = "默认 {name}", ["composer.compute_count"] = "{n} 个远程环境" }
        });
        Check(NativeComposerMenu.ReviewerLabel(new("follow_session", "旧文案"), locale) == "Follow session backend", "reserved reviewer IDs use shared locale text");
        Check(NativeComposerMenu.ReviewerLabel(new("http:profile", "默认 HTTP 模型"), locale) == "默认 HTTP 模型", "user model names are never translated");
        Check(NativeComposerMenu.SpecialistLabel(new("", "旧文案"), locale) == "None", "empty specialist identity localizes None");
        Check(NativeComposerMenu.ComputeSummary(null, null, 0, locale) == "Local by default", "no remote membership uses local default copy");
        Check(NativeComposerMenu.ComputeSummary("local", null, 2, locale) == "2 remote", "remote membership count appears when default is local");
        Check(NativeComposerMenu.ComputeSummary("ssh:gpu", " GPU lab ", 0, locale) == "Default GPU lab", "remote default wins over zero explicit memberships");
        locale.Preview("zh");
        Check(NativeComposerMenu.ComputeSummary("ssh:missing", null, 3, locale) == "默认 ssh:missing", "missing context label preserves exact identity in localized summary");
        Check(NativeComposerMenu.ReviewerLabel(new("http:", "old"), locale) == "默认 HTTP 模型", "locale change refreshes the default choice");
        NativeAcpSessionState State(params string[] ids) => new("s", new JsonObject { ["availableModes"] = new JsonArray(ids.Select(id => (JsonNode)new JsonObject { ["id"] = id, ["name"] = "display " + id }).ToArray()) }, []);
        Check(NativeComposerMenu.PlanPair(State("agent", "plan_mode", "default")) == ("plan_mode", "default"), "ACP plan pairing prefers exact default return mode");
        Check(NativeComposerMenu.PlanPair(State("read", "PLAN")) == ("PLAN", "read"), "ACP pairing keeps protocol IDs and supports named plan modes");
        Check(NativeComposerMenu.PlanPair(State("plan")) == null && NativeComposerMenu.PlanPair(State("read", "agent")) == null, "hide plan toggle without both plan and exit modes");
        Console.WriteLine("Composer menu grouping, localization, summaries and ACP mode adapter passed.");
    }
}
