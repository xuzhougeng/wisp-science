using System.Runtime.CompilerServices;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

// Adapt only the field's decoration. Native editing, selection, IME, automation,
// and keyboard handling stay with TextBox/PasswordBox/ComboBox.
internal static class PreferenceFieldAppearance
{
    private static readonly ConditionalWeakTable<Control, Adapter> adapters = new();

    // Chromium's unstyled input::placeholder is #757575 in both application themes.
    // Scope this to fields that have that style in WebView; keep the system's
    // disabled and high-contrast colors.
    public static void StylePlaceholder(TextBox field)
    {
        var accessibility = new Windows.UI.ViewManagement.AccessibilitySettings();
        var hint = new SolidColorBrush(Windows.UI.Color.FromArgb(255, 117, 117, 117));
        void Refresh()
        {
            if (field.IsEnabled && !accessibility.HighContrast) field.PlaceholderForeground = hint;
            else field.ClearValue(TextBox.PlaceholderForegroundProperty);
        }
        // Observe the XAML theme on this control. AccessibilitySettings' WinRT
        // change event requires a CoreWindow, which this unpackaged app lacks.
        field.Loaded += (_, _) => Refresh();
        field.ActualThemeChanged += (_, _) => Refresh();
        field.IsEnabledChanged += (_, _) => Refresh();
        Refresh();
    }

    public static void Apply(Control control, WispDesign design)
    {
        if (control is TextBox or PasswordBox or ComboBox) adapters.GetValue(control, item => new Adapter(item, design)).Refresh();
    }

    // A WebView text input does not reserve a native clear-button column when
    // typing. In the proxy row Clear is already a separate visible action.
    public static void UsePlainTextField(TextBox field)
    {
        StylePlaceholder(field);
        void Mount()
        {
            field.ApplyTemplate();
            if (Adapter.Find(field, "DeleteButton") is not Button clear) return;
            clear.MinWidth = clear.Width = clear.MaxWidth = 0;
            clear.IsHitTestVisible = false;
        }
        field.Loaded += (_, _) => Mount();
        if (field.IsLoaded) Mount();
    }

    public static void ApplyTree(DependencyObject? element, WispDesign design)
    {
        if (element is TextBox or PasswordBox or ComboBox) { Apply((Control)element, design); return; }
        // Walk authored content, not template parts or generated popup items.
        switch (element)
        {
            case Panel panel: foreach (var child in panel.Children) ApplyTree(child, design); break;
            case Border border: ApplyTree(border.Child, design); break;
            case ContentControl content: ApplyTree(content.Content as DependencyObject, design); break;
        }
    }

    private sealed class Adapter
    {
        private readonly Control control;
        private readonly WispDesign design;
        private readonly string prefix;
        private readonly SolidColorBrush outline = new();
        private Border? frame, ring;

        public Adapter(Control control, WispDesign design)
        {
            this.control = control; this.design = design;
            prefix = control is ComboBox ? "ComboBox" : "TextControl";
            control.Background = design.Brush("bg-input");
            control.Foreground = design.Brush("text");
            control.BorderBrush = outline;
            control.BorderThickness = new Thickness(1);
            foreach (var state in new[] { "", "PointerOver", "Pressed", "Focused", "Unfocused" })
            {
                control.Resources[prefix + "Background" + state] = design.Brush("bg-input");
                control.Resources[prefix + "Foreground" + state] = design.Brush("text");
            }
            control.Resources["TextControlBorderThemeThicknessFocused"] = new Thickness(1);
            control.Resources["TextControlBorderBrushFocused"] = design.Brush("clay");
            foreach (var state in new[] { "", "PointerOver", "Pressed" })
                control.Resources[prefix + "BorderBrush" + state] = outline;
            control.Loaded += (_, _) => Mount();
            control.GotFocus += (_, _) => Refresh(); control.LostFocus += (_, _) => Refresh();
            control.IsEnabledChanged += (_, _) => Refresh();
            if (control is ComboBox combo)
            {
                PreferencePickerAppearance.Apply(combo, design);
                combo.DropDownOpened += (_, _) => Refresh();
                combo.DropDownClosed += (_, _) => Refresh();
            }
            Refresh();
            if (control.IsLoaded) Mount();
        }

        private void Mount()
        {
            control.ApplyTemplate();
            var current = Find(control, control is ComboBox ? "Background" : "BorderElement") as Border;
            if (current == null || current.Parent is not Grid root) return;
            if (frame != current)
            {
                frame = current;
                ring = new Border { IsHitTestVisible = false, BorderThickness = new Thickness(3),
                    BorderBrush = design.Brush("focus-ring"), Margin = new Thickness(-3) };
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetAccessibilityView(ring,
                    Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
                Grid.SetRow(ring, Grid.GetRow(frame)); Grid.SetColumn(ring, Grid.GetColumn(frame));
                Grid.SetRowSpan(ring, Grid.GetRowSpan(frame)); Grid.SetColumnSpan(ring, Grid.GetColumnSpan(frame));
                root.Children.Insert(0, ring);
                if (control is ComboBox)
                {
                    // The default keyboard rectangle/pill is replaced by the same
                    // focus ring used for pointer focus, including an open popup.
                    if (Find(root, "HighlightBackground") is FrameworkElement highlight) highlight.Visibility = Visibility.Collapsed;
                    if (Find(root, "Pill") is FrameworkElement pill) pill.Visibility = Visibility.Collapsed;
                }
                control.UseSystemFocusVisuals = false;
            }
            Refresh();
        }

        public void Refresh()
        {
            var focused = NativeSettingsLayout.PreferenceFocusVisible(control.IsEnabled,
                control.FocusState != FocusState.Unfocused, control is ComboBox { IsDropDownOpen: true });
            // Updating ResourceDictionary during a native focus transition can
            // throw 0x800F0902. Keep resources mounted and mutate only their brush.
            outline.Color = design.Brush(focused ? "clay" : "border-strong").Color;
            // Do not overwrite the template's muted disabled-state decoration.
            if (frame != null && control.IsEnabled) frame.BorderBrush = outline;
            if (ring != null)
            {
                ring.Visibility = focused ? Visibility.Visible : Visibility.Collapsed;
                var radius = control.CornerRadius;
                ring.CornerRadius = new CornerRadius(radius.TopLeft + 3, radius.TopRight + 3,
                    radius.BottomRight + 3, radius.BottomLeft + 3);
            }
        }

        internal static DependencyObject? Find(DependencyObject parent, string name)
        {
            if (parent is FrameworkElement element && element.Name == name) return element;
            for (var i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
                if (Find(VisualTreeHelper.GetChild(parent, i), name) is { } found) return found;
            return null;
        }
    }
}
