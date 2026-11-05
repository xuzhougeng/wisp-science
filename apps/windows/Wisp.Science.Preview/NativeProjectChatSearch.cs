using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeProjectChatSearch : UserControl, IWorkspaceSheet
{
    private readonly ProjectChatHistoryModel model;
    private readonly Action close, refresh;
    public bool IsModal => true;
    public NativeProjectChatSearch(ProjectChatHistoryModel model, string projectName, WispDesign design, Action close,
        Action<ProjectChatHit> open, Action<FlyoutBase> register)
    {
        this.model = model; this.close = close;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        var backdrop = new Grid();
        NativeDialogVisuals.Backdrop(backdrop, 8, design.Dark ? Windows.UI.Color.FromArgb(140, 16, 15, 14) : Windows.UI.Color.FromArgb(97, 255, 255, 255));
        var body = new Grid { TabFocusNavigation = KeyboardNavigationMode.Cycle };
        foreach (var height in new[] { GridLength.Auto, GridLength.Auto, GridLength.Auto, new GridLength(1, GridUnitType.Star), GridLength.Auto, GridLength.Auto })
            body.RowDefinitions.Add(new() { Height = height });
        var card = new Border { Child = body, VerticalAlignment = VerticalAlignment.Top, HorizontalAlignment = HorizontalAlignment.Center,
            CornerRadius = new(12), BorderThickness = new(1), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"),
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
        backdrop.Children.Add(card); Content = backdrop;
        AutomationProperties.SetName(this, design.T("chat_search.title")); AutomationProperties.SetAutomationId(this, "project-chat-history");
        backdrop.SizeChanged += (_, e) =>
        {
            var w = e.NewSize.Width; var h = e.NewSize.Height;
            var top = Math.Min(h / 2, h <= 700 ? 12 : Math.Min(72, .1 * h));
            card.Width = Math.Max(0, Math.Min(560, w <= 820 ? w - 24 : w * .92));
            card.MaxHeight = Math.Max(0, Math.Min(w <= 820 || h <= 700 ? h : 480, h - top - 16));
            card.Margin = new(0, top, 0, 0);
        };
        backdrop.PointerPressed += (_, e) => { if (ReferenceEquals(e.OriginalSource, backdrop)) { close(); e.Handled = true; } };
        var header = new Grid { Padding = new(16), ColumnSpacing = 12 };
        header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var heading = new StackPanel { Spacing = 4 };
        var title = design.Text(design.T("chat_search.title"), 16); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        heading.Children.Add(title); var project = design.Text(projectName, 14, "text-muted"); project.TextWrapping = TextWrapping.Wrap; heading.Children.Add(project);
        header.Children.Add(heading); var dismiss = design.ToolButton(design.T("chat_search.close"), "close"); dismiss.Click += (_, _) => close();
        Grid.SetColumn(dismiss, 1); header.Children.Add(dismiss); body.Children.Add(header);
        var input = new Grid { Padding = new(14, 4, 14, 4), ColumnSpacing = 10, BorderThickness = new(0, 0, 0, 1), BorderBrush = design.Brush("border") };
        input.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); input.ColumnDefinitions.Add(new());
        input.Children.Add(design.Icon("search", 18, design.ColorText("text-muted")));
        var clearBrush = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        var query = new TextBox { PlaceholderText = design.T("chat_search.keyword"), MaxLength = 256, IsSpellCheckEnabled = false,
            Padding = new(0, 6, 0, 6), BorderThickness = new(0), Background = clearBrush, Foreground = design.Brush("text") };
        query.Resources["TextControlBorderBrushFocused"] = clearBrush; query.Resources["TextControlBackgroundFocused"] = design.Brush("bg-elev");
        design.BindTypography(query, size: 15); AutomationProperties.SetName(query, query.PlaceholderText);
        var flyout = new TextCommandBarFlyout(); query.ContextFlyout = flyout; register(flyout);
        Grid.SetColumn(query, 1); input.Children.Add(query); Grid.SetRow(input, 1); body.Children.Add(input);
        var scope = design.Text(design.T("chat_search.scope"), 12, "text-muted"); scope.TextWrapping = TextWrapping.Wrap; scope.Margin = new(16, 10, 16, 10);
        Grid.SetRow(scope, 2); body.Children.Add(scope);
        var results = new ListView { IsItemClickEnabled = true, SelectionMode = ListViewSelectionMode.Single, Padding = new(8, 0, 8, 0),
            HorizontalContentAlignment = HorizontalAlignment.Stretch };
        results.Resources["ListViewItemSelectionIndicatorBrush"] = clearBrush; results.Resources["ListViewItemBackgroundSelected"] = design.Brush("clay-soft");
        AutomationProperties.SetName(results, design.T("chat_search.title")); Grid.SetRow(results, 3); body.Children.Add(results);
        var status = design.Text("", 12, "text-muted"); status.TextWrapping = TextWrapping.Wrap; status.Margin = new(16, 10, 16, 10);
        AutomationProperties.SetLiveSetting(status, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        Grid.SetRow(status, 4); body.Children.Add(status);
        var more = new Button { HorizontalAlignment = HorizontalAlignment.Center, Margin = new(16, 0, 16, 12) };
        more.Click += async (_, _) => await model.ContinueAsync(); Grid.SetRow(more, 5); body.Children.Add(more);
        void Update()
        {
            // Preserve keyboard selection and the visible page when appending more matches.
            if (results.Items.Count > model.Items.Count) results.Items.Clear();
            for (var i = results.Items.Count; i < model.Items.Count; i++)
            {
                var hit = model.Items[i];
                var row = new StackPanel { Spacing = 6 };
                var meta = design.Text("", 12, "text-muted"); meta.TextWrapping = TextWrapping.Wrap;
                meta.Inlines.Add(new Run { Text = hit.SessionTitle, FontWeight = Microsoft.UI.Text.FontWeights.SemiBold, Foreground = design.Brush("text") });
                var role = design.T("chat_search." + hit.Role);
                meta.Inlines.Add(new Run { Text = "   " + role + "   " + Timestamp(hit.Ts, design.Localization.Locale) });
                row.Children.Add(meta);
                var snippet = design.Text("", 14); snippet.TextWrapping = TextWrapping.Wrap; snippet.IsTextSelectionEnabled = false;
                snippet.Inlines.Add(new Run { Text = hit.Before });
                snippet.Inlines.Add(new Run { Text = hit.Matched, FontWeight = Microsoft.UI.Text.FontWeights.Bold, Foreground = design.Brush("clay-strong") });
                snippet.Inlines.Add(new Run { Text = hit.After });
                var highlight = new TextHighlighter { Background = design.Brush("clay-soft"), Foreground = design.Brush("clay-strong") };
                highlight.Ranges.Add(new TextRange { StartIndex = hit.Before.Length, Length = hit.Matched.Length }); snippet.TextHighlighters.Add(highlight);
                row.Children.Add(snippet);
                var item = new ListViewItem { Content = row, Tag = hit, Padding = new(10, 8, 10, 8), HorizontalContentAlignment = HorizontalAlignment.Stretch };
                item.PointerMoved += (_, e) =>
                {
                    if (e.Pointer.PointerDeviceType == Microsoft.UI.Input.PointerDeviceType.Mouse) results.SelectedItem = item;
                };
                AutomationProperties.SetName(item, $"{hit.SessionTitle}, {role}: {hit.Before}{hit.Matched}{hit.After}"); results.Items.Add(item);
            }
            if (results.Items.Count > 0 && results.SelectedIndex < 0) results.SelectedIndex = 0;
            status.Text = model.Error ?? (model.Loading ? design.T("chat_search.loading") : string.IsNullOrWhiteSpace(model.Query) ? design.T("chat_search.start")
                : model.Items.Count == 0 ? design.T("chat_search.empty") : design.Localization.Format("chat_search.count", ("n", model.Items.Count.ToString())));
            more.Content = design.T(model.Error != null ? "chat_search.retry" : "chat_search.more");
            more.IsEnabled = !model.Loading; more.Visibility = model.Next != null || model.Error != null ? Visibility.Visible : Visibility.Collapsed;
        }
        var composing = false;
        query.TextCompositionStarted += (_, _) => composing = true; query.TextCompositionEnded += (_, _) => composing = false;
        query.TextChanged += async (_, _) => await model.EditAsync(query.Text, TimeSpan.FromMilliseconds(250));
        query.PreviewKeyDown += (_, e) =>
        {
            if (composing || results.Items.Count == 0) return;
            if (e.Key is VirtualKey.Up or VirtualKey.Down)
            {
                results.SelectedIndex = SearchPaletteLayout.StepChoice(results.SelectedIndex, results.Items.Count, e.Key == VirtualKey.Down);
                results.ScrollIntoView(results.SelectedItem); e.Handled = true;
            }
            else if (e.Key == VirtualKey.Enter && results.SelectedItem is ListViewItem { Tag: ProjectChatHit hit }) { e.Handled = true; open(hit); }
        };
        results.ItemClick += (_, e) =>
        {
            if (results.Items.OfType<ListViewItem>().FirstOrDefault(i => ReferenceEquals(i, e.ClickedItem) || ReferenceEquals(i.Content, e.ClickedItem))?.Tag is ProjectChatHit hit) open(hit);
        };
        var focused = false;
        Loaded += (_, _) => { if (!focused) focused = query.Focus(FocusState.Programmatic); };
        refresh = Update; model.Changed += refresh; Update();
    }
    private static string Timestamp(long ts, string locale)
    {
        if (ts <= 0) return "";
        try { return (ts > 1_000_000_000_000 ? DateTimeOffset.FromUnixTimeMilliseconds(ts) : DateTimeOffset.FromUnixTimeSeconds(ts))
            .LocalDateTime.ToString("g", CultureInfo.GetCultureInfo(locale == "zh" ? "zh-CN" : "en-US")); }
        catch (ArgumentOutOfRangeException) { return ""; }
    }
    public void HandleEscape() => close();
    public void Dispose() { model.Changed -= refresh; model.Dispose(); }
}
