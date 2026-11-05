namespace Wisp.ProjectBrowser;

public sealed record NativeSourceRun(int Start, int End, string Text);

/// <summary>A source selection captured before an action takes keyboard focus.</summary>
public sealed record NativeSourceSelection(string Text, int Start, int Length, NativeDocumentSelection Selection)
{
    public static NativeSourceSelection? CaptureHighlighted(string text, IReadOnlyList<NativeSourceRun> runs,
        int start, int end, string selected)
    {
        if (start < 0 || end <= start || runs.Count == 0) return null;
        var length = 0; var previousEnd = 0; var mappedStart = 0; var mappedEnd = 0;
        foreach (var run in runs)
        {
            if (run.Start < previousEnd || run.End - run.Start != run.Text.Length
                || length > text.Length - run.Text.Length
                || !text.AsSpan(length, run.Text.Length).SequenceEqual(run.Text)) return null;
            // XAML adds element boundary symbols between syntax runs. Those
            // boundaries have no source characters; only each Run's content
            // contributes to UTF-16 positions in the original source.
            mappedStart += Math.Clamp(start - run.Start, 0, run.Text.Length);
            mappedEnd += Math.Clamp(end - run.Start, 0, run.Text.Length);
            length += run.Text.Length; previousEnd = run.End;
        }
        return length == text.Length ? CaptureReadonly(text, mappedStart, mappedEnd, selected) : null;
    }

    public static NativeSourceSelection? CaptureReadonly(string text, int start, int end, string selected)
    {
        // TextPointer offsets must agree with the displayed characters, including
        // selections crossing syntax runs. Never locate repeated text by search.
        if (end <= start) return null;
        var value = Capture(text, text, start, end - start);
        return value?.Selection.Text == selected.Replace("\r\n", "\n").Replace('\r', '\n') ? value : null;
    }

    public static NativeSourceSelection? Capture(string original, string current, int start, int length)
    {
        static string Lines(string value) => value.Replace("\r\n", "\n").Replace('\r', '\n');
        var selection = NativeDocumentSelection.FromSource(current, start, length, false, Lines(original) != Lines(current));
        return selection == null ? null : new(current, start, length, selection);
    }

    // A menu may keep its selection while focus moves, but must not quote an
    // earlier revision after an editor change, even when the offsets still fit.
    public NativeDocumentSelection? ForText(string current, bool jump)
        => current == Text ? Selection with { Jump = jump } : null;
}
