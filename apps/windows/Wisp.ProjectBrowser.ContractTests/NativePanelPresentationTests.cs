using System.Text.Json;
using Wisp.ProjectBrowser;

internal static class NativePanelPresentationTests
{
    public static void Run(string fixture)
    {
        var root = Path.GetFullPath(Path.Combine(Path.GetDirectoryName(fixture)!, "../../.."));
        var resources = Path.Combine(root, "apps/macos/Sources/WispProjectBrowserUI/Resources");
        var catalog = JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(
            File.ReadAllText(Path.Combine(resources, "native-locales.json")))!;
        var locale = new NativeLocalization(catalog);
        foreach (var code in new[] { "en", "zh" })
        {
            locale.Preview(code);
            foreach (var (panel, key) in new[] { ("files", "file"), ("artifacts", "artifacts"), ("notebook", "notebook"), ("highlights", "highlights"), ("provenance", "tools"), ("runs", "runs") })
            {
                var copy = NativePanelEmptyCopy.For(panel, locale);
                Check(copy.Title == catalog[code]["right.no_" + key + ".title"] && copy.Body == catalog[code]["right.no_" + key + ".body"],
                    $"{panel} uses the {code} WebView empty-state copy");
                if (code == "en") Check(!copy.Title.Concat(copy.Body).Any(c => c >= '\u4e00' && c <= '\u9fff'), "English empty panels contain no hard-coded Chinese");
            }
            var filtered = NativePanelEmptyCopy.For("notebook", locale, true);
            Check(filtered.Title != NativePanelEmptyCopy.For("notebook", locale).Title && filtered.Body.Length > 0,
                "a filter with no matches explains how to recover instead of claiming there is no notebook content");
            foreach (var key in new[] { "right.launcher.start", "right.launcher.title", "right.launcher.home", "right.launcher.more", "agents.activity.empty", "agents.manage_workflows", "sidechat.title", "sidechat.empty", "sidechat.placeholder", "specialists.subagent_model", "specialists.subagent_model.hint" })
                Check(locale.Text(key) != key && !string.IsNullOrWhiteSpace(locale.Text(key)), $"{key} has a {code} translation");
        }
        foreach (var icon in new[] { "compass", "swarm", "folder", "layers", "server", "activity-orbit", "notebook", "star", "branch", "chat", "memory", "arrow-up" })
            Check(File.Exists(Path.Combine(resources, "icon-" + icon + ".svg")), $"the launcher and side chat ship the shared {icon} icon");
        Console.WriteLine("Native panel localized empty states and launcher assets passed.");
    }
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
}
