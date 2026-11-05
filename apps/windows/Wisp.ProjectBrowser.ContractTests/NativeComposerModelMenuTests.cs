using System.Text.Json.Nodes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

internal static class NativeComposerModelMenuTests
{
    public static void Run()
    {
        static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
        JsonObject Profile(string id, string label, string model, string provider = "provider") => new()
            { ["id"] = id, ["label"] = label, ["model"] = model, ["provider"] = provider };
        JsonObject[] profiles = [Profile("first", "Shared", "chat-a"), Profile("second", " shared ", "chat-b", "gateway"),
            Profile("third", "Unique", "chat-c"), Profile("image", "Shared", "gpt-image-2"), Profile("video", "Movie", "grok-imagine-video")];
        var choices = NativeComposerModelMenu.Choices(profiles, [new("local", "Local investigator")], "second");
        Check(!choices[0].SupportsVision, "missing vision capability must use the image-omission warning");
        profiles[0]["supports_vision"] = true;
        Check(NativeComposerModelMenu.Choices(profiles, [], "first")[0].SupportsVision, "explicit vision capability uses the cache-rebuild warning");
        Check(choices.Select(c => c.Id).SequenceEqual(["first", "second", "third", "acp:local"]), "chat models then exact ACP profile keys, excluding media-only profiles");
        Check(choices[0].Detail == "provider · chat-a" && choices[1].Detail == "gateway · chat-b" && choices[2].Detail == "",
            "duplicate labels are distinguished by provider/model while unique labels stay compact");
        Check(choices[2].Tooltip == "Unique\nprovider · chat-c" && choices.Single(c => c.Selected).Id == "second", "tooltip retains model details and selection uses identity");
        choices = NativeComposerModelMenu.Choices(profiles, [new("local", "Local investigator")], "acp:local");
        Check(choices.Count(c => !c.Acp) == 3 && choices.Single(c => c.Selected) is { Acp: true, Detail: "ACP · local stdio" },
            "an ACP selection retains the HTTP catalog for disabled inspection instead of replacing it");
        Check(NativeComposerModelMenu.Choices([], [], null).Length == 0, "empty catalogs have no fabricated model choices");
        var mode = new NativeAcpSessionState("s", new() { ["currentModeId"] = "analysis", ["availableModes"] = new JsonArray(new JsonObject { ["id"] = "analysis", ["name"] = "Analysis" }) }, []);
        Check(NativeComposerModelMenu.ShowMode(mode) && NativeComposerModelMenu.ModeLabel(mode) == "Analysis", "one mode is displayed as its literal label");
        mode = mode with { Modes = new() { ["currentModeId"] = "custom" } };
        Check(NativeComposerModelMenu.ShowMode(mode) && NativeComposerModelMenu.ModeLabel(mode) == "custom", "a current mode without choices is still shown read-only");
        Check(!NativeComposerModelMenu.ShowMode(mode with { ConfigOptions = [new() { ["id"] = "mode" }] })
            && !NativeComposerModelMenu.ShowMode(mode with { ConfigOptions = [new() { ["name"] = "MODE" }] }), "explicit mode config suppresses duplicate standalone mode row");
        Check(!NativeComposerModelMenu.ShowMode(mode with { Modes = null }), "absent mode does not produce an empty selector");
    }
}
