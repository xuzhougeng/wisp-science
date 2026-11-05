using System.Text.Json.Nodes;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>The installed detail and package preview share the same source rows.</summary>
internal static class NativeSkillSourceView
{
    public static FrameworkElement Create(JsonObject source, WispDesign design)
    {
        var grid = new Grid { ColumnSpacing = 12, RowSpacing = 7 };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(130) });
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        foreach (var entry in NativeSkillSourcePresentation.Entries(source))
        {
            var row = grid.RowDefinitions.Count; grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var label = design.Text(entry.Label, 13); label.FontWeight = FontWeights.SemiBold; Grid.SetRow(label, row); grid.Children.Add(label);
            var text = design.Text(entry.Value, 13); text.IsTextSelectionEnabled = true;
            text.TextWrapping = TextWrapping.Wrap;
            if (entry.Code) text.FontFamily = design.Font(true);
            FrameworkElement value = text;
            if (entry.Url is { } url)
            {
                text.Foreground = design.Brush("clay"); text.TextDecorations = Windows.UI.Text.TextDecorations.Underline;
                // The link owns pointer selection; plain provenance values remain selectable.
                text.IsTextSelectionEnabled = false;
                var link = new HyperlinkButton { Content = text, NavigateUri = new Uri(url), Padding = new Thickness(0),
                    HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
                AutomationProperties.SetName(link, entry.Value); ToolTipService.SetToolTip(link, url); value = link;
            }
            Grid.SetColumn(value, 1); Grid.SetRow(value, row); grid.Children.Add(value);
        }
        grid.SizeChanged += (_, e) => grid.ColumnDefinitions[0].Width = new GridLength(e.NewSize.Width < 420 ? 80 : 130);
        return grid;
    }
}
