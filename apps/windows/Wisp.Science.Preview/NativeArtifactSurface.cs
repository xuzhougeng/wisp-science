using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>The same file renderer is used by side previews, artifact dialogs and center tabs.</summary>
internal sealed class NativeArtifactSurface(WispDesign design, Action escape,
    Func<NativePanelArtifact, NativePanelFileContent, NativeDocumentSelection, bool>? quote = null) : ContentControl, IDisposable
{
    private NativeRichPreview? rich;
    private NativeCodeView? code;
    private NativeArtifactTextActions? textActions;
    private NativeRasterPreview? raster;
    private (NativePanelArtifact, NativePanelFileContent?, string?, double, double, bool, string, string, NativeTypography, string)? rendered;
    private int revision;
    private bool disposed;
    public bool HandleEscape()
    {
        return textActions?.HandleEscape() == true || code?.HandleEscape() == true || rich?.DismissSelection() == true;
    }
    public bool HandlePdfKey(int key) => rich?.HandlePdfKey(key) == true;
    public async void Present(NativePanelArtifact item, NativePanelFileContent? value, bool loading, string? error, double width, double height)
    {
        var state = (item, value, error, width, height, design.Dark, design.LightPalette, design.DarkPalette, design.Typography, design.Localization.Locale);
        if (disposed || rendered is { } same && same == state && ReferenceEquals(same.Item1, item) && ReferenceEquals(same.Item2, value)) return;
        if ((raster != null || rich?.IsPdf == true) && rendered is { } previous
            && ReferenceEquals(previous.Item1, item) && ReferenceEquals(previous.Item2, value)
            && (previous.Item1, previous.Item2, previous.Item3, width, height, previous.Item6, previous.Item7, previous.Item8, previous.Item9, previous.Item10) == state)
        { rendered = state; raster?.Resize(width, height); rich?.ResizePdf(height); return; }
        rendered = state; var current = ++revision;
        rich?.Dispose(); rich = null; code?.Dispose(); code = null; textActions?.Dispose(); textActions = null; raster = null; Content = null;
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        if (loading) { Content = new ProgressRing { IsActive = true, Width = 28, Height = 28, Margin = new(20) }; rendered = null; return; }
        if (value == null) { Content = Note(error ?? design.L("暂无预览", "Preview unavailable")); return; }
        try
        {
            FrameworkElement preview;
            if (value.Text is { } csv && (value.Mime == "text/csv" || value.Path.EndsWith(".csv", StringComparison.OrdinalIgnoreCase)))
                preview = NativeCsvPreview.Create(NativeCsvTable.Parse(csv, value.Truncated), design, height);
            else if (value.Text != null && NativeDocumentPreview.Kind(value.Mime) == null && NativeScientificPreview.Kind(value.Path) == null && value.Mime != "image/svg+xml")
            {
                var language = NativeCodePreview.FileLanguage(value.Path, item.Name, item.Kind);
                var head = NativeArtifactTextPreview.Create(value with { Text = NativeCodePreview.FormatFile(value.Text, language) });
                var body = new StackPanel { Spacing = 8 };
                if (head.Notice(design.Localization) is { } notice) body.Children.Add(Note(notice));
                if (NativeArtifactTextPreview.IsMarkdown(value.Path, item.Name, item.Kind))
                {
                    var markdown = TranscriptView.RenderMarkdown(head.Text, design, skillDocument: true);
                    if (quote != null && NativeArtifactSelection.PreviewText(value, item.Name, item.Kind) is { } displayed)
                        textActions = new(markdown, displayed, design, () => !disposed && current == revision && IsLoaded,
                            selection => quote(item, value, selection));
                    body.Children.Add(markdown);
                }
                else
                {
                    var hasSourcePosition = NativeArtifactSelection.SourceText(value, item.Name, item.Kind) != null;
                    code = new(head.Text, language, design, height,
                        quote == null ? null : selection => !disposed && current == revision && IsLoaded
                            && quote(item, value, hasSourcePosition ? selection : selection with { Location = new("preview") }));
                    body.Children.Add(code);
                }
                preview = body;
            }
            else if (value.Truncated) preview = Note(design.L("内容过大，预览已截断。", "This preview is truncated."));
            else if (value.Base64 != null && value.Mime.StartsWith("image/") && value.Mime != "image/svg+xml")
                preview = await NativeRasterPreview.CreateAsync(value, item.Name, design, width, height);
            else if ((NativeDocumentPreview.Kind(value.Mime) ?? (value.Mime == "image/svg+xml" ? "image" : NativeScientificPreview.Kind(value.Path))) is { } kind)
                preview = new NativeRichPreview(design, kind, NativeScientificPreview.Kind(value.Path) != null ? value.Text ?? "" : value.Base64 ?? "", item.Name,
                    quote: quote == null || NativeDocumentPreview.Kind(value.Mime) == null ? null : selection =>
                        !disposed && current == revision && IsLoaded && quote(item, value, selection),
                    escape: escape, format: value.Mime == "image/svg+xml" ? value.Mime : NativeScientificPreview.Format(value.Path)) { MaxHeight = height };
            else if (value.Text is { } text)
                preview = code = new(text, NativeCodePreview.FileLanguage(value.Path, item.Name, item.Kind), design, height);
            else preview = Note(design.L("无法预览此文件。", "Preview is unavailable for this file."));
            if (disposed || current != revision) { (preview as IDisposable)?.Dispose(); return; }
            rich = preview as NativeRichPreview; rich?.ResizePdf(height); raster = preview as NativeRasterPreview; Content = preview;
        }
        catch (Exception ex) { if (!disposed && current == revision) Content = Note(ex.Message); }
    }
    private TextBlock Note(string value) { var text = design.Text(value, 12.5); text.Foreground = design.Brush("text-faint"); text.Margin = new(8); return text; }
    public void Dispose() { if (disposed) return; disposed = true; revision++; textActions?.Dispose(); code?.Dispose(); rich?.Dispose(); Content = null; }
}
