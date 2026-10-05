using Wisp.ProjectBrowser;

internal static class NativeComposerCommandsTests
{
    public static void Run()
    {
        Check(NativeComposerCommands.Match("/", true, true).Count == 11, "all wired commands discoverable");
        Check(NativeComposerCommands.Match("  /FI", true, true).Single().Command == "/files", "case insensitive prefix");
        Check(NativeComposerCommands.Match("/", false, false).Count == 0, "unavailable routes not advertised");
        Check(NativeComposerCommands.Match("/", false, true).All(item => item.Command != "/upload"), "attachment capability gate");
        foreach (var text in new[] { "ordinary /files", "/files retain this", "/files\nmore", "/unknown" })
        {
            Check(NativeComposerCommands.Match(text, true, true).Count == 0, "prose and arguments not consumed: " + text);
            Check(NativeComposerCommands.Exact(text) == null, "only exact commands execute: " + text);
        }
        Check(NativeComposerCommands.Exact(" /FILES ")?.Command == "/files", "exact route normalizes casing and outer whitespace");
        foreach (var composing in new[] { false, true })
        foreach (var control in new[] { false, true })
        foreach (var shift in new[] { false, true })
            Check(NativeComposerCommands.ShouldSubmit(control, shift, composing) == (control && !shift && !composing), "IME and return policy");
        var enter = NativeInputPreferences.From(new());
        var modifier = NativeInputPreferences.From(new() { ["send_with_modifier"] = true, ["selection_popup_enabled"] = false });
        Check(!enter.SendWithModifier && enter.SelectionPopupEnabled, "old hosts use the shared Enter and selection defaults");
        Check(modifier.SendWithModifier && !modifier.SelectionPopupEnabled, "persisted interaction preferences are consumed");
        Check(NativeInputPreferences.From(new() { ["send_with_modifier"] = "bad", ["selection_popup_enabled"] = 42 }) == enter,
            "malformed preferences do not break input");
        Check(NativeComposerCommands.ShouldSubmit(false, false, false, enter.SendWithModifier), "Enter mode sends without a modifier");
        Check(!NativeComposerCommands.ShouldSubmit(false, false, false, modifier.SendWithModifier), "modifier mode leaves Enter for a newline");
        foreach (var preference in new[] { enter, modifier })
        {
            Check(NativeComposerCommands.ShouldSubmit(true, false, false, preference.SendWithModifier), "Ctrl+Enter sends in either mode");
            foreach (var control in new[] { false, true })
            {
                Check(!NativeComposerCommands.ShouldSubmit(control, true, false, preference.SendWithModifier), "Shift+Enter always inserts a newline");
                Check(!NativeComposerCommands.ShouldSubmit(control, false, true, preference.SendWithModifier), "IME commit never submits in either mode");
            }
        }
        Console.WriteLine("Native composer command discovery, exact dispatch, capability and IME policy passed.");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
