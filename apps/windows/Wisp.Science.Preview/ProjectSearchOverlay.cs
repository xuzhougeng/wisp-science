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
        Action<string> command, bool hasProject, bool hasSession, bool commandsOnly = false, string? version = null,
        Func<string, string?>? projectDescription = null)
    {
        this.model = model;
        NativeDialogVisuals.Backdrop(this, 8, Windows.UI.Color.FromArgb(design.Dark ? (byte)173 : (byte)128, 16, 15, 14));
        AutomationProperties.SetAutomationId(this, commandsOnly ? "action-palette" : "workspace-search");
        AutomationProperties.SetName(this, design.T(commandsOnly ? "command.placeholder" : "command.search_ph"));
        var card = new Border {
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Top,
            Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(12),
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
        void Resize()
        {
            var layout = SearchPaletteLayout.ForViewport(ActualWidth, ActualHeight, commandsOnly);
            card.Width = layout.Width; card.MaxHeight = layout.MaxHeight; card.Margin = new Thickness(0, layout.Top, 0, 0);
        }
        SizeChanged += (_, _) => Resize();
        Loaded += (_, _) => Resize();
        var content = new Grid();
        content.TabFocusNavigation = KeyboardNavigationMode.Cycle;
        content.RowDefinitions.Add(new() { Height = GridLength.Auto });
        content.RowDefinitions.Add(new() { Height = GridLength.Auto });
        content.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        content.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var inputRow = new Grid { ColumnSpacing = 10, Padding = new(14, 5, 12, 5), BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) };
        inputRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        inputRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        inputRow.Children.Add(design.Icon("search", 16, design.ColorText("text-faint")));
        var query = new TextBox { PlaceholderText = design.T(commandsOnly ? "command.placeholder" : "command.search_ph"), FontSize = 15,
            BorderThickness = new Thickness(0), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent), Padding = new Thickness(4, 8, 4, 8) };
        query.Resources["TextControlBorderThemeThicknessFocused"] = new Thickness(0);
        query.Resources["TextControlBorderBrushFocused"] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        query.Resources["TextControlBackgroundFocused"] = design.Brush("bg-elev");
        query.Resources["TextControlBackgroundPointerOver"] = design.Brush("bg-elev");
        AutomationProperties.SetName(query, query.PlaceholderText);
        var textMenu = new TextCommandBarFlyout(); query.ContextFlyout = textMenu; register(textMenu);
        Grid.SetColumn(query, 1); inputRow.Children.Add(query);
        var top = new StackPanel(); top.Children.Add(inputRow);
        if (hasProject && !commandsOnly)
        {
            var label = design.T("chat_search.entry");
            var line = new Grid { ColumnSpacing = 8 };
            line.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); line.ColumnDefinitions.Add(new()); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            line.Children.Add(design.Icon("chat", 18)); var name = design.Text(label, 14); name.TextWrapping = TextWrapping.Wrap;
            Grid.SetColumn(name, 1); line.Children.Add(name); var keys = design.Text("Ctrl+Shift+F", 11, "text-muted");
            Grid.SetColumn(keys, 2); line.Children.Add(keys);
            var history = new Button { Content = line, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            design.QuietButton(history); history.Padding = new(16, 10, 16, 10); history.BorderThickness = new(0, 0, 0, 1); history.BorderBrush = design.Brush("border");
            AutomationProperties.SetName(history, label); AutomationProperties.SetAutomationId(history, "project-chat-search-entry");
            history.Click += (_, _) => { close(); command("search-project-chats"); }; top.Children.Add(history);
        }
        content.Children.Add(top);
        var scope = new TextBlock { TextWrapping = TextWrapping.Wrap,
            FontSize = 12, Foreground = design.Brush("text-muted"), Margin = new Thickness(16, 8, 16, 2) };
        void Status(string? message) { scope.Text = message ?? ""; scope.Visibility = string.IsNullOrEmpty(message) ? Visibility.Collapsed : Visibility.Visible; }
        AutomationProperties.SetLiveSetting(scope, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        Grid.SetRow(scope, 1); content.Children.Add(scope);
        var results = new ListView { IsItemClickEnabled = true, SelectionMode = ListViewSelectionMode.Single, BorderThickness = new Thickness(0), Padding = new(8, 4, 8, 8) };
        var clear = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        results.Resources["ListViewItemSelectionIndicatorBrush"] = clear;
        results.Resources["ListViewItemBackgroundPointerOver"] = clear;
        foreach (var state in new[] { "Selected", "SelectedPointerOver", "SelectedPressed" })
            results.Resources["ListViewItemBackground" + state] = design.Brush("clay-soft");
        AutomationProperties.SetName(results, design.L("搜索结果", "Search results"));
        var empty = new TextBlock { Text = commandsOnly ? design.T("command.no_results") : design.L("没有匹配的结果", "No matching results"), TextWrapping = TextWrapping.Wrap,
            Margin = new Thickness(16, 20, 16, 20), FontSize = 13, Foreground = design.Brush("text-faint") };
        Grid.SetRow(results, 2); Grid.SetRow(empty, 2); content.Children.Add(results); content.Children.Add(empty);
        Border Keycap(string key, bool pill = false) => new() { Child = design.Text(key, 11, "text-muted"),
            Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1),
            CornerRadius = new(pill ? 10 : 6), Padding = new(5, 1, 5, 1), VerticalAlignment = VerticalAlignment.Center };
        var hints = new NativeActionWrap { Gap = 10, RowGap = 4, VerticallyCentered = true };
        void Hint(string key, string meaning)
        {
            var pair = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
            pair.Children.Add(Keycap(key, true)); pair.Children.Add(design.Text(design.T("command.hint." + meaning), 12, "text-muted")); hints.Children.Add(pair);
        }
        Hint("↑↓", "navigate"); Hint("↵", commandsOnly ? "run" : "open"); if (!commandsOnly) Hint("⇧↵", "attach"); Hint("esc", "close");
        var footerRow = new Grid { ColumnSpacing = 8 };
        footerRow.ColumnDefinitions.Add(new()); footerRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); footerRow.Children.Add(hints);
        if (!string.IsNullOrWhiteSpace(version))
        {
            var stamp = design.Text("v" + version.TrimStart('v'), 12, "text-faint");
            stamp.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(stamp, 1); footerRow.Children.Add(stamp);
        }
        var sunken = design.Brush("bg-sunken").Color; var elevated = design.Brush("bg-elev").Color;
        byte Blend(byte a, byte b) => (byte)Math.Round(.72 * a + .28 * b);
        var footColor = Windows.UI.Color.FromArgb(255, Blend(sunken.R, elevated.R), Blend(sunken.G, elevated.G), Blend(sunken.B, elevated.B));
        var foot = new Border { Padding = new(12, 8, 12, 8), Background = new SolidColorBrush(footColor), BorderBrush = design.Brush("border"), BorderThickness = new(0, 1, 0, 0),
            Child = footerRow };
        var gradient = new LinearGradientBrush { StartPoint = new(0, 0), EndPoint = new(0, 1) };
        gradient.GradientStops.Add(new() { Offset = 0, Color = Windows.UI.Color.FromArgb(0, footColor.R, footColor.G, footColor.B) });
        gradient.GradientStops.Add(new() { Offset = 1, Color = footColor });
        var fade = new Border { Height = 18, VerticalAlignment = VerticalAlignment.Bottom, Background = gradient, IsHitTestVisible = false, Visibility = Visibility.Collapsed };
        Grid.SetRow(fade, 2); content.Children.Add(fade);
        results.Loaded += (_, _) =>
        {
            results.ApplyTemplate();
            if (FindListScroll(results) is not { } scroll) return;
            void RefreshFade() => fade.Visibility = SearchPaletteLayout.ListOverflows(scroll.ScrollableHeight) ? Visibility.Visible : Visibility.Collapsed;
            scroll.RegisterPropertyChangedCallback(ScrollViewer.ScrollableHeightProperty, (_, _) => RefreshFade());
            RefreshFade();
        };
        Grid.SetRow(foot, 3); content.Children.Add(foot); card.Child = content; Children.Add(card);
        var choices = new List<ListViewItem>();
        var highlight = new Dictionary<ListViewItem, Action<bool>>();
        results.SelectionChanged += (_, e) =>
        {
            foreach (var item in e.RemovedItems.OfType<ListViewItem>()) if (highlight.TryGetValue(item, out var paint)) paint(false);
            foreach (var item in e.AddedItems.OfType<ListViewItem>()) if (highlight.TryGetValue(item, out var paint)) paint(true);
        };
        void Heading(string key)
        {
            var title = new TextBlock { Text = design.T(key), FontSize = 11, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Foreground = design.Brush("text-faint") };
            AutomationProperties.SetHeadingLevel(title, Microsoft.UI.Xaml.Automation.Peers.AutomationHeadingLevel.Level2);
            var padding = new Thickness(8, results.Items.Count == 0 ? 6 : 10, 8, 4);
            results.Items.Add(new ListViewItem { Content = title, Padding = padding, MinHeight = 0, IsTabStop = false, IsEnabled = false, IsHitTestVisible = false });
        }
        void AddChoice(object tag, string title, string icon, string? detail, string? shortcut, bool selectedOnly)
        {
            var row = new Grid { ColumnSpacing = 10 };
            row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var glyph = design.Icon(icon, 16, design.ColorText("text-muted"));
            var regular = glyph.Source; var active = design.Icon(icon, 16, design.ColorText("clay-strong")).Source;
            row.Children.Add(glyph);
            var label = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
            var name = design.Text(title, 14); name.TextTrimming = TextTrimming.CharacterEllipsis; name.TextWrapping = TextWrapping.NoWrap; label.Children.Add(name);
            if (!string.IsNullOrWhiteSpace(detail))
            {
                var sub = design.Text(detail, 12, "text-muted"); sub.TextTrimming = TextTrimming.CharacterEllipsis; sub.TextWrapping = TextWrapping.NoWrap;
                label.Children.Add(sub);
            }
            Grid.SetColumn(label, 1); row.Children.Add(label);
            var keys = shortcut == null ? null : Keycap(shortcut);
            if (keys != null) { Grid.SetColumn(keys, 2); row.Children.Add(keys); keys.Opacity = selectedOnly ? 0 : 1; }
            var item = new ListViewItem { Tag = tag, Content = row, MinHeight = commandsOnly ? 32 : 40,
                Padding = commandsOnly ? new(8, 4, 8, 4) : new(10, 6, 10, 6), HorizontalContentAlignment = HorizontalAlignment.Stretch };
            highlight[item] = selected =>
            {
                name.FontWeight = selected ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal;
                glyph.Source = selected ? active : regular;
                if (keys != null && selectedOnly) keys.Opacity = selected ? 1 : 0;
            };
            item.PointerMoved += (_, e) => { if (e.Pointer.PointerDeviceType == Microsoft.UI.Input.PointerDeviceType.Mouse) results.SelectedItem = item; };
            AutomationProperties.SetName(item, title); choices.Add(item); results.Items.Add(item);
        }
        void Update()
        {
            results.Items.Clear(); choices.Clear(); highlight.Clear();
            Status(model.Error ?? (model.Busy ? design.L("正在搜索…", "Searching…") : null));
            foreach (var group in model.Items.GroupBy(r => r.Kind).OrderBy(g => g.Key switch { "project" => 0, "artifact" => 1, _ => 2 }))
            {
                Heading(group.Key switch { "project" => "command.section.projects", "artifact" => "command.section.files", _ => "command.section.sessions" });
                foreach (var result in group)
                {
                    var detail = result.Kind == "project" ? projectDescription?.Invoke(result.Id) : result.ProjectName;
                    AddChoice(result, result.Title, result.Kind switch { "project" => "folder", "artifact" => "doc", _ => "bubble" }, detail,
                        result.Kind is "project" or "session" ? "Ctrl↵ " + design.T("command.hint.open_new_window") : null, true);
                }
            }
            var actions = NativePaletteActions.Match(query.Text, hasProject || !commandsOnly && model.CommandProjectId != null, hasSession, commandsOnly, design.Localization);
            string? lastGroup = null;
            foreach (var action in actions)
            {
                var group = commandsOnly ? "command.group." + action.Group : "command.section.commands";
                if (lastGroup != group) { Heading(group); lastGroup = group; }
                AddChoice(action, action.Label(design.Localization), action.Icon, null, commandsOnly
                    ? action.Id switch { "new" => "Ctrl+N", "search" => "Ctrl+K", "settings" => "Ctrl+,", "privacy-mode" => "Ctrl+Shift+H", "toggle-sidebar" => "Ctrl+B", _ => null } : null, false);
            }
            results.SelectedItem = choices.FirstOrDefault();
            empty.Visibility = choices.Count == 0 && !model.Busy && model.Error == null ? Visibility.Visible : Visibility.Collapsed;
            design.ApplyTypography(this);
        }
        static bool Down(VirtualKey key) => (Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(key) & Windows.UI.Core.CoreVirtualKeyStates.Down) != 0;
        void Accept(ListViewItem? item, bool keyboard)
        {
            if (keyboard && !commandsOnly && Down(VirtualKey.Shift))
            {
                var reference = SearchPaletteLayout.Attachment(item?.Tag as NativeSearchItem, model.Items);
                if (reference == null) return;
                if (!canAttach) { Status(design.L("请先打开可编辑会话。", "Open an editable conversation to attach a reference.")); return; }
                close(); open(reference, true, false); return;
            }
            if (item?.Tag is NativePaletteAction action)
            { close(); command(action.Id); return; }
            if (item?.Tag is not NativeSearchItem result) return;
            close(); open(result, false, keyboard && Down(VirtualKey.Control) && result.Kind is ("project" or "session"));
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
                if (choices.Count > 0)
                {
                    var current = results.SelectedItem is ListViewItem selected ? choices.IndexOf(selected) : -1;
                    results.SelectedItem = choices[SearchPaletteLayout.StepChoice(current, choices.Count, e.Key == VirtualKey.Down)];
                }
                if (results.SelectedItem != null) results.ScrollIntoView(results.SelectedItem);
                e.Handled = true;
            }
            else if (e.Key == VirtualKey.Enter)
            {
                e.Handled = true;
                Accept(results.SelectedItem as ListViewItem, true);
            }
        };
        // ListViewItem consumes Enter before bubbling, and ItemClick supplies
        // its content when an explicit container was added to Items.
        results.PreviewKeyDown += (_, e) => { if (e.Key == VirtualKey.Enter) { Accept(results.SelectedItem as ListViewItem, true); e.Handled = true; } };
        results.ItemClick += (_, e) => Accept(results.Items.OfType<ListViewItem>()
            .FirstOrDefault(item => ReferenceEquals(item, e.ClickedItem) || ReferenceEquals(item.Content, e.ClickedItem)), false);
        PointerPressed += (_, e) => { if (ReferenceEquals(e.OriginalSource, this)) { close(); e.Handled = true; } };
        Loaded += (_, _) => query.Focus(FocusState.Programmatic);
        update = Update;
        model.Changed += update;
        Update(); if (!commandsOnly) _ = model.SearchAsync("");
    }
    public void Dispose() { model.Changed -= update; model.Dispose(); }
    private static ScrollViewer? FindListScroll(DependencyObject parent)
    {
        if (parent is ScrollViewer scroll) return scroll;
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(parent); i++)
            if (FindListScroll(VisualTreeHelper.GetChild(parent, i)) is { } child) return child;
        return null;
    }
}
