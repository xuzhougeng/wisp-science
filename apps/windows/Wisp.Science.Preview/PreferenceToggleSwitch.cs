using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Animation;
using Microsoft.UI.Xaml.Shapes;

namespace Wisp.Science.Preview;

// Keep WinUI's keyboard, drag and automation behavior, adapting only the
// geometry in the pinned Windows App SDK 1.6 ToggleSwitch template.
internal static class PreferenceToggleSwitch
{
    public static void Apply(ToggleSwitch control) => control.Loaded += (_, _) => ApplyGeometry(control);
    public static void Apply(ToggleSwitch control, WispDesign design)
    {
        // Same 58% accent blend as .toggle input:checked + .toggle-track.
        var background = design.Brush("bg-elev").Color; var accent = design.Brush("clay").Color;
        var back = background.A * .42; var front = accent.A * .58; var alpha = back + front;
        byte Channel(byte b, byte f) => alpha == 0 ? (byte)0 : (byte)Math.Round((b * back + f * front) / alpha);
        var track = new SolidColorBrush(Windows.UI.Color.FromArgb((byte)Math.Round(alpha),
            Channel(background.R, accent.R), Channel(background.G, accent.G), Channel(background.B, accent.B)));
        foreach (var state in new[] { "", "PointerOver", "Pressed" })
        {
            control.Resources["ToggleSwitchFillOn" + state] = control.Resources["ToggleSwitchStrokeOn" + state] = track;
            control.Resources["ToggleSwitchFillOff" + state] = control.Resources["ToggleSwitchStrokeOff" + state] = design.Brush("border-strong");
            control.Resources["ToggleSwitchKnobFillOn" + state] = control.Resources["ToggleSwitchKnobFillOff" + state] = design.Brush("on-clay");
        }
        control.Resources["ToggleSwitchPreContentMargin"] = 0d;
        control.Resources["ToggleSwitchPostContentMargin"] = 0d;
        Apply(control);
    }
    private static void ApplyGeometry(ToggleSwitch control)
    {
        control.ApplyTemplate();
        if (VisualTreeHelper.GetChildrenCount(control) == 0
            || VisualTreeHelper.GetChild(control, 0) is not FrameworkElement root
            || Find(root, "OuterBorder") is not Rectangle outer
            || Find(root, "SwitchKnobBounds") is not Rectangle bounds
            || Find(root, "SwitchKnobOn") is not Border on
            || Find(root, "SwitchKnobOff") is not Rectangle off
            || Find(root, "SwitchAreaGrid") is not Grid area
            || area.Parent is not Grid layout || layout.ColumnDefinitions.Count < 2) return;

        var onState = VisualStateManager.GetVisualStateGroups(root)
            .SelectMany(group => group.States).FirstOrDefault(state => state.Name == "On");
        var movement = onState?.Storyboard?.Children.OfType<DoubleAnimation>()
            .FirstOrDefault(animation => Storyboard.GetTargetName(animation) == "KnobTranslateTransform");
        if (movement == null) return;

        outer.Width = bounds.Width = 36;
        on.Width = on.Height = off.Width = off.Height = 16;
        on.CornerRadius = new CornerRadius(8); off.RadiusX = off.RadiusY = 8;
        on.Margin = off.Margin = new Thickness(0);
        // The thumb bounds remain 20px; the WebView knob moves by 16px.
        movement.To = 16;
        // Normal/hover/pressed states otherwise animate the native knob back
        // to 12/14/17px, overriding its local Width and Height.
        foreach (var state in VisualStateManager.GetVisualStateGroups(root).SelectMany(group => group.States))
            if (state.Storyboard is { } storyboard)
                foreach (var animation in storyboard.Children.OfType<DoubleAnimationUsingKeyFrames>())
                    if (Storyboard.GetTargetName(animation) is "SwitchKnobOn" or "SwitchKnobOff"
                        && Storyboard.GetTargetProperty(animation) is "Width" or "Height")
                        foreach (var keyframe in animation.KeyFrames) keyframe.Value = 16;
        layout.ColumnDefinitions[1].Width = new GridLength(0);
        layout.ColumnDefinitions[1].MaxWidth = 0;
        VisualStateManager.GoToState(control, "Dragging", false);
        VisualStateManager.GoToState(control, control.IsOn ? "On" : "Off", false);
        VisualStateManager.GoToState(control, control.IsEnabled ? "Normal" : "Disabled", false);
    }
    private static FrameworkElement? Find(DependencyObject parent, string name)
    {
        if (parent is FrameworkElement element && element.Name == name) return element;
        for (var index = 0; index < VisualTreeHelper.GetChildrenCount(parent); index++)
            if (Find(VisualTreeHelper.GetChild(parent, index), name) is { } match) return match;
        return null;
    }
}
