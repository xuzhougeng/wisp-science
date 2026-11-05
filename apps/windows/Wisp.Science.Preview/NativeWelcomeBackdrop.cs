using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>Welcome's content-sized accent wash, including live palette changes.</summary>
internal sealed class NativeWelcomeBackdrop : Grid
{
    public NativeWelcomeBackdrop(WispDesign design, FrameworkElement content)
    {
        HorizontalAlignment = HorizontalAlignment.Center;
        VerticalAlignment = VerticalAlignment.Center;
        MaxWidth = PreviewLayout.ConversationMaxWidth;
        var accent = design.Brush("clay");
        var gradient = new RadialGradientBrush { MappingMode = BrushMappingMode.Absolute };
        var wash = new Border { Background = gradient, IsHitTestVisible = false,
            HorizontalAlignment = HorizontalAlignment.Left, VerticalAlignment = VerticalAlignment.Top };
        // A Canvas keeps decoration out of the content's desired-size calculation.
        var decoration = new Canvas { IsHitTestVisible = false };
        decoration.Children.Add(wash); Children.Add(decoration); Children.Add(content);
        SizeChanged += (_, e) =>
        {
            var bounds = PreviewLayout.WelcomeGlow(e.NewSize.Width, e.NewSize.Height);
            wash.Visibility = bounds.Width > 0 && bounds.Height > 0 ? Visibility.Visible : Visibility.Collapsed;
            if (wash.Visibility == Visibility.Collapsed) return;
            Canvas.SetLeft(wash, bounds.Left); Canvas.SetTop(wash, bounds.Top);
            wash.Width = bounds.Width; wash.Height = bounds.Height;
            gradient.Center = gradient.GradientOrigin = new Point(bounds.Width / 2, bounds.Height / 2);
            gradient.RadiusX = bounds.RadiusX; gradient.RadiusY = bounds.RadiusY;
        };
        void UpdateColor()
        {
            var color = accent.Color;
            // Replacing stop colors updates their properties but can leave WinUI's
            // radial composition brush showing the old colors. Replace the brush
            // while retaining geometry so a retained page repaints immediately.
            gradient = new RadialGradientBrush { MappingMode = BrushMappingMode.Absolute,
                Center = gradient.Center, GradientOrigin = gradient.GradientOrigin,
                RadiusX = gradient.RadiusX, RadiusY = gradient.RadiusY };
            gradient.GradientStops.Add(new GradientStop { Offset = 0,
                Color = Windows.UI.Color.FromArgb((byte)Math.Round(color.A * .14), color.R, color.G, color.B) });
            gradient.GradientStops.Add(new GradientStop { Offset = .68,
                Color = Windows.UI.Color.FromArgb(0, color.R, color.G, color.B) });
            wash.Background = gradient;
        }
        long? colorToken = null;
        Loaded += (_, _) =>
        {
            UpdateColor();
            colorToken ??= accent.RegisterPropertyChangedCallback(SolidColorBrush.ColorProperty, (_, _) => UpdateColor());
        };
        Unloaded += (_, _) =>
        {
            if (colorToken is { } token) accent.UnregisterPropertyChangedCallback(SolidColorBrush.ColorProperty, token);
            colorToken = null;
        };
        UpdateColor();
    }
}
