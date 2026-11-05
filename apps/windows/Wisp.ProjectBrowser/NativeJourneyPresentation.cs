using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public readonly record struct NativeJourneyLayout(bool Stacked, bool Narrow, double Inset, double AsideWidth, double DateWidth)
{
    public static NativeJourneyLayout ForWidth(double width) => new(width <= 960, width <= 640, width <= 640 ? 16 : width <= 1200 ? 20 : 28, width <= 1200 ? 238 : 342, width <= 640 ? 68 : width <= 1200 ? 82 : 104);
}

public readonly record struct JourneyGraphPoint(double X, double Y);
public readonly record struct JourneyGraphSegment(JourneyGraphPoint Control1, JourneyGraphPoint Control2, JourneyGraphPoint End, bool IsLine = false);
public sealed record JourneyGraphRoute(JourneyGraphPoint Start, JourneyGraphPoint Label, JourneyGraphSegment[] Segments)
{
    // Match research.rs: long edges travel between rows instead of crossing the
    // nodes in intervening columns. The final tangent defines arrow direction.
    public static JourneyGraphRoute Between(double sourceX, double sourceY, double targetX, double targetY)
    {
        if (sourceX == targetX)
        {
            var down = sourceY < targetY;
            var startY = sourceY + (down ? 58 : 0); var endY = targetY + (down ? 0 : 58);
            var centerX = sourceX + 88; var sideX = sourceX + 210;
            return new(new(centerX, startY), new(sideX, (startY + endY) / 2 - 5), [new(new(sideX, startY), new(sideX, endY), new(centerX, endY))]);
        }
        var direction = sourceX < targetX ? 1 : -1;
        var start = new JourneyGraphPoint(sourceX + (direction > 0 ? 176 : 0), sourceY + 29);
        var end = new JourneyGraphPoint(targetX + (direction > 0 ? 0 : 176), targetY + 29);
        var middleX = (start.X + end.X) / 2;
        if (Math.Abs(sourceX - targetX) <= 240)
            return new(start, new(middleX, (start.Y + end.Y) / 2 - 5), [new(new(middleX, start.Y), new(middleX, end.Y), end)]);
        var lane = Math.Max(start.Y, end.Y) + 47;
        var laneStart = new JourneyGraphPoint(start.X + direction * 42, lane);
        var laneEnd = new JourneyGraphPoint(end.X - direction * 42, lane);
        return new(start, new(middleX, lane - 5), [
            new(new(start.X + direction * 24, start.Y), new(start.X + direction * 24, lane), laneStart),
            new(laneStart, laneEnd, laneEnd, true),
            new(new(end.X - direction * 24, lane), new(end.X - direction * 24, end.Y), end)]);
    }
}

public static class NativeJourneyPresentation
{
    public static JourneyEntry Headline(JourneyDay day) => day.Entries.FirstOrDefault(e => e.Kind == "archive")
        ?? day.Entries.FirstOrDefault(e => e.Kind == "progress") ?? day.Entries.FirstOrDefault(e => e.Kind == "decision")
        ?? day.Entries.FirstOrDefault(e => e.Kind == "run" && e.Status != "started") ?? day.Entries[0];
    public static IEnumerable<JourneyEntry> Notes(JourneyDay day) => day.Entries.Where(e => e.Kind is not ("artifact" or "session" or "run"));
    public static int Experiments(JourneyDay day) => day.Entries.Where(e => e.Kind == "run").Select(e => e.SourceId).Distinct().Count();
    public static string Icon(string kind) => kind switch { "finding" => "lightbulb", "decision" => "fork", "next" => "play", "run" => "flask", "session" => "chat", "archive" => "archive", "paper" => "book", "data_asset" or "data" => "database", _ => "doc" };
    public static string Label(string kind, bool chinese) => (kind, chinese) switch
    {
        ("progress", true) => "进展", ("finding", true) => "发现", ("decision", true) => "决策", ("next", true) => "待继续", ("run", true) => "实验", ("session", true) => "会话", ("archive", true) => "研究归档", ("paper", true) => "文献", ("data_asset" or "data", true) => "数据", (_, true) => "产出",
        ("progress", _) => "Progress", ("finding", _) => "Finding", ("decision", _) => "Decision", ("next", _) => "Next step", ("run", _) => "Experiment", ("session", _) => "Conversation", ("archive", _) => "Research archive", ("paper", _) => "Paper", ("data_asset" or "data", _) => "Data", _ => "Output"
    };
    // An edited recap uses one item per line, as in the WebView editor. Source
    // indexes are deliberately cleared because the text has changed.
    public static JourneyRecapItem[] RecapItems(string text) => text.ReplaceLineEndings("\n").Split('\n').Select(s => s.Trim()).Where(s => s.Length > 0).Select(s => new JourneyRecapItem(s, [])).ToArray();
}
