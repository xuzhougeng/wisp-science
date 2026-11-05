using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativePanelEmptyState : StackPanel
{
    public NativePanelEmptyState(WispDesign design, string panel, bool filtered = false)
    {
        var copy = NativePanelEmptyCopy.For(panel, design.Localization);
        if (filtered) copy = NativePanelEmptyCopy.For(panel, design.Localization, true);
        Spacing = 8; Padding = new(24); HorizontalAlignment = HorizontalAlignment.Center;
        VerticalAlignment = VerticalAlignment.Center;
        Children.Add(new Microsoft.UI.Xaml.Shapes.Rectangle { Width = 28, Height = 28, RadiusX = 6, RadiusY = 6,
            Stroke = design.Brush("border-strong"), StrokeThickness = 1.5, StrokeDashArray = new DoubleCollection { 2, 2 },
            Fill = design.Brush("bg-sunken"), HorizontalAlignment = HorizontalAlignment.Center });
        var title = design.Text(copy.Title, 13, "text-muted"); title.TextAlignment = TextAlignment.Center;
        title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; Children.Add(title);
        var hint = design.Text(copy.Body, 12, "text-faint"); hint.TextAlignment = TextAlignment.Center; hint.MaxWidth = 260;
        design.BindLineHeight(hint, 12, 1.5); Children.Add(hint);
    }
}
