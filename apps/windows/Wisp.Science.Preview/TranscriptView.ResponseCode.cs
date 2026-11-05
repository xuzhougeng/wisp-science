using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Windows.ApplicationModel.DataTransfer;

namespace Wisp.Science.Preview;

internal static partial class TranscriptView
{
    private static FrameworkElement ResponseCode(string value, string? info, WispDesign design)
    {
        // Markdown's HTML textContent uses LF. Keeping CR/LF on separate syntax
        // runs would also make XAML render an extra blank line after comments.
        value = NativeResponseMarkdown.CodeText(value);
        var size = design.FontSize(12.5);
        var body = new TextBlock { FontFamily = design.Font(true), FontSize = size,
            Foreground = design.Brush("syntax-text"), LineHeight = size * 1.55,
            LineStackingStrategy = LineStackingStrategy.MaxHeight, IsTextSelectionEnabled = true,
            TextWrapping = TextWrapping.NoWrap, Padding = new Thickness(14, 12, 48, 12) };
        ApplyHighlight(body.Inlines, value, NativeCodeHighlight.NormalizeLanguage(info), design, syntax: true);
        var copy = design.ToolButton(design.T("tool.copy_code"), "copy");
        copy.Width = copy.Height = 28; copy.Padding = new(0); copy.CornerRadius = new(6);
        copy.HorizontalAlignment = HorizontalAlignment.Right; copy.VerticalAlignment = VerticalAlignment.Top; copy.Margin = new(8);
        copy.Background = design.Brush("bg-elev"); copy.BorderBrush = design.Brush("border"); copy.BorderThickness = new(1);
        copy.Click += (_, _) => { try { var data = new DataPackage(); data.SetText(value); Clipboard.SetContent(data); } catch { } };
        var content = new Grid(); content.Children.Add(new ScrollViewer { Content = body,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled });
        content.Children.Add(copy);
        return new Border { Child = content, Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"),
            BorderThickness = new(1), CornerRadius = new(10), Margin = new(0, 0, 0, 12) };
    }
}
