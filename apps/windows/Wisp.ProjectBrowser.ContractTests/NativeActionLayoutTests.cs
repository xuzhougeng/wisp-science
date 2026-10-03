using Wisp.ProjectBrowser;

internal static class NativeActionLayoutTests
{
    public static void Run()
    {
        NativeActionLayout.Size[] sizes = [new(80, 32), new(100, 40), new(70, 32)];
        var narrow = NativeActionLayout.Arrange(sizes, 200);
        Check(narrow[0].X == 0 && narrow[1].X == 88 && narrow[2].Y == 48, "wrap preserves order and tallest row");
        Check(narrow.All(s => s.X + s.Width <= 200), "all actions fit a narrow column");
        var wide = NativeActionLayout.Arrange(sizes, 400);
        Check(wide.All(s => s.Y == 0), "wide columns retain a single action row");
        var oversized = NativeActionLayout.Arrange([new(700, 50), new(50, 32)], 120);
        Check(oversized[0].Width == 120 && oversized[1].Y == 58, "long labels wrap inside the column without overlapping the next action");
        Check(NativeActionLayout.Arrange([], 0).Length == 0, "empty actions have no layout");
        Check(PreviewLayout.TerminalMaxHeight(536) == 180, "short windows reserve room for composer and send actions");
        var screenshot = PreviewLayout.Workspace(800, true, false, 360);
        Check(PreviewLayout.CompactConversationToolbar(screenshot.ContentWidth), "screenshot-size conversation keeps title and overflow on one row");
        Check(!PreviewLayout.StackComposerSend(screenshot.ContentWidth - 64), "screenshot-size composer keeps Send beside the tools");
        Check(PreviewLayout.StackComposerSend(320) && !PreviewLayout.CompactConversationToolbar(900), "very narrow composers wrap while wide headers expose actions");
        Check(PreviewLayout.StackComposerSend(479) && !PreviewLayout.StackComposerSend(480), "composer wraps the trailing controls only below the 480-DIP breakpoint");
        Check(PreviewLayout.TerminalMaxHeight(900) == 520 && PreviewLayout.TerminalMaxHeight(1600) == 600, "taller windows preserve terminal resize space");
        Console.WriteLine("Native action wrapping and narrow-column bounds passed.");
    }
    private static void Check(bool ok, string description) { if (!ok) throw new InvalidOperationException(description); }
}
