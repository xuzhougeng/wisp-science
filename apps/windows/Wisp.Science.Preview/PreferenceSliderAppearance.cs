using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;

namespace Wisp.Science.Preview;

// Keep Slider's native drag, keyboard, range and automation implementation.
// Only the horizontal font slider's stock decoration changes.
internal static class PreferenceSliderAppearance
{
    public static void Apply(Slider slider, WispDesign design)
    {
        var accent = design.Brush("clay");
        var track = design.Brush("bg-sunken");
        foreach (var state in new[] { "", "PointerOver", "Pressed" })
        {
            slider.Resources["SliderThumbBackground" + state] = accent;
            slider.Resources["SliderTrackValueFill" + state] = accent;
            slider.Resources["SliderTrackFill" + state] = track;
        }
        slider.Resources["SliderOuterThumbBackground"] = accent;
        slider.Resources["SliderThumbBorderBrush"] = accent;
        slider.Resources["SliderHorizontalThumbWidth"] = 16d;
        slider.Resources["SliderHorizontalThumbHeight"] = 16d;
        slider.Resources["SliderTrackThemeHeight"] = 8d;
        slider.Resources["SliderThumbCornerRadius"] = new CornerRadius(8);
        slider.CornerRadius = new CornerRadius(4);
        slider.IsThumbToolTipEnabled = false; // The adjacent px output already shows the value.
        slider.FocusVisualMargin = new Thickness(-2, 0, -2, 0);
        slider.FocusVisualPrimaryThickness = new Thickness(1);
        slider.FocusVisualSecondaryThickness = new Thickness(0);
        slider.FocusVisualPrimaryBrush = design.Brush("text");
        slider.Loaded += (_, _) => Mount();
        if (slider.IsLoaded) Mount();

        void Mount()
        {
            slider.ApplyTemplate();
            if (Find(slider, "HorizontalThumb") is Thumb thumb)
            {
                thumb.ApplyTemplate();
                if (VisualTreeHelper.GetChildrenCount(thumb) > 0 && VisualTreeHelper.GetChild(thumb, 0) is Border border)
                {
                    // WinUI's -2 margin creates the white 22-DIP outer ring.
                    // The WebView range uses one solid 16-DIP accent circle.
                    border.Margin = new Thickness(0);
                    border.BorderThickness = new Thickness(0);
                }
            }
            if (Find(slider, "HorizontalTrackRect") is Rectangle rail)
            {
                rail.Stroke = design.Brush("border-strong");
                rail.StrokeThickness = 1;
            }
        }
    }

    private static DependencyObject? Find(DependencyObject parent, string name)
    {
        if (parent is FrameworkElement element && element.Name == name) return element;
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
            if (Find(VisualTreeHelper.GetChild(parent, i), name) is { } found) return found;
        return null;
    }
}
