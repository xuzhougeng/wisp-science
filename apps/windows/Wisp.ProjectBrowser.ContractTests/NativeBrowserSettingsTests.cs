using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeBrowserSettingsTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static async Task RunAsync()
    {
        var client = new Fake(); using var model = new NativeSettingsEditorModel(client, "owned");
        Check(!await model.AddBrowserRuleAsync("block", "example.com", ""), "missing filters must not be replaced by an invented empty list");
        await model.LoadAsync("get_browser_auto_launch", "get_browser_auto_close_tabs", "get_browser_url_filters");
        var original = model.Values["get_browser_url_filters"]!.ToJsonString();
        Check(!await model.AddBrowserRuleAsync("other", "example.com", "") && !await model.AddBrowserRuleAsync("block", "  ", ""), "invalid kind and blank domain never submit");
        client.Fail = true;
        Check(!await model.AddBrowserRuleAsync("block", " example.com ", " updated ") && model.Values["get_browser_url_filters"]!.ToJsonString() == original, "failed rule saves preserve confirmed rows");
        client.Fail = false;
        Check(await model.AddBrowserRuleAsync("block", " EXAMPLE.com ", " updated "), "case-insensitive replacement can save");
        var sent = client.Writes[^1];
        Check(sent.Project == "owned" && sent.Command == "set_browser_url_filters" && sent.Args["filters"]!["block"]!.AsArray().Count == 1 && sent.Args["filters"]!["block"]![0]!["reason"]!.GetValue<string>() == "updated", "rule submission uses explicit context, trims input and replaces duplicate hosts");
        Check(sent.Args["filters"]!["prefer"]![0]!["host"]!.GetValue<string>() == "papers.example" && sent.Args["filters"]!["future"]!.GetValue<bool>(), "editing one list preserves sibling rules and unknown fields");
        Check(model.Values["get_browser_url_filters"]!["block"]![0]!["host"]!.GetValue<string>() == "example.com", "display follows normalized host acknowledgement");
        var calls = client.Writes.Count;
        Check(!await model.RemoveBrowserRuleAsync("block", "missing") && client.Writes.Count == calls, "stale removal never submits a changed snapshot");
        Check(await model.RemoveBrowserRuleAsync("block", "example.com") && model.Values["get_browser_url_filters"]!["block"]!.AsArray().Count == 0 && model.Values["get_browser_url_filters"]!["prefer"]!.AsArray().Count == 1, "acknowledged removal affects the requested list only");
        client.Fail = true;
        Check(!await model.SetBrowserBehaviorAsync("get_browser_auto_launch", false) && model.Values["get_browser_auto_launch"]!.GetValue<bool>(), "failed toggle retains acknowledged state"); client.Fail = false;
        Check(await model.SetBrowserBehaviorAsync("get_browser_auto_launch", false) && !model.Values["get_browser_auto_launch"]!.GetValue<bool>() && client.Writes[^1].Args["enabled"]!.GetValue<bool>() == false, "toggle uses scalar enabled argument");
        Check(!await model.SetBrowserBehaviorAsync("get_settings", true), "behavior helper cannot write arbitrary settings");
        client.Pending = new(); var pending = model.AddBrowserRuleAsync("prefer", "new.example", ""); calls = client.Writes.Count;
        Check(model.Busy && !await model.RemoveBrowserRuleAsync("prefer", "papers.example") && client.Writes.Count == calls, "pending save blocks concurrent stale-list writes");
        client.Pending.SetResult(JsonValue.Create("invalid"));
        Check(!await pending && model.Values["get_browser_url_filters"]!["prefer"]!.AsArray().Count == 1, "malformed acknowledgement never replaces confirmed filters");
        client.Pending = new(); pending = model.AddBrowserRuleAsync("prefer", "late.example", ""); model.Dispose(); client.Pending.SetResult(Fake.Filters()); await pending;
        Check(model.Values.Count == 0, "late acknowledgements cannot repopulate a closed settings page");
    }
    private sealed class Fake : INativeSettingsClient
    {
        public bool Fail;
        public TaskCompletionSource<JsonNode?>? Pending;
        public List<(string Command, JsonObject Args, string? Project)> Writes { get; } = [];
        public static JsonObject Filters() => new() { ["block"] = new JsonArray(new JsonObject { ["host"] = "example.com", ["reason"] = "old" }), ["prefer"] = new JsonArray(new JsonObject { ["host"] = "papers.example", ["reason"] = "papers" }), ["future"] = true };
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            if (command == "get_browser_url_filters") return Task.FromResult<JsonNode?>(Filters());
            if (command == "get_browser_auto_launch") return Task.FromResult<JsonNode?>(JsonValue.Create(true));
            if (command == "get_browser_auto_close_tabs") return Task.FromResult<JsonNode?>(JsonValue.Create(false));
            Writes.Add((command, (JsonObject)arguments.DeepClone(), projectId));
            if (Fail) return Task.FromException<JsonNode?>(new IOException("lost acknowledgement"));
            if (Pending != null) return Pending.Task;
            if (command == "set_browser_url_filters")
            {
                var reply = (JsonObject)arguments["filters"]!.DeepClone();
                foreach (var kind in new[] { "block", "prefer" }) foreach (var row in reply[kind]!.AsArray()) row!["host"] = row["host"]!.GetValue<string>().ToLowerInvariant();
                return Task.FromResult<JsonNode?>(reply);
            }
            if (command is "set_browser_auto_launch" or "set_browser_auto_close_tabs") return Task.FromResult(arguments["enabled"]?.DeepClone());
            throw new InvalidOperationException(command);
        }
    }
}
