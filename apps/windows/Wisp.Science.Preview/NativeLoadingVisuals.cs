using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Windows.Foundation;
using Windows.UI.ViewManagement;

namespace Wisp.Science.Preview;

internal static class NativeLoadingVisuals
{
    public static FrameworkElement QueueDot(WispDesign design)
    {
        var halo = new Grid { Width = 12, Height = 12, VerticalAlignment = VerticalAlignment.Center };
        halo.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Fill = design.Brush("clay-soft") });
        halo.Children.Add(new Microsoft.UI.Xaml.Shapes.Ellipse { Width = 6, Height = 6, Fill = design.Brush("clay") });
        var animation = new DoubleAnimation { From = 1, To = .4, Duration = new Duration(TimeSpan.FromSeconds(.7)), AutoReverse = true,
            RepeatBehavior = RepeatBehavior.Forever, EnableDependentAnimation = true };
        Storyboard.SetTarget(animation, halo); Storyboard.SetTargetProperty(animation, "Opacity");
        var storyboard = new Storyboard(); storyboard.Children.Add(animation);
        halo.Loaded += (_, _) => { if (new UISettings().AnimationsEnabled) storyboard.Begin(); };
        halo.Unloaded += (_, _) => storyboard.Stop(); return halo;
    }
    // The same compose_icon loader and 1.2-second rotation as the skill pages.
    // Stop the clock when its owning view is removed; respect reduced motion.
    public static Image Spinner(WispDesign design, int size) => Rotate(design.Icon("loader", size, design.ColorText("clay")), 1.2);
    public static Image Activity(WispDesign design, int size) => Rotate(design.Icon("activity-orbit", size, design.ColorText("clay-strong")), 2.4);
    private static Image Rotate(Image icon, double seconds)
    {
        var rotation = new RotateTransform(); icon.RenderTransform = rotation; icon.RenderTransformOrigin = new Point(.5, .5);
        var animation = new DoubleAnimation { From = 0, To = 360, Duration = new Duration(TimeSpan.FromSeconds(seconds)),
            RepeatBehavior = RepeatBehavior.Forever, EnableDependentAnimation = true };
        Storyboard.SetTarget(animation, rotation); Storyboard.SetTargetProperty(animation, "Angle");
        var storyboard = new Storyboard(); storyboard.Children.Add(animation);
        icon.Loaded += (_, _) => { if (new UISettings().AnimationsEnabled) storyboard.Begin(); };
        icon.Unloaded += (_, _) => storyboard.Stop();
        return icon;
    }
}
