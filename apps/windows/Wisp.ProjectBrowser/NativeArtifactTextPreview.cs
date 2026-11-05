using System.Text;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

/// <summary>Keep a bounded text head visible, with the WebView's 8,000-line cap.</summary>
public sealed record NativeArtifactTextPreview(string Text, bool Truncated, ulong ShownBytes, ulong TotalBytes)
{
    public static NativeArtifactTextPreview Create(NativePanelFileContent content)
    {
        var source = content.Text ?? ""; var end = source.Length; var lines = 0;
        for (var i = 0; i < source.Length; i++)
            if (source[i] == '\n' && ++lines == 8_000) { end = i + 1; break; }
        var text = source[..end]; var shown = (ulong)Encoding.UTF8.GetByteCount(text);
        return new(text, content.Truncated || end < source.Length, shown, Math.Max(shown, content.TotalBytes ?? (ulong)Encoding.UTF8.GetByteCount(source)));
    }
    public static bool IsMarkdown(string path, string name, string kind)
        => (NativeGeneratedArtifacts.FileKind(path) ?? NativeGeneratedArtifacts.FileKind(name) ?? kind) is "markdown" or "document";
    public string? Notice(NativeLocalization locale) => !Truncated ? null : locale.Format("preview.text_truncated",
        ("shown", NativeStoragePresentation.FormatBytes(ShownBytes)), ("total", NativeStoragePresentation.FormatBytes(TotalBytes)));
}
