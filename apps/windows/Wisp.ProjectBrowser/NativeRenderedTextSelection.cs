using System.Text;

namespace Wisp.ProjectBrowser;

public sealed record NativeRenderedTextSpan(int Start, int End, string Text, bool Atomic = false);

/// <summary>Selection over rendered runs, paragraph separators and inline code.
/// Atomic inline elements contribute their visible text, never object markers.</summary>
public static class NativeRenderedTextSelection
{
    public static string Capture(IEnumerable<NativeRenderedTextSpan> spans, int start, int end, int maxLength = 32768)
    {
        if (start < 0 || end <= start) return "";
        var text = new StringBuilder();
        foreach (var span in spans)
        {
            if (span.End < span.Start || !span.Atomic && span.End - span.Start != span.Text.Length) return "";
            if (start >= span.End || end <= span.Start) continue;
            if (span.Atomic) text.Append(span.Text);
            else
            {
                var first = Math.Max(start, span.Start) - span.Start;
                var count = Math.Min(end, span.End) - span.Start - first;
                text.Append(span.Text, first, count);
            }
            if (text.Length > maxLength) return "";
        }
        return text.ToString();
    }
}
