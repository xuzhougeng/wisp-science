using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private Button SessionToolbarButton(string title, string icon)
    {
        var button = design.ToolButton(title, icon);
        design.QuietButton(button); button.Width = button.Height = 26; button.MinWidth = button.MinHeight = 0;
        button.Padding = new Thickness(0); button.Content = design.Icon(icon, 16, design.ColorText("text-faint"));
        return button;
    }
    private Flyout SessionArrangementMenu(WorkspaceSessionGroups groups)
    {
        var menu = new Flyout { Placement = Microsoft.UI.Xaml.Controls.Primitives.FlyoutPlacementMode.BottomEdgeAlignedRight };
        var style = new Style(typeof(FlyoutPresenter));
        style.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(8)));
        style.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        style.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        style.Setters.Add(new Setter(Control.BorderThicknessProperty, new Thickness(1)));
        style.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(10)));
        menu.FlyoutPresenterStyle = style;
        var body = new StackPanel { MinWidth = 166 };
        foreach (var sort in new[] { false, true })
        {
            if (sort) body.Children.Add(new Border { Height = 1, Margin = new Thickness(4, 7, 4, 7), Background = design.Brush("border") });
            var title = Text(design.T(sort ? "sidebar.sort_by" : "sidebar.group_by"), 10.5, "text-faint"); title.Margin = new Thickness(6, 2, 6, 5); body.Children.Add(title);
            foreach (var value in sort ? new[] { "newest", "name" } : new[] { "none", "folder", "date" })
            {
                var selected = (sort ? groups.Sort : groups.Group) == value;
                var row = new Grid { ColumnSpacing = 8 };
                row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                var label = design.T("sidebar." + (sort ? "sort_" : "group_") + value);
                row.Children.Add(Text(label, 13, selected ? "clay" : "text"));
                if (selected) { var check = design.Icon("check", 15, design.ColorText("clay")); Grid.SetColumn(check, 1); row.Children.Add(check); }
                var choice = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
                design.QuietButton(choice); choice.Padding = new Thickness(8, 6, 8, 6); choice.MinWidth = choice.MinHeight = 0;
                Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(choice, label);
                choice.Click += (_, _) =>
                {
                    menu.Hide();
                    if (sort) settings.SessionSort = groups.Sort = value; else settings.SessionGroup = groups.Group = value;
                    SaveSettings(); Render();
                };
                body.Children.Add(choice);
            }
        }
        menu.Content = body; Register(menu); return menu;
    }
}
