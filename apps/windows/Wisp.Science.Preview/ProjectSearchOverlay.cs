using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>Compact native command palette; the window owns Escape and modal focus.</summary>
internal sealed class ProjectSearchOverlay : Grid
{
    public ProjectSearchOverlay(ProjectBrowserModel model, WispDesign design, Action close,
        Action<SearchResult> open, Action<Microsoft.UI.Xaml.Controls.Primitives.FlyoutBase> register)
    {
        Background = new SolidColorBrush(Windows.UI.Color.FromArgb(70, 0, 0, 0));
        var card = new Border { MaxWidth = 560, MaxHeight = 410, Margin = new Thickness(24), Padding = new Thickness(16),
            HorizontalAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Center,
            Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(12) };
        var content = new Grid { RowSpacing = 12 };
        content.TabFocusNavigation = KeyboardNavigationMode.Cycle;
        content.RowDefinitions.Add(new() { Height = GridLength.Auto });
        content.RowDefinitions.Add(new() { Height = GridLength.Auto });
        content.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        content.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var inputRow = new Grid { ColumnSpacing = 10 };
        inputRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        inputRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        inputRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        inputRow.Children.Add(design.Icon("search"));
        var query = new TextBox { PlaceholderText = model.ActiveProjectId == null ? "搜索项目或最近会话" : "搜索当前项目的会话", FontSize = 15,
            BorderThickness = new Thickness(0), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), Padding = new Thickness(4, 8, 4, 8) };
        query.Resources["TextControlBorderBrushFocused"] = design.Brush("clay");
        query.Resources["TextControlBackgroundFocused"] = design.Brush("bg-elev");
        query.Resources["TextControlBackgroundPointerOver"] = design.Brush("bg-elev");
        AutomationProperties.SetName(query, "搜索关键词");
        var textMenu = new TextCommandBarFlyout(); query.ContextFlyout = textMenu; register(textMenu);
        Grid.SetColumn(query, 1); inputRow.Children.Add(query);
        var dismiss = new Button { Content = design.Icon("close", 16), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0), Padding = new Thickness(6), VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetName(dismiss, "关闭搜索"); ToolTipService.SetToolTip(dismiss, "关闭 · Esc");
        dismiss.Click += (_, _) => close(); Grid.SetColumn(dismiss, 2); inputRow.Children.Add(dismiss);
        content.Children.Add(inputRow);
        var scope = new TextBlock { Text = model.ActiveProjectId == null ? "项目与最近会话" : model.Projects.FirstOrDefault(p => p.Id == model.ActiveProjectId)?.Name,
            FontSize = 11, Foreground = design.Brush("text-faint"), Margin = new Thickness(4, 0, 4, 0) };
        Grid.SetRow(scope, 1); content.Children.Add(scope);
        var results = new ListView { IsItemClickEnabled = true, SelectionMode = ListViewSelectionMode.Single, BorderThickness = new Thickness(0) };
        results.Resources["ListViewItemSelectionIndicatorBrush"] = design.Brush("clay");
        results.Resources["ListViewItemBackgroundSelected"] = design.Brush("bg-sunken");
        AutomationProperties.SetName(results, "搜索结果");
        var empty = new TextBlock { Text = "没有匹配的项目或会话", Margin = new Thickness(8, 20, 8, 20), FontSize = 13, Foreground = design.Brush("text-faint") };
        Grid.SetRow(results, 2); Grid.SetRow(empty, 2); content.Children.Add(results); content.Children.Add(empty);
        var hint = new TextBlock { Text = "↑↓ 选择    Enter 打开    Esc 关闭", FontSize = 11, Foreground = design.Brush("text-faint"), Margin = new Thickness(4, 4, 4, 0) };
        Grid.SetRow(hint, 3); content.Children.Add(hint); card.Child = content; Children.Add(card);
        void Update()
        {
            results.Items.Clear();
            foreach (var result in model.Search(query.Text))
            {
                var row = new Grid { ColumnSpacing = 10 };
                row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
                row.Children.Add(design.Icon(result.SessionId == null ? "folder" : "chat", 16));
                var label = new StackPanel { Spacing = 3 };
                label.Children.Add(new TextBlock { Text = result.Title, FontSize = 13, Foreground = design.Brush("text"), TextTrimming = TextTrimming.CharacterEllipsis });
                // Inside a project the scope is already in the header; don't repeat it on every result.
                if (model.ActiveProjectId == null) label.Children.Add(new TextBlock { Text = result.Detail, FontSize = 11,
                    Foreground = design.Brush("text-faint"), TextTrimming = TextTrimming.CharacterEllipsis });
                Grid.SetColumn(label, 1); row.Children.Add(label);
                var item = new ListViewItem { Content = row, Tag = result, MinHeight = 36, Padding = new Thickness(10, 7, 10, 7), HorizontalContentAlignment = HorizontalAlignment.Stretch };
                AutomationProperties.SetName(item, result.Title); results.Items.Add(item);
            }
            results.SelectedIndex = results.Items.Count > 0 ? 0 : -1;
            empty.Visibility = results.Items.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            design.ApplyTypography(this);
        }
        void Accept(ListViewItem? item) { if (item?.Tag is SearchResult result) { close(); open(result); } }
        var composing = false;
        query.TextCompositionStarted += (_, _) => composing = true;
        query.TextCompositionEnded += (_, _) => composing = false;
        query.TextChanged += (_, _) => Update();
        query.KeyDown += (_, e) =>
        {
            if (composing) return;
            if (e.Key is VirtualKey.Down or VirtualKey.Up)
            {
                if (results.Items.Count > 0) results.SelectedIndex = Math.Clamp(results.SelectedIndex + (e.Key == VirtualKey.Down ? 1 : -1), 0, results.Items.Count - 1);
                if (results.SelectedItem != null) results.ScrollIntoView(results.SelectedItem);
                e.Handled = true;
            }
            else if (e.Key == VirtualKey.Enter) { Accept(results.SelectedItem as ListViewItem); e.Handled = true; }
        };
        results.KeyDown += (_, e) => { if (e.Key == VirtualKey.Enter) { Accept(results.SelectedItem as ListViewItem); e.Handled = true; } };
        results.ItemClick += (_, e) => Accept(e.ClickedItem as ListViewItem);
        PointerPressed += (_, e) => { if (ReferenceEquals(e.OriginalSource, this)) { close(); e.Handled = true; } };
        Loaded += (_, _) => query.Focus(FocusState.Programmatic);
        Update();
    }
}
