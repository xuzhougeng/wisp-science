using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;

namespace Wisp.Science.Preview;

/// <summary>The two viewport-relative accent washes from projects.css.</summary>
internal sealed class NativeHomeBackdrop : Grid
{
    public NativeHomeBackdrop(WispDesign design, UIElement content)
    {
        Background = design.Brush("bg-app");
        // CSS paints the first background on top. These layers stay with the
        // viewport while the project list scrolls, and never intercept input.
        AddGlow(design, .92, .12, .40, .28, .07, .70);
        AddGlow(design, .50, -.08, .70, .42, .16, .58);
        Children.Add(content);
    }

    private void AddGlow(WispDesign design, double x, double y, double rx, double ry, double opacity, double fade)
    {
        var accent = design.Brush("clay").Color;
        var gradient = new RadialGradientBrush
        {
            MappingMode = BrushMappingMode.RelativeToBoundingBox,
            Center = new Point(x, y), GradientOrigin = new Point(x, y), RadiusX = rx, RadiusY = ry
        };
        gradient.GradientStops.Add(new GradientStop { Offset = 0,
            Color = Windows.UI.Color.FromArgb((byte)Math.Round(accent.A * opacity), accent.R, accent.G, accent.B) });
        gradient.GradientStops.Add(new GradientStop { Offset = fade,
            Color = Windows.UI.Color.FromArgb(0, accent.R, accent.G, accent.B) });
        Children.Add(new Border { Background = gradient, IsHitTestVisible = false });
    }
}
