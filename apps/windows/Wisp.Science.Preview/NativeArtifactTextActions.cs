using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>Owns rendered Markdown selection surfaces for one mounted preview.</summary>
internal sealed class NativeArtifactTextActions : IDisposable
{
    private readonly FrameworkElement content;
    private readonly string displayed;
    private readonly WispDesign design;
    private readonly Func<bool> current;
    private readonly Func<NativeDocumentSelection, bool> quote;
    private readonly Dictionary<FrameworkElement, NativeSourceSelectionActions> actions = [];
    private bool disposed;
    public NativeArtifactTextActions(FrameworkElement content, string displayed, WispDesign design,
        Func<bool> current, Func<NativeDocumentSelection, bool> quote)
    {
        this.content = content; this.displayed = displayed; this.design = design; this.current = current; this.quote = quote;
        content.Loaded += Loaded;
        if (content.IsLoaded) Visit(content);
    }
    private void Loaded(object _, RoutedEventArgs __) => Visit(content);
    private void Visit(DependencyObject node)
    {
        if (disposed) return;
        if (node is RichTextBlock rich && rich.IsTextSelectionEnabled && !actions.ContainsKey(rich))
            actions.Add(rich, NativeSourceSelectionActions.ForPreview(rich, () => Selected(rich),
                () => rich.Select(rich.ContentStart, rich.ContentStart), displayed, design, Current, quote));
        else if (node is TextBlock text && text.IsTextSelectionEnabled && !actions.ContainsKey(text))
            actions.Add(text, NativeSourceSelectionActions.ForPreview(text, () => text.SelectedText,
                () => text.Select(text.ContentStart, text.ContentStart), displayed, design, Current, quote));
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(node); i++) Visit(VisualTreeHelper.GetChild(node, i));
    }
    private bool Current() => !disposed && content.IsLoaded && current();
    private static string Selected(RichTextBlock rich)
    {
        if (string.IsNullOrWhiteSpace(rich.SelectedText)) return "";
        var spans = new List<NativeRenderedTextSpan>();
        void Add(Inline inline)
        {
            switch (inline)
            {
                case Run run: spans.Add(new(run.ContentStart.Offset, run.ContentEnd.Offset, run.Text)); break;
                case Span span: foreach (var child in span.Inlines) Add(child); break;
                case LineBreak line: spans.Add(new(line.ContentStart.Offset, line.ContentEnd.Offset, "\n", true)); break;
                case InlineUIContainer ui:
                    // Markdown inline code is a selectable TextBlock inside its
                    // styled chip. RichTextBlock.SelectedText alone omits it.
                    var text = ui.Child is Border { Child: TextBlock code } ? code.Text : "";
                    spans.Add(new(ui.ContentStart.Offset, ui.ContentEnd.Offset, text, true)); break;
            }
        }
        var paragraphs = rich.Blocks.OfType<Paragraph>().ToArray();
        for (var i = 0; i < paragraphs.Length; i++)
        {
            foreach (var inline in paragraphs[i].Inlines) Add(inline);
            if (i + 1 < paragraphs.Length)
                spans.Add(new(paragraphs[i].ContentEnd.Offset, paragraphs[i + 1].ContentStart.Offset, "\n", true));
        }
        return NativeRenderedTextSelection.Capture(spans, rich.SelectionStart.Offset, rich.SelectionEnd.Offset, int.MaxValue);
    }
    public bool HandleEscape()
    {
        foreach (var action in actions.Values.Reverse()) if (action.HandleEscape()) return true;
        return false;
    }
    public void Dispose()
    {
        if (disposed) return; disposed = true; content.Loaded -= Loaded;
        foreach (var action in actions.Values) action.Dispose(); actions.Clear();
    }
}
