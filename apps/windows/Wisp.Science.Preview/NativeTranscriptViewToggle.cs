using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeTranscriptViewToggle : UserControl
{
    private readonly Border frame = new();
    private readonly ToggleButton full = new(), current = new();
    private readonly NativeModelContextModel model;
    private readonly WispDesign design;
    private readonly bool labels;
    public NativeTranscriptViewToggle(NativeModelContextModel model, WispDesign design, Action<bool> select, bool labels = false)
    {
        this.model = model; this.design = design; this.labels = labels;
        HorizontalAlignment = HorizontalAlignment.Left;
        var row = new Grid { ColumnSpacing = 2 };
        row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new());
        row.Children.Add(full); Grid.SetColumn(current, 1); row.Children.Add(current); frame.Child = row; Content = frame;
        full.Click += (_, _) => { select(false); Update(); }; current.Click += (_, _) => { select(true); Update(); };
        Loaded += (_, _) => { model.Changed += Update; Update(); }; Unloaded += (_, _) => model.Changed -= Update;
        Update();
    }
    private void Update()
    {
        frame.Padding = new(labels ? 3 : 0); frame.CornerRadius = new(labels ? 18 : 6);
        frame.Background = design.Brush(labels ? "bg-sunken" : "bg-elev"); frame.BorderThickness = new(labels ? 1 : 0); frame.BorderBrush = design.Brush("border");
        foreach (var (button, active, key, icon, id) in new[] {
            (full, !model.Active, "chat.view_full", "history", "transcript-view-full"),
            (current, model.Active, "chat.view_model", "eye", "transcript-view-model") })
        {
            button.IsEnabled = button == full || model.Available;
            button.MinWidth = 30; button.MinHeight = 0; button.Height = labels ? 28 : 30;
            button.Padding = labels ? new(10, 0, 10, 0) : new(7, 0, 7, 0); button.CornerRadius = new(labels ? 16 : 6);
            button.BorderThickness = new(0); button.Background = design.Brush(active ? "clay-soft" : "bg-sunken");
            button.Foreground = design.Brush(active ? "clay-strong" : "text-muted");
            foreach (var suffix in new[] { "", "Checked" })
            {
                button.Resources["ToggleButtonBackground" + suffix] = button.Background;
                button.Resources["ToggleButtonForeground" + suffix] = button.Foreground;
            }
            foreach (var suffix in new[] { "PointerOver", "CheckedPointerOver", "Pressed", "CheckedPressed" }) button.Resources["ToggleButtonBackground" + suffix] = design.Brush("surface-hover");
            var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            content.Children.Add(design.Icon(icon, labels ? 14 : 16, design.ColorText(active ? "clay-strong" : "text-muted")));
            if (labels)
            {
                var label = design.Text(design.T(key), 12); label.Foreground = button.Foreground; content.Children.Add(label);
            }
            button.Content = content; button.IsChecked = active;
            AutomationProperties.SetName(button, design.T(key)); AutomationProperties.SetAutomationId(button, (labels ? "context-usage-" : "") + id);
            ToolTipService.SetToolTip(button, design.T(key));
        }
    }
}
