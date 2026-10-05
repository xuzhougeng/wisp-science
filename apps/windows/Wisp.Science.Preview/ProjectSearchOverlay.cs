using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>Compact native command palette; the window owns Escape and modal focus.</summary>
internal sealed class ProjectSearchOverlay : Grid, IDisposable
{
    private readonly WorkspaceSearchModel model;
    private readonly Action update;
    public ProjectSearchOverlay(WorkspaceSearchModel model, WispDesign design, Action close,
        Action<NativeSearchItem, bool, bool> open, bool canAttach, Action<Microsoft.UI.Xaml.Controls.Primitives.FlyoutBase> register,
        Action<string> command, bool hasProject, bool hasSession, bool commandsOnly = false)
    {
        this.model = model;
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
        var query = new TextBox { PlaceholderText = commandsOnly ? "搜索命令" : "搜索项目、产物、会话标题或消息正文", FontSize = 15,
            BorderThickness = new Thickness(0), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), Padding = new Thickness(4, 8, 4, 8) };
        query.Resources["TextControlBorderBrushFocused"] = design.Brush("clay");
        query.Resources["TextControlBackgroundFocused"] = design.Brush("bg-elev");
        query.Resources["TextControlBackgroundPointerOver"] = design.Brush("bg-elev");
        AutomationProperties.SetName(query, commandsOnly ? "命令关键词" : "搜索关键词");
        var textMenu = new TextCommandBarFlyout(); query.ContextFlyout = textMenu; register(textMenu);
        Grid.SetColumn(query, 1); inputRow.Children.Add(query);
        var dismiss = new Button { Content = design.Icon("close", 16), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
            BorderThickness = new Thickness(0), Padding = new Thickness(6), VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetName(dismiss, "关闭搜索"); ToolTipService.SetToolTip(dismiss, "关闭 · Esc");
        dismiss.Click += (_, _) => close(); Grid.SetColumn(dismiss, 2); inputRow.Children.Add(dismiss);
        content.Children.Add(inputRow);
        var scope = new TextBlock { Text = "跨项目搜索 · 当前项目优先", TextWrapping = TextWrapping.Wrap,
            FontSize = 11, Foreground = design.Brush("text-faint"), Margin = new Thickness(4, 0, 4, 0) };
        AutomationProperties.SetLiveSetting(scope, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        Grid.SetRow(scope, 1); content.Children.Add(scope);
        var results = new ListView { IsItemClickEnabled = true, SelectionMode = ListViewSelectionMode.Single, BorderThickness = new Thickness(0) };
        results.Resources["ListViewItemSelectionIndicatorBrush"] = design.Brush("clay");
        results.Resources["ListViewItemBackgroundSelected"] = design.Brush("bg-sunken");
        AutomationProperties.SetName(results, "搜索结果");
        var empty = new TextBlock { Text = "没有匹配的项目、产物或会话", Margin = new Thickness(8, 20, 8, 20), FontSize = 13, Foreground = design.Brush("text-faint") };
        Grid.SetRow(results, 2); Grid.SetRow(empty, 2); content.Children.Add(results); content.Children.Add(empty);
        var hint = new TextBlock { Text = commandsOnly ? "↑↓ 选择    Enter 执行    Esc 关闭" : "↑↓ 选择    Enter 打开    Ctrl+Enter 新窗口    Shift+Enter 引用    Esc 关闭", TextWrapping = TextWrapping.Wrap, FontSize = 11, Foreground = design.Brush("text-faint"), Margin = new Thickness(4, 4, 4, 0) };
        Grid.SetRow(hint, 3); content.Children.Add(hint); card.Child = content; Children.Add(card);
        void Update()
        {
            results.Items.Clear();
            scope.Text = commandsOnly ? "命令面板 · Ctrl+Shift+P" : model.Error ?? (model.Busy ? "正在搜索…" : "跨项目搜索 · 当前项目优先");
            foreach (var result in model.Items)
            {
                var row = new Grid { ColumnSpacing = 10 };
                row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
                row.Children.Add(design.Icon(result.Kind switch { "project" => "folder", "artifact" => "doc", _ => "chat" }, 16));
                var label = new StackPanel { Spacing = 3 };
                label.Children.Add(new TextBlock { Text = result.Title, FontSize = 13, Foreground = design.Brush("text"), TextTrimming = TextTrimming.CharacterEllipsis });
                label.Children.Add(new TextBlock { Text = (result.Kind switch { "project" => "项目", "artifact" => "产物", _ => "会话" }) + " · " + result.Detail, FontSize = 11,
                    Foreground = design.Brush("text-faint"), TextTrimming = TextTrimming.CharacterEllipsis });
                Grid.SetColumn(label, 1); row.Children.Add(label);
                var item = new ListViewItem { Content = row, Tag = result, MinHeight = 36, Padding = new Thickness(10, 7, 10, 7), HorizontalContentAlignment = HorizontalAlignment.Stretch };
                AutomationProperties.SetName(item, result.Title); results.Items.Add(item);
            }
            foreach (var action in NativePaletteActions.Match(query.Text, hasProject, hasSession, commandsOnly))
            {
                var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
                row.Children.Add(design.Icon(action.Icon, 16));
                row.Children.Add(new TextBlock { Text = action.Title, FontSize = 13, Foreground = design.Brush("text") });
                var item = new ListViewItem { Content = row, Tag = action, MinHeight = 36, Padding = new Thickness(10, 7, 10, 7) };
                AutomationProperties.SetName(item, "命令 · " + action.Title); results.Items.Add(item);
            }
            empty.Text = commandsOnly ? "没有匹配的命令" : "没有匹配的项目、产物或会话";
            results.SelectedIndex = results.Items.Count > 0 ? 0 : -1;
            empty.Visibility = results.Items.Count == 0 && !model.Busy && model.Error == null ? Visibility.Visible : Visibility.Collapsed;
            design.ApplyTypography(this);
        }
        static bool Down(VirtualKey key) => (Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(key) & Windows.UI.Core.CoreVirtualKeyStates.Down) != 0;
        void Accept(ListViewItem? item)
        {
            var attach = Down(VirtualKey.Shift);
            var newWindow = !attach && Down(VirtualKey.Control);
            if (item?.Tag is NativePaletteAction action)
            { if (!attach) { close(); command(action.Id); } return; }
            if (item?.Tag is not NativeSearchItem result) return;
            if (newWindow && result.Kind is not ("project" or "session"))
            { scope.Text = "请选择项目或会话在新窗口打开。"; return; }
            if (attach && (!canAttach || result.Kind is not ("artifact" or "session")))
            { scope.Text = "请先打开可编辑会话，再选择会话或产物加入引用。"; return; }
            close(); open(result, attach, newWindow);
        }
        var composing = false;
        query.TextCompositionStarted += (_, _) => composing = true;
        query.TextCompositionEnded += (_, _) => composing = false;
        query.TextChanged += async (_, _) => { if (commandsOnly) Update(); else await model.SearchAsync(query.Text, TimeSpan.FromMilliseconds(180)); };
        query.PreviewKeyDown += (_, e) =>
        {
            if (composing) return;
            if (e.Key is VirtualKey.Down or VirtualKey.Up)
            {
                if (results.Items.Count > 0) results.SelectedIndex = Math.Clamp(results.SelectedIndex + (e.Key == VirtualKey.Down ? 1 : -1), 0, results.Items.Count - 1);
                if (results.SelectedItem != null) results.ScrollIntoView(results.SelectedItem);
                e.Handled = true;
            }
            else if (e.Key == VirtualKey.Enter)
            {
                e.Handled = true;
                Accept(results.SelectedItem as ListViewItem);
            }
        };
        // ListViewItem consumes Enter before bubbling, and ItemClick supplies
        // its content when an explicit container was added to Items.
        results.PreviewKeyDown += (_, e) => { if (e.Key == VirtualKey.Enter) { Accept(results.SelectedItem as ListViewItem); e.Handled = true; } };
        results.ItemClick += (_, e) => Accept(results.Items.OfType<ListViewItem>()
            .FirstOrDefault(item => ReferenceEquals(item, e.ClickedItem) || ReferenceEquals(item.Content, e.ClickedItem)));
        PointerPressed += (_, e) => { if (ReferenceEquals(e.OriginalSource, this)) { close(); e.Handled = true; } };
        Loaded += (_, _) => query.Focus(FocusState.Programmatic);
        update = Update;
        model.Changed += update;
        Update(); if (!commandsOnly) _ = model.SearchAsync("");
    }
    public void Dispose() { model.Changed -= update; model.Dispose(); }
}
