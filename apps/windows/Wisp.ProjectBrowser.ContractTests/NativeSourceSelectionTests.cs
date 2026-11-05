using Wisp.ProjectBrowser;

internal static class NativeSourceSelectionTests
{
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    public static void Run()
    {
        const string original = "first\r\nalpha = 42\r\nlast";
        const string normalized = "first\ralpha = 42\rlast";
        var selected = NativeSourceSelection.Capture(original, normalized, 6, 10)!;
        Check(selected.Selection is { Text: "alpha = 42", Location: { Line: 2, EndLine: 2, Unsaved: false } },
            "TextBox newline normalization does not mark unchanged source as an unsaved edit");
        Check(selected.ForText(normalized, true) is { Jump: true, Text: "alpha = 42" }
            && selected.Selection.Jump == false, "focus changes retain the captured quote and jump is an action intent");
        Check(selected.ForText(normalized.Replace("42", "99"), false) == null,
            "editing after opening a selection menu retires its captured source revision");
        var edited = normalized.Replace("42", "99");
        Check(NativeSourceSelection.Capture(original, edited, 6, 15)?.Selection is
            { Text: "alpha = 99\rlast", Location: { Line: 2, EndLine: 3, Unsaved: true } },
            "an unsaved multiline selection retains the edited text and exact line span");
        Check(NativeSourceSelection.Capture(original, normalized, 0, 0) == null
            && NativeSourceSelection.Capture(original, normalized, 100, 1) == null
            && NativeSourceSelection.Capture("😀", "😀", 0, 1) == null
            && NativeSourceSelection.Capture(" ", " ", 0, 1) == null,
            "empty, unavailable, split-surrogate and blank selections do not expose quote actions");
        Check(NativeSourceSelection.Capture("x", new string('x', 32769), 0, 32769) == null,
            "selection popup obeys the document quote bound");
        const string highlighted = "# 示例 😀\nalpha = 42\nalpha = 42\nprint(alpha)";
        var second = highlighted.LastIndexOf("alpha = 42", StringComparison.Ordinal);
        Check(NativeSourceSelection.CaptureReadonly(highlighted, second, highlighted.Length, "alpha = 42\r\nprint(alpha)")?.Selection is
            { Text: "alpha = 42\nprint(alpha)", Location: { Line: 3, EndLine: 4, Unsaved: false } },
            "readonly offsets identify repeated and multiline syntax text after Unicode, without searching for the first match");
        Check(NativeSourceSelection.CaptureReadonly(highlighted, second + 1, second + 6, "alpha") == null
            && NativeSourceSelection.CaptureReadonly(highlighted, second, second, "alpha") == null,
            "disagreement between XAML positions and selected text fails closed");
        NativeSourceRun[] runs = [new(2, 19, "# Synthetic 中文 😀"), new(21, 30, "\nalpha = "),
            new(32, 34, "42"), new(36, 45, "\nalpha = "), new(47, 49, "42"), new(51, 65, "\nprint(alpha)\n")];
        var rendered = string.Concat(runs.Select(run => run.Text));
        Check(NativeSourceSelection.CaptureHighlighted(rendered, runs, 37, 52, "alpha = 42\n")?.Selection is
            { Text: "alpha = 42\n", Location: { Line: 3, EndLine: 3 } },
            "observed WinUI symbol offsets map repeated, colored source with a trailing newline to line three");
        Check(NativeSourceSelection.CaptureHighlighted(rendered, runs, 22, 65, "alpha = 42\nalpha = 42\nprint(alpha)\n")?.Selection is
            { Location: { Line: 2, EndLine: 4 } }, "multiline selections span several syntax runs without including element markers");
        Check(NativeSourceSelection.CaptureHighlighted(rendered, runs, 19, 21, "") == null
            && NativeSourceSelection.CaptureHighlighted(rendered + "changed", runs, 37, 52, "alpha = 42\n") == null,
            "formatting-only ranges and changed highlighted source cannot quote");
    }
}
