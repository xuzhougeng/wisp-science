using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

// App-owned compact disclosure, without the boxed WinUI Expander chrome. Keep
// its body mounted when collapsed so polling does not destroy nested controls.
internal sealed class NativeTranscriptDisclosure : StackPanel
{
    public ToggleButton Button { get; } = new();
    private readonly Border body;
    private readonly NativeDisclosureState state = new();
    public bool IsExpanded { get => state.Expanded; set { state.Choice = value; Update(); } }
    public bool? Choice { get => state.Choice; set { state.Choice = value; Update(); } }
    public void SetAutomatic(bool value) { state.Automatic = value; Update(); }
    public event Action? ExpansionChanged;
    public NativeTranscriptDisclosure(WispDesign design, UIElement header, UIElement content, string name, bool expanded,
        bool process = false, bool hasBody = true, bool steps = false)
    {
        Button.Content = header; Button.HorizontalAlignment = HorizontalAlignment.Stretch; Button.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        Button.MinHeight = 0; Button.MinWidth = 0; Button.Padding = process ? new(0, 8, 0, 11) : steps ? new(13, 9, 13, 9) : new(8, 5, 8, 5);
        Button.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent); Button.BorderThickness = new(0); Button.CornerRadius = new(4);
        Button.Foreground = design.Brush("text"); Button.IsEnabled = hasBody; state.HasBody = hasBody;
        foreach (var suffix in new[] { "", "Checked", "Disabled" }) Button.Resources["ToggleButtonBackground" + suffix] = Button.Background;
        foreach (var suffix in new[] { "PointerOver", "CheckedPointerOver", "Pressed", "CheckedPressed" }) Button.Resources["ToggleButtonBackground" + suffix] = design.Brush("surface-hover");
        AutomationProperties.SetName(Button, name);
        body = new Border { Child = content, Padding = process ? new(14, 3, 0, 8) : steps ? new(6, 2, 6, 6) : new(30, 2, 8, 8) };
        Children.Add(Button); Children.Add(body);
        if (process) Children.Add(new Border { Height = 1, Background = design.Brush("border") });
        Button.Click += (_, _) => { state.Choice = Button.IsChecked == true; Update(); };
        SetAutomatic(expanded);
    }
    private void Update() { Button.IsChecked = state.Expanded; body.Visibility = state.Expanded ? Visibility.Visible : Visibility.Collapsed; ExpansionChanged?.Invoke(); }
}
