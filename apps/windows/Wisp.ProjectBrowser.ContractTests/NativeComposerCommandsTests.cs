using Wisp.ProjectBrowser;

internal static class NativeComposerCommandsTests
{
    public static void Run()
    {
        Check(NativeComposerCommands.Match("/", true, true).Count == 12, "all wired commands discoverable");
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
        Console.WriteLine("Native composer command discovery, exact dispatch, capability and IME policy passed.");
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
