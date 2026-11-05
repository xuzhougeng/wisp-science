using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeExplorationRow : Button
{
    public NativeExplorationRow(NativeExplorationSummary summary, WispDesign design, bool active, Action open, Microsoft.UI.Xaml.Controls.Primitives.FlyoutBase? menu = null, bool rail = false)
    {
        var content = new Grid { ColumnSpacing = 9 };
        content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.ColumnDefinitions.Add(new());
        var icon = design.Icon("flask", 15, design.ColorText(active ? "text" : "text-muted"));
        icon.VerticalAlignment = VerticalAlignment.Center; content.Children.Add(icon);
        var copy = new StackPanel { Spacing = 1 };
        var title = design.Text(summary.Exploration.Name, 13);
        title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis;
        title.Foreground = design.Brush(active ? "text" : "text-muted"); copy.Children.Add(title);
        var meta = design.T(summary.StatusKey) + " · " + design.T(summary.IsolationKey);
        var detail = design.Text(meta, 9.5); detail.Foreground = design.Brush("text-faint");
        detail.TextWrapping = TextWrapping.NoWrap; detail.TextTrimming = TextTrimming.CharacterEllipsis; copy.Children.Add(detail);
        Grid.SetColumn(copy, 1); content.Children.Add(copy); Content = content;
        HorizontalAlignment = HorizontalAlignment.Stretch; HorizontalContentAlignment = HorizontalAlignment.Stretch;
        design.QuietButton(this); MinWidth = MinHeight = 0; Padding = new(9, 6, 9, 6); BorderThickness = new(1);
        BorderBrush = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.Transparent);
        CornerRadius = new(6); Resources["ButtonBackgroundPointerOver"] = design.Brush(active ? "clay-soft" : "bg-elev");
        if (active) Background = design.Brush("clay-soft");
        if (rail)
        {
            content.Children.Remove(icon); Content = icon; Padding = new(4); MinHeight = 32;
            HorizontalContentAlignment = HorizontalAlignment.Center;
        }
        var name = summary.Exploration.Name + " · " + meta;
        AutomationProperties.SetName(this, name); ToolTipService.SetToolTip(this, name);
        AutomationProperties.SetAutomationId(this, "exploration-" + summary.Exploration.Id);
        Click += (_, _) => open();
        if (menu != null) { ContextFlyout = menu; Unloaded += (_, _) => menu.Hide(); }
    }
}

internal sealed class NativeExplorationBanner : UserControl
{
    public NativeExplorationBanner(NativeExplorationState state, WispDesign design, Action? review = null, Action? startAnother = null, bool canStartAnother = true)
    {
        var summary = state.Summary; var mainline = summary == null;
        var copy = new StackPanel { Spacing = 2 };
        var eyebrow = design.Text(design.T(mainline ? "exploration.mainline_label" : "exploration.banner_label"), 9.5);
        eyebrow.FontWeight = Microsoft.UI.Text.FontWeights.Bold; eyebrow.CharacterSpacing = 60;
        eyebrow.Foreground = design.Brush("clay-strong"); copy.Children.Add(eyebrow);
        var title = design.Text(mainline ? state.ActiveCount > 0
            ? design.T("exploration.mainline_count").Replace("{n}", state.ActiveCount.ToString())
            : design.T("exploration.mainline_frozen_placeholder") : summary!.Exploration.Name, 13.5);
        title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.TextWrapping = TextWrapping.NoWrap;
        title.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(title, title.Text); copy.Children.Add(title);
        var detail = design.Text(mainline ? design.T("exploration.mainline_warning")
            : design.T(summary!.StatusKey) + " · " + design.T(summary.IsolationKey) + " · " + design.T("exploration.external_warning_short"), 11);
        detail.Foreground = design.Brush("text-muted"); design.BindLineHeight(detail, 11, 1.4); copy.Children.Add(detail);
        var layout = new Grid { ColumnSpacing = 14 }; layout.ColumnDefinitions.Add(new()); layout.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        layout.RowDefinitions.Add(new()); layout.RowDefinitions.Add(new() { Height = GridLength.Auto }); layout.Children.Add(copy);
        if (!mainline && review != null || mainline && startAnother != null)
        {
            var actions = new NativeActionWrap { Gap = 5, RowGap = 5, VerticalAlignment = VerticalAlignment.Center };
            foreach (var key in mainline ? new[] { "exploration.start_another" } : new[] { "exploration.view_diff", "exploration.promote", "exploration.discard" })
            {
                var button = new Button { Content = design.Text(design.T(key), 11), IsEnabled = mainline ? canStartAnother : key != "exploration.promote" || summary!.Exploration.Status == "active" };
                button.Opacity = button.IsEnabled ? 1 : .5;
                design.ActionButton(button, key == "exploration.promote"); button.Padding = new(8, 5, 8, 5); button.MinWidth = button.MinHeight = 0;
                if (key == "exploration.promote")
                {
                    button.Resources["ButtonBackgroundDisabled"] = button.Resources["ButtonBorderBrushDisabled"] = design.Brush("clay");
                    button.Resources["ButtonForegroundDisabled"] = design.Brush("on-clay");
                    if (!button.IsEnabled) button.Opacity = .5;
                }
                ((TextBlock)button.Content).Foreground = design.Brush(key == "exploration.promote" ? "on-clay" : key == "exploration.discard" ? "err" : "text");
                button.Click += (_, _) => { if (mainline) startAnother!(); else review!(); }; actions.Children.Add(button);
            }
            layout.Children.Add(actions);
            void Resize() { var compact = ActualWidth < 680; layout.RowSpacing = compact ? 9 : 0; layout.ColumnDefinitions[1].Width = compact ? new(0) : GridLength.Auto;
                Grid.SetColumn(actions, compact ? 0 : 1); Grid.SetRow(actions, compact ? 1 : 0); actions.HorizontalAlignment = compact ? HorizontalAlignment.Left : HorizontalAlignment.Right; }
            SizeChanged += (_, _) => Resize(); Resize();
        }
        Content = new Border { Child = layout, Padding = new(14, 12, 14, 12), BorderThickness = new(1), CornerRadius = new(6),
            Background = design.Brush(mainline ? "bg-elev" : "clay-soft"), BorderBrush = design.Brush(mainline ? "border-strong" : "clay") };
        AutomationProperties.SetAutomationId(this, mainline ? "mainline-exploration-banner" : "exploration-banner");
    }
}
