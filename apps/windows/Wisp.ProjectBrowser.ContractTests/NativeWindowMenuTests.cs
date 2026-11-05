using System.Text.Json.Nodes;
using System.Text.RegularExpressions;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeWindowMenuTests
{
    private static void Check(bool condition, string message) { if (!condition) throw new InvalidOperationException(message); }
    public static async Task RunAsync(string fixtureDirectory)
    {
        var root = Path.GetFullPath(Path.Combine(fixtureDirectory, "../../.."));
        var source = File.ReadAllText(Path.Combine(root, "ui/src/window_titlebar.rs"));
        foreach (var home in new[] { false, true })
        foreach (var group in NativeWindowMenus.Groups)
        {
            var constant = (home && group != "help" ? "HOME_" : "") + group.ToUpperInvariant() + "_ITEMS";
            var body = Regex.Match(source, $@"const {constant}:.*?=\s*&\[(.*?)\];", RegexOptions.Singleline).Groups[1].Value;
            Check(body.Length > 0, "reference menu exists: " + constant);
            var entries = Regex.Matches(body, "\\(\\s*\"([^\"]*)\"\\s*,\\s*\"([^\"]*)\"\\s*,\\s*\"([^\"]*)\"\\s*,?\\s*\\)")
                .Select(match => new NativeWindowMenuItem(match.Groups[1].Value, match.Groups[2].Value, match.Groups[3].Value));
            Check(NativeWindowMenus.Items(group, home).SequenceEqual(entries), "menu actions, order, labels and shortcuts match WebView: " + constant);
        }
        Check(NativeHomePresentation.WindowTitle(null) == "wisp science" && NativeHomePresentation.WindowTitle(" \t") == "wisp science", "home title matches WebView");
        Check(NativeHomePresentation.WindowTitle("  Study 中文  ") == "wisp science — Study 中文", "project title is trimmed and shared with taskbar");
        Check(!NativeWindowMenus.Enabled("import-claude", false) && !NativeWindowMenus.Enabled("export-current-project", false)
            && NativeWindowMenus.Enabled("import-claude", true), "project-specific actions cannot target a missing project");
        var fake = new Fake();
        using var check = new NativeUpdateCheckModel(fake);
        Check(await check.LoadAsync() && fake.Commands.SequenceEqual(new[] { "check_for_updates" }) && check.Result?["latest_version"]?.GetValue<string>() == "2.0.0",
            "update check is read-only and retains its result");
        fake.Reject = true;
        Check(!await check.LoadAsync() && check.Error != null, "failed update request remains visible");
        fake.Reject = false;
        Check(await check.LoadAsync() && check.Error == null, "explicit read retry recovers");
        fake.Pending = new(TaskCreationOptions.RunContinuationsAsynchronously);
        var closing = new NativeUpdateCheckModel(fake);
        var pending = closing.LoadAsync(); closing.Dispose(); fake.Pending.SetResult(new JsonObject { ["update_available"] = true });
        Check(!await pending && closing.Result == null, "closed update dialog ignores late results");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public List<string> Commands = []; public bool Reject; public TaskCompletionSource<JsonNode?>? Pending;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject args, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Commands.Add(command);
            Check(projectId == null && args.Count == 0, "update check has no project or write arguments");
            return Pending?.Task ?? (Reject ? Task.FromException<JsonNode?>(new IOException("offline"))
                : Task.FromResult<JsonNode?>(new JsonObject { ["update_available"] = true, ["latest_version"] = "2.0.0" }));
        }
    }
}
