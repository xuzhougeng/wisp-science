using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Windows.System;

namespace Wisp.Science.Preview;

// Keep the mounted text draft and validation/save lifecycle while adding
// browser-style numeric keys and repeat buttons inside the standard input.
internal static class PreferenceNumberBox
{
    public static void Apply(TextBox input, WispDesign design, int minimum, int maximum, Func<Task>? commitStep = null) =>
        _ = new Adapter(input, design, (text, direction) => NativePreferenceInteger.Step(text, direction, minimum, maximum), commitStep);

    public static void ApplyUnsigned(TextBox input, WispDesign design, ulong minimum, ulong maximum) =>
        _ = new Adapter(input, design, (text, direction) => NativePreferenceInteger.StepUnsigned(text, direction, minimum, maximum));

    private sealed class Adapter
    {
        private readonly TextBox input;
        private readonly WispDesign design;
        private readonly Func<string, int, string> step;
        private readonly Func<Task>? commitStep;
        private Grid? template, arrows;
        private RepeatButton? up, down;
        private bool hovering;

        public Adapter(TextBox input, WispDesign design, Func<string, int, string> step, Func<Task>? commitStep = null)
        {
            this.input = input; this.design = design; this.step = step; this.commitStep = commitStep;
            input.IsSpellCheckEnabled = input.IsTextPredictionEnabled = false;
            input.InputScope = new Microsoft.UI.Xaml.Input.InputScope { Names = { new() { NameValue = Microsoft.UI.Xaml.Input.InputScopeNameValue.Number } } };
            input.PreviewKeyDown += async (_, e) =>
            {
                if (e.Key is not (VirtualKey.Up or VirtualKey.Down) || !input.IsEnabled || input.IsReadOnly) return;
                e.Handled = true; await Step(e.Key == VirtualKey.Up ? 1 : -1);
            };
            input.PointerEntered += (_, _) => { hovering = true; Refresh(); };
            input.PointerExited += (_, _) => { hovering = false; Refresh(); };
            input.GotFocus += (_, _) => Refresh(); input.LostFocus += (_, _) => Refresh();
            input.IsEnabledChanged += (_, _) => Refresh(); input.TextChanged += (_, _) => Refresh();
            input.Loaded += (_, _) => Mount();
        }
        private void Mount()
        {
            input.ApplyTemplate();
            if (Find(input, "DeleteButton") is not Button clear || clear.Parent is not Grid root || root == template) return;
            template = root;
            clear.MinWidth = clear.MaxWidth = clear.Width = 0; clear.IsHitTestVisible = false;
            arrows = new Grid { Width = 18, Margin = new Thickness(0, 3, 5, 3), VerticalAlignment = VerticalAlignment.Center };
            arrows.RowDefinitions.Add(new() { Height = GridLength.Auto }); arrows.RowDefinitions.Add(new() { Height = GridLength.Auto });
            Grid.SetRow(arrows, Grid.GetRow(clear)); Grid.SetColumn(arrows, Grid.GetColumn(clear));
            RepeatButton Button(bool increase)
            {
                var button = new RepeatButton { Content = design.Icon(increase ? "chevron-up" : "chevron-down", 10),
                    Width = 18, Height = 13, MinWidth = 0, MinHeight = 0, Padding = new Thickness(0), BorderThickness = new Thickness(0),
                    IsTabStop = false, Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), Foreground = design.Brush("text-muted") };
                button.Resources["RepeatButtonBackgroundPointerOver"] = design.Brush("bg-sunken");
                button.Resources["RepeatButtonBackgroundPressed"] = design.Brush("clay-soft");
                AutomationProperties.SetName(button, design.L(increase ? "增加" : "减少", increase ? "Increase" : "Decrease") + " · " + AutomationProperties.GetName(input));
                button.Click += async (_, _) => { input.Focus(FocusState.Pointer); await Step(increase ? 1 : -1); };
                return button;
            }
            up = Button(true); down = Button(false); Grid.SetRow(down, 1); arrows.Children.Add(up); arrows.Children.Add(down); root.Children.Add(arrows);
            Refresh();
        }
        private async Task Step(int direction)
        {
            if (!input.IsEnabled || input.IsReadOnly) return;
            var next = step(input.Text, direction); if (next == input.Text) return;
            input.Text = next;
            input.Select(input.Text.Length, 0);
            if (commitStep != null) await commitStep();
        }
        private void Refresh()
        {
            if (arrows == null || up == null || down == null) return;
            var editable = input.IsEnabled && !input.IsReadOnly;
            arrows.Opacity = editable && (hovering || input.FocusState != FocusState.Unfocused) ? 1 : 0;
            up.IsEnabled = editable && step(input.Text, 1) != input.Text;
            down.IsEnabled = editable && step(input.Text, -1) != input.Text;
        }
        private static FrameworkElement? Find(DependencyObject parent, string name)
        {
            if (parent is FrameworkElement element && element.Name == name) return element;
            for (var n = 0; n < VisualTreeHelper.GetChildrenCount(parent); n++)
                if (Find(VisualTreeHelper.GetChild(parent, n), name) is { } found) return found;
            return null;
        }
    }
}
