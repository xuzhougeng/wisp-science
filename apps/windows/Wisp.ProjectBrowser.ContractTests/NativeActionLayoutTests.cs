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
        var nudge = NativeActionLayout.Arrange([new(150, 18), new(260, 26)], 600, gap: 14, spaceBetween: true, verticallyCentered: true);
        Check(nudge[0].X == 0 && nudge[0].Y == 4 && nudge[1].X == 340 && nudge[1].Y == 0, "Context nudge keeps text left and actions right when they fit.");
        var wrappedNudge = NativeActionLayout.Arrange([new(150, 18), new(260, 26)], 300, gap: 14, rowGap: 10, spaceBetween: true, verticallyCentered: true);
        Check(wrappedNudge[1].X == 0 && wrappedNudge[1].Y == 28, "Nudge actions wrap left without overlapping their explanation.");
        Check(wide.All(s => s.Y == 0), "wide columns retain a single action row");
        var centered = NativeActionLayout.Arrange(sizes, 200, rightAligned: true, rowGap: 12, verticallyCentered: true);
        Check(centered[0].Y == 4 && centered[1].Y == 0 && centered[2].Y == 52,
            "mixed-size heading icons and badges center within their own wrapped row without shifting the next row");
        Check(centered[0].X == 12 && centered[1].X == 100 && centered[2].X == 130,
            "vertical centering preserves right alignment and action order");
        var unconstrainedHeading = NativeActionLayout.Arrange([new(24, 24), new(100, 28), new(44, 16)], double.PositiveInfinity, gap: 12, verticallyCentered: true);
        Check(unconstrainedHeading[0].Y == 2 && unconstrainedHeading[1].Y == 0 && unconstrainedHeading[2].Y == 6
            && unconstrainedHeading.All(slot => double.IsFinite(slot.X)), "natural-width headings center their badge without nonfinite offsets");
        // Real WinUI labels have fractional desired widths. Exercise both the
        // natural-width toolbar and a constrained footer across display scales.
        NativeActionLayout.Size[] fractional = [new(183.31, 36), new(125.67, 36), new(126.31, 36)];
        foreach (var available in new[] { double.PositiveInfinity, 991.33, 562.67, 350.33 })
        {
            var constraint = NativeActionLayout.MeasureConstraint(available);
            var measured = NativeActionLayout.Arrange(fractional, constraint);
            foreach (var aligned in new[] { false, true })
            {
                var desired = NativeActionLayout.DesiredWidth(measured, constraint, aligned);
                foreach (var scale in new[] { 1d, 1.25, 1.5, 2d })
                {
                    var rounded = Math.Round(desired * scale) / scale;
                    var arranged = NativeActionLayout.Arrange(fractional, rounded, rightAligned: aligned);
                    Check(arranged.Select(slot => slot.Y).SequenceEqual(measured.Select(slot => slot.Y)),
                        "display rounding must not add an unmeasured row or clip the final action");
                }
            }
        }
        var modelActions = NativeActionLayout.Arrange([new(132, 36), new(84, 36), new(84, 36)], 527.33, rightAligned: true);
        Check(modelActions.All(s => s.Y == 0) && Math.Abs(modelActions[^1].X + modelActions[^1].Width - 527.33) < .001,
            "model Test, Cancel and Save use the full measured footer width without clipping Save");
        var modelTabs = NativeActionLayout.Arrange([new(110, 32), new(110, 32)], 226, gap: 6);
        Check(modelTabs[1].X == 116 && modelTabs[1].Y == 0, "model tabs retain the six-pixel gap at the exact fit boundary");
        var providerRoles = NativeActionLayout.Arrange([new(180, 20), new(160, 20), new(150, 20)], 354, gap: 14, rowGap: 8);
        Check(providerRoles[1].X == 194 && providerRoles[1].Y == 0 && providerRoles[2].Y == 28,
            "provider roles use independent horizontal and vertical gaps when they wrap");
        NativeActionLayout.Size[] reviewer = [new(116.4, 36), new(84, 36), new(84, 36)];
        var reviewerFooter = NativeActionLayout.Arrange(reviewer, 956.67, rightAligned: true);
        Check(reviewerFooter.All(s => s.Y == 0) && Math.Abs(reviewerFooter[^1].X + reviewerFooter[^1].Width - 956.67) < 0.001,
            "Reviewer, Cancel and Save share a right-aligned row using the full measured footer width");
        var compactFooter = NativeActionLayout.Arrange(reviewer, 220, rightAligned: true);
        Check(compactFooter[2].Y == 44 && compactFooter[2].X == 136
            && compactFooter.All(s => s.X >= 0 && s.X + s.Width <= 220), "narrow right-aligned footer preserves order and reserves the final button row");
        var oversized = NativeActionLayout.Arrange([new(700, 50), new(50, 32)], 120);
        Check(oversized[0].Width == 120 && oversized[1].Y == 58, "long labels wrap inside the column without overlapping the next action");
        Check(NativeActionLayout.Arrange([], 0).Length == 0, "empty actions have no layout");
        Check(PreviewLayout.TerminalMaxHeight(536) == 180, "short windows reserve room for composer and send actions");
        var screenshot = PreviewLayout.Workspace(760, true, false, 360);
        Check(PreviewLayout.CompactConversationToolbar(screenshot.ContentWidth), "screenshot-size conversation keeps title and overflow on one row");
        Check(!PreviewLayout.CompactComposer(441) && PreviewLayout.CompactComposer(440), "compact control sizing follows the chat pane, even in a wide window");
        var pickers = PreviewLayout.FitComposerPickers(386, 82, 80, 164, 101);
        Check(!pickers.Wrap && pickers.Model < 164 && pickers.Effort < 101 && Math.Abs(pickers.Model + pickers.Effort - 224) < .01,
            "both labels shrink on one row beside a docked panel; Send retains its full width");
        Check(PreviewLayout.FitComposerPickers(320, 65, 100, 0, 100) == (0d, 100d, false),
            "a hidden model does not consume space or force a second row");
        Check(PreviewLayout.FitComposerPickers(240, 65, 116, 140, 100).Wrap,
            "move to a second row if fixed running actions and the two picker icons cannot fit");
        Check(PreviewLayout.TerminalMaxHeight(900) == 520 && PreviewLayout.TerminalMaxHeight(1600) == 600, "taller windows preserve terminal resize space");
        Console.WriteLine("Native action wrapping and narrow-column bounds passed.");
    }
    private static void Check(bool ok, string description) { if (!ok) throw new InvalidOperationException(description); }
}
