using Wisp.ProjectBrowser.Contracts;
using System.Text.RegularExpressions;

namespace Wisp.ProjectBrowser;

/// <summary>Validate a selection against the mounted artifact's original content.</summary>
public static class NativeArtifactSelection
{
    public static string? PreviewText(NativePanelFileContent content, string name, string kind)
    {
        if (content.Text == null || NativeDocumentPreview.Kind(content.Mime) != null
            || NativeScientificPreview.Kind(content.Path) != null || content.Mime == "image/svg+xml"
            || content.Mime == "text/csv" || content.Path.EndsWith(".csv", StringComparison.OrdinalIgnoreCase)) return null;
        var language = NativeCodePreview.FileLanguage(content.Path, name, kind);
        var displayed = NativeArtifactTextPreview.Create(content with { Text = NativeCodePreview.FormatFile(content.Text, language) }).Text;
        return NativeArtifactTextPreview.IsMarkdown(content.Path, name, kind)
            ? NativeSkillMarkdown.VisibleText(displayed) : NativeCodePreview.Create(displayed, language).Text;
    }

    public static bool ContainsPreviewText(string displayed, string selected)
    {
        // RichTextBlock represents paragraph and soft-line separators differently
        // from HTML. Compare visible text while preserving the exact quote payload.
        static string Normalize(string value) => Regex.Replace(value, @"\s+", " ").Trim();
        var text = Normalize(selected);
        return text.Length > 0 && !selected.Contains('\uFFFC') && Normalize(displayed).Contains(text, StringComparison.Ordinal);
    }

    public static string? SourceText(NativePanelFileContent content, string name, string kind)
    {
        if (content.Text == null || content.Truncated || NativeDocumentPreview.Kind(content.Mime) != null
            || NativeScientificPreview.Kind(content.Path) != null || content.Mime == "image/svg+xml"
            || content.Mime == "text/csv" || content.Path.EndsWith(".csv", StringComparison.OrdinalIgnoreCase)
            || NativeArtifactTextPreview.IsMarkdown(content.Path, name, kind)) return null;
        var language = NativeCodePreview.FileLanguage(content.Path, name, kind);
        var original = NativeCodePreview.Create(content.Text, language).Text;
        var displayed = NativeCodePreview.Create(NativeCodePreview.FormatFile(content.Text, language), language).Text;
        // Pretty-printed JSON lines aren't positions in the original file.
        // Keep its formatted display and standard copy commands, but do not
        // fabricate source locations until there is an explicit source map.
        if (original != displayed || NativeArtifactTextPreview.Create(content).Truncated) return null;
        return original;
    }

    public static bool Allows(NativePanelFileContent content, string name, string kind, NativeDocumentSelection selection)
    {
        if (!selection.Valid) return false;
        if (selection.Location.Kind == "preview")
            return PreviewText(content, name, kind) is { } displayed && ContainsPreviewText(displayed, selection.Text);
        if (content.Truncated) return false;
        if (selection.Location.Kind != "source")
            return !string.IsNullOrEmpty(content.Base64) && NativeDocumentPreview.Kind(content.Mime) == selection.Location.Kind;
        if (selection.Location.Unsaved || SourceText(content, name, kind) is not { } source) return false;
        // Both text and reported line range must match the immutable read. The
        // renderer never gets to replace the reader's file identity or content.
        var first = selection.Location.Line!.Value;
        var start = 0;
        for (var line = 1; line < first; line++)
        { var next = source.IndexOf('\n', start); if (next < 0) return false; start = next + 1; }
        var lineEnd = source.IndexOf('\n', start);
        if (lineEnd < 0) lineEnd = source.Length;
        var index = source.IndexOf(selection.Text, start, StringComparison.Ordinal);
        return index >= start && index <= lineEnd
            && NativeDocumentSelection.FromSource(source, index, selection.Text.Length, selection.Jump)?.Location == selection.Location;
    }
}
