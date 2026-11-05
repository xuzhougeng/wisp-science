using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeQuickSetupTests
{
    public static async Task RunAsync()
    {
        var locale = new NativeLocalization(new Dictionary<string, Dictionary<string, string>>(), "en");
        var host = new Fake();
        using var wizard = new NativeQuickSetupModel(host, locale);
        Check(await NativeQuickSetupModel.ShouldShowAsync(host));
        await wizard.AdvanceAsync(""); await wizard.AdvanceAsync("");
        await wizard.AdvanceAsync("  ");
        Check(wizard.Step == 3 && host.Commands.SequenceEqual(new[] { "get_onboarding_state" }));
        wizard.Back(); wizard.Back(); Check(wizard.Step == 1);
        await wizard.AdvanceAsync(""); await wizard.AdvanceAsync("fixture-only");
        Check(wizard.Step == 3 && host.Models.SequenceEqual(new[] { "deepseek-v4-pro", "deepseek-v4-flash" })
            && host.Reader["model_id"]?.ToString() == "active-flash" && host.Reader["future"]?.ToString() == "untouched");
        await wizard.AdvanceAsync(""); Check(wizard.Finished);
        await wizard.DismissAsync(); Check(!await NativeQuickSetupModel.ShouldShowAsync(host));
        host.Reader["model_id"] = "chosen-by-user";
        using var reopen = new NativeQuickSetupModel(host, locale);
        await reopen.AdvanceAsync(""); await reopen.AdvanceAsync(""); await reopen.AdvanceAsync("fixture-only");
        Check(host.Reader["model_id"]?.ToString() == "chosen-by-user" && host.Commands.Count(c => c == "save_specialist_cmd") == 1);
        using var failed = new NativeQuickSetupModel(host, locale);
        await failed.AdvanceAsync(""); await failed.AdvanceAsync("");
        host.Unconfirmed = true;
        var submitted = host.Models.Count;
        await failed.AdvanceAsync("fixture-only");
        Check(failed.Step == 2 && failed.Error is { } message && !message.Contains("fixture-only") && host.Models.Count == submitted + 1);
        await failed.AdvanceAsync(""); Check(failed.Step == 3 && host.Models.Count == submitted + 1);
        Check(NativePaletteActions.Match("onboarding", false, false, true, locale).Single().Id == "setup");
    }
    private static void Check(bool value) { if (!value) throw new InvalidOperationException("Quick setup contract failed"); }
    private sealed class Fake : INativeSettingsClient
    {
        public List<string> Commands = [], Models = [];
        public JsonObject Reader = new() { ["id"] = "reader", ["future"] = "untouched", ["model_id"] = "" };
        public bool Unconfirmed;
        private bool show = true;
        public Task<JsonNode?> InvokeAsync(string command, JsonObject arguments, string? projectId = null, CancellationToken cancellationToken = default)
        {
            Check(projectId == null); Commands.Add(command);
            switch (command)
            {
                case "get_onboarding_state": return Task.FromResult<JsonNode?>(new JsonObject { ["show"] = show });
                case "dismiss_onboarding": show = false; break;
                case "save_model":
                    var profile = arguments["profile"]!.AsObject();
                    Check(arguments["key"]?.ToString() == "fixture-only" && !profile.ContainsKey("key"));
                    Models.Add(profile["model"]!.ToString()); if (Unconfirmed) throw new IOException("fixture-only");
                    var active = (JsonObject)profile.DeepClone(); active["id"] = "active-flash"; active["active"] = true;
                    var older = (JsonObject)active.DeepClone(); older["id"] = "unrelated"; older["active"] = false;
                    return Task.FromResult<JsonNode?>(new JsonArray(older, active));
                case "list_specialists": return Task.FromResult<JsonNode?>(new JsonArray(Reader.DeepClone()));
                case "save_specialist_cmd": Reader = (JsonObject)arguments["spec"]!.DeepClone(); break;
                default: throw new InvalidOperationException(command);
            }
            return Task.FromResult<JsonNode?>(null);
        }
    }
}
