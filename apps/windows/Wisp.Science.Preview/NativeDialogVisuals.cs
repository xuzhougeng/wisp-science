using Microsoft.Graphics.Canvas.Effects;
using Microsoft.UI.Composition;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Hosting;
using Microsoft.UI.Xaml.Media;

namespace Wisp.Science.Preview;

internal static class NativeDialogVisuals
{
    public static void Backdrop(Grid overlay, WispDesign design, bool syncJoin = false)
    {
        var tint = design.Dark ? Windows.UI.Color.FromArgb(140, 17, 14, 15)
            : Windows.UI.Color.FromArgb(syncJoin ? (byte)87 : (byte)71, 27, 34, 45);
        Backdrop(overlay, syncJoin ? 5 : 3, tint);
    }

    public static void Backdrop(Grid overlay, float blur, Windows.UI.Color tint)
    {
        // Keep the blur below the tint and dialog, so text inside the dialog stays sharp.
        // XamlCompositionBrushBase releases GPU resources when the overlay leaves the tree.
        overlay.Background = new BackdropBrush(ElementCompositionPreview.GetElementVisual(overlay).Compositor, blur);
        overlay.Children.Insert(0, new Border { Background = new SolidColorBrush(tint), IsHitTestVisible = false });
    }

    public static Border Input(Control input, WispDesign design)
    {
        input.Resources["TextControlBorderThemeThicknessFocused"] = new Thickness(1);
        input.Resources["TextControlBorderBrushFocused"] = design.Brush("clay");
        input.Resources["TextControlBorderBrushPointerOver"] = design.Brush("border-strong");
        input.Resources["TextControlBackgroundFocused"] = design.Brush("bg-input");
        input.Resources["TextControlBackgroundPointerOver"] = design.Brush("bg-input");
        input.Resources["TextControlForegroundFocused"] = design.Brush("text");
        input.Resources["TextControlForegroundPointerOver"] = design.Brush("text");
        // Negative margin gives the CSS-style outer ring without changing field or row sizes.
        var ring = new Border { Child = input, BorderThickness = new Thickness(3), Margin = new Thickness(-3),
            CornerRadius = new CornerRadius(input.CornerRadius.TopLeft + 3) };
        void Refresh() => ring.BorderBrush = input.FocusState == FocusState.Unfocused ? null : design.Brush("focus-ring");
        input.GotFocus += (_, _) => Refresh();
        input.LostFocus += (_, _) => Refresh();
        input.Loaded += (_, _) => Refresh();
        return ring;
    }

    private sealed class BackdropBrush(Compositor compositor, float blur) : XamlCompositionBrushBase
    {
        private CompositionBackdropBrush? source;
        protected override void OnConnected()
        {
            if (CompositionBrush != null || !new CompositionCapabilities().AreEffectsSupported()) return;
            using var effect = new GaussianBlurEffect { BlurAmount = blur, BorderMode = EffectBorderMode.Hard,
                Source = new CompositionEffectSourceParameter("backdrop") };
            using var factory = compositor.CreateEffectFactory(effect);
            var brush = factory.CreateBrush();
            source = compositor.CreateBackdropBrush();
            brush.SetSourceParameter("backdrop", source);
            CompositionBrush = brush;
        }
        protected override void OnDisconnected()
        {
            CompositionBrush?.Dispose(); CompositionBrush = null;
            source?.Dispose(); source = null;
        }
    }
}
