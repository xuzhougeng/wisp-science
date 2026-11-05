using System.Globalization;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.Foundation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private NativeUsagePager? usagePager;
    private NativeUsageActivityMode usageMode;
    private double UsageViewport => XamlRoot?.Size.Width ?? ActualWidth;
    private void UsageChanged() { if (!model.Closed) Render(); }
    private void BackToUsage() { BodyScroll.ChangeView(null, 0, null, true); usagePager?.Back(); }
    private TextBlock UsageText(string text, double size = 12, string color = "text-faint", bool bold = false)
    {
        var block = Design.Text(text, size); block.Foreground = Design.Brush(color);
        if (bold) block.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        return block;
    }
    private static SolidColorBrush UsageColor(string color) => new(Windows.UI.Color.FromArgb(255,
        byte.Parse(color.AsSpan(1, 2), NumberStyles.HexNumber), byte.Parse(color.AsSpan(3, 2), NumberStyles.HexNumber), byte.Parse(color.AsSpan(5, 2), NumberStyles.HexNumber)));
    private void Usage()
    {
        Body.Padding = new Thickness(24, 18, 24, 28); Form.Spacing = 14;
        if (model.Values["get_token_usage"] is not JsonObject overview) { Form.Children.Add(ModelHint(T("settings.usage.loading"))); return; }
        var workspaces = NativeUsagePresentation.Workspaces(overview);
        if (workspaces.Length == 0) { Form.Children.Add(ModelHint(T("settings.usage.empty"))); return; }
        if (usagePager?.Selected is { } selected) { UsageDetail(selected); return; }
        var hint = ModelHint(T("settings.usage.hint")); hint.Margin = new Thickness(0, 12, 0, 12); Form.Children.Add(hint);
        Form.Children.Add(UsageSummary(NativeUsagePresentation.Totals(workspaces)));
        Form.Children.Add(UsageActivity(overview));
        var layout = new Grid { ColumnSpacing = 14, RowSpacing = 14, VerticalAlignment = VerticalAlignment.Top };
        layout.ColumnDefinitions.Add(new() { Width = new GridLength(.72, GridUnitType.Star) }); layout.ColumnDefinitions.Add(new() { Width = new GridLength(1.6, GridUnitType.Star) });
        layout.RowDefinitions.Add(new() { Height = GridLength.Auto }); layout.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var left = new StackPanel { Spacing = 14 }; left.Children.Add(UsageModels(overview)); left.Children.Add(UsageTools(overview)); layout.Children.Add(left);
        var right = UsageCard("workspaces", "workspaces_hint", out var content); content.Children.Add(UsageTable(workspaces)); layout.Children.Add(right);
        void Arrange()
        {
            var columns = NativeUsageLayout.OverviewColumns(UsageViewport, layout.ActualWidth);
            Grid.SetColumnSpan(left, columns ? 1 : 2); Grid.SetColumnSpan(right, columns ? 1 : 2); Grid.SetColumn(right, columns ? 1 : 0); Grid.SetRow(right, columns ? 0 : 1);
            layout.RowSpacing = columns ? 0 : 14;
        }
        layout.SizeChanged += (_, _) => Arrange(); layout.Loaded += (_, _) => Arrange(); Form.Children.Add(layout);
    }
    private Grid UsageSummary(NativeUsageTotals totals)
    {
        var grid = new Grid { ColumnSpacing = 10, RowSpacing = 10, Margin = new Thickness(0, 12, 0, 18) };
        for (var i = 0; i < 4; i++)
        {
            var text = new StackPanel { Spacing = 3 }; text.Children.Add(UsageText(NativeUsagePresentation.Tokens(totals.Values[i]), 20, "text", true)); text.Children.Add(UsageText(T("settings.usage." + NativeUsagePresentation.TotalKeys[i])));
            grid.Children.Add(new Border { Child = text, Padding = new Thickness(14, 12, 14, 12), Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10) });
        }
        void Arrange()
        {
            var columns = NativeUsageLayout.SummaryColumns(grid.ActualWidth); if (columns == grid.ColumnDefinitions.Count) return;
            grid.ColumnDefinitions.Clear(); grid.RowDefinitions.Clear();
            for (var i = 0; i < columns; i++) grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            for (var i = 0; i < (4 + columns - 1) / columns; i++) grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
            for (var i = 0; i < 4; i++) { var tile = (FrameworkElement)grid.Children[i]; Grid.SetRow(tile, i / columns); Grid.SetColumn(tile, i % columns); Grid.SetColumnSpan(tile, columns == 3 && i == 3 ? 3 : 1); }
        }
        grid.Loaded += (_, _) => Arrange(); grid.SizeChanged += (_, _) => Arrange(); return grid;
    }
    private Border UsageCard(string title, string hint, out StackPanel body, FrameworkElement? actions = null)
    {
        body = new StackPanel { Spacing = 14 };
        var head = new Grid { ColumnSpacing = 16, RowSpacing = 16 };
        head.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        head.RowDefinitions.Add(new() { Height = GridLength.Auto }); head.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var copy = new StackPanel { Spacing = 3 }; copy.Children.Add(UsageText(T("settings.usage." + title), 15, "text", true)); copy.Children.Add(UsageText(T("settings.usage." + hint), 11.5)); head.Children.Add(copy);
        if (actions != null)
        {
            actions.VerticalAlignment = VerticalAlignment.Top; head.Children.Add(actions);
            void Arrange()
            {
                var compact = NativeUsageLayout.Compact(UsageViewport); Grid.SetColumn(actions, compact ? 0 : 1); Grid.SetRow(actions, compact ? 1 : 0); Grid.SetColumnSpan(copy, compact ? 2 : 1); Grid.SetColumnSpan(actions, compact ? 2 : 1); head.RowSpacing = compact ? 16 : 0;
            }
            head.Loaded += (_, _) => Arrange(); head.SizeChanged += (_, _) => Arrange();
        }
        body.Children.Add(head);
        return new Border { Child = body, Padding = new Thickness(16), BorderThickness = new Thickness(1), BorderBrush = Design.Brush("border"), Background = Design.Brush("bg-elev"), CornerRadius = new CornerRadius(10), VerticalAlignment = VerticalAlignment.Top };
    }
    private Border UsageActivity(JsonNode overview)
    {
        var days = NativeUsagePresentation.Days(overview);
        var blocks = new List<Border>();
        var tabs = new List<(NativeUsageActivityMode Mode, ToggleButton Button)>();
        var modes = new StackPanel { Orientation = Orientation.Horizontal };
        foreach (var (mode, key) in new[] { (NativeUsageActivityMode.Daily, "daily"), (NativeUsageActivityMode.Weekly, "weekly"), (NativeUsageActivityMode.Cumulative, "cumulative") })
        {
            var tab = new ToggleButton { Content = UsageText(T("settings.usage." + key), 12), MinHeight = 28, MinWidth = 0, Padding = new Thickness(9, 4, 9, 4), CornerRadius = new CornerRadius(6), BorderThickness = new Thickness(0), FocusVisualPrimaryBrush = Design.Brush("clay") };
            AutomationProperties.SetName(tab, T("settings.usage." + key));
            tab.Click += (_, _) => { usageMode = mode; Update(); }; tabs.Add((mode, tab)); modes.Children.Add(tab);
        }
        var modeFrame = new Border { Child = modes, Padding = new Thickness(2), CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-sunken"), HorizontalAlignment = HorizontalAlignment.Left };
        var card = UsageCard("activity", "activity_hint", out var body, modeFrame); card.Margin = new Thickness(0, 0, 0, 14);
        var grid = new Grid { MinWidth = 620, ColumnSpacing = 3, RowSpacing = 3, Margin = new Thickness(0, 2, 0, 4) };
        for (var col = 0; col < Math.Max(53, (days.Length + 6) / 7); col++) grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star), MinWidth = 8 });
        grid.RowDefinitions.Add(new() { Height = new GridLength(18) }); for (var row = 0; row < 7; row++) grid.RowDefinitions.Add(new() { Height = new GridLength(10) });
        foreach (var (week, month) in NativeUsagePresentation.Months(days))
        {
            var label = UsageText(Design.Localization.Locale == "zh" ? month + "月" : CultureInfo.InvariantCulture.DateTimeFormat.GetAbbreviatedMonthName(month), 10); label.TextWrapping = TextWrapping.NoWrap;
            Grid.SetColumn(label, week); Grid.SetColumnSpan(label, Math.Min(4, grid.ColumnDefinitions.Count - week)); grid.Children.Add(label);
        }
        for (var i = 0; i < days.Length; i++)
        {
            var block = new Border { CornerRadius = new CornerRadius(3) }; blocks.Add(block);
            Grid.SetColumn(block, i / 7); Grid.SetRow(block, i % 7 + 1); grid.Children.Add(block);
        }
        // Retain the controls and scroll position when switching modes, including keyboard focus.
        void Update()
        {
            foreach (var (mode, tab) in tabs)
            {
                var active = usageMode == mode; tab.IsChecked = active;
                tab.Background = active ? Design.Brush("bg-elev") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
                ((TextBlock)tab.Content).Foreground = Design.Brush(active ? "text" : "text-faint");
                foreach (var state in new[] { "", "Checked", "CheckedPointerOver", "CheckedPressed", "PointerOver", "Pressed" }) tab.Resources["ToggleButtonBackground" + state] = tab.Background;
            }
            var cells = NativeUsagePresentation.Activity(days, usageMode);
            for (var i = 0; i < cells.Length; i++)
            {
                var cell = cells[i]; var blend = new[] { 0d, .25, .48, .72, 1 }[cell.Level];
                blocks[i].Background = new SolidColorBrush(Mix(Design.Brush("bg-sunken").Color, Design.Brush("clay").Color, blend)); blocks[i].Opacity = cell.Future ? .35 : 1;
                ToolTipService.SetToolTip(blocks[i], cell.Future ? null : Design.Localization.Format("settings.usage.activity_tooltip", ("period", cell.Period), ("tokens", NativeUsagePresentation.Tokens(cell.Tokens))));
            }
        }
        Update();
        var scroll = new ScrollViewer { Content = grid, HorizontalScrollMode = ScrollMode.Enabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        body.Children.Add(scroll); return card;
    }
    private Border UsageModels(JsonNode overview)
    {
        var card = UsageCard("model_share", "model_share_hint", out var body);
        var slices = NativeUsagePresentation.Models(overview, model.Values["list_models"], T("settings.usage.unknown_model"), T("settings.usage.other_models"));
        if (slices.Length == 0) { body.Children.Add(ModelHint(T("settings.usage.models_empty"))); return card; }
        var total = NativeUsagePresentation.Sum(slices.Select(slice => slice.Tokens));
        var layout = new Grid { ColumnSpacing = 16, RowSpacing = 16 };
        layout.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); layout.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); layout.RowDefinitions.Add(new() { Height = GridLength.Auto }); layout.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var pie = UsagePie(slices, total); layout.Children.Add(pie);
        var legend = new StackPanel(); layout.Children.Add(legend);
        foreach (var slice in slices)
        {
            var row = new Grid { ColumnSpacing = 7, MinHeight = 28 };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(9) }); row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto, MinWidth = 42 });
            row.Children.Add(new Border { Width = 9, Height = 9, CornerRadius = new CornerRadius(3), Background = UsageColor(slice.Color), VerticalAlignment = VerticalAlignment.Center });
            var name = UsageText(slice.Label, 11.5, "text"); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(name, slice.Label); Grid.SetColumn(name, 1); row.Children.Add(name);
            var percent = UsageText((slice.Tokens / (double)Math.Max(1, total) * 100).ToString("0.0", CultureInfo.InvariantCulture) + "%", 11.5, "text", true); Grid.SetColumn(percent, 2); row.Children.Add(percent);
            var tokens = UsageText(NativeUsagePresentation.Tokens(slice.Tokens), 11.5); tokens.HorizontalAlignment = HorizontalAlignment.Right; Grid.SetColumn(tokens, 3); row.Children.Add(tokens);
            foreach (var child in row.Children.OfType<FrameworkElement>()) child.VerticalAlignment = VerticalAlignment.Center; legend.Children.Add(row);
        }
        void Arrange()
        {
            var beside = NativeUsageLayout.ModelBesideLegend(UsageViewport); Grid.SetColumnSpan(pie, beside ? 1 : 2); Grid.SetColumnSpan(legend, beside ? 1 : 2); Grid.SetColumn(legend, beside ? 1 : 0); Grid.SetRow(legend, beside ? 0 : 1); layout.RowSpacing = beside ? 0 : 16;
        }
        layout.Loaded += (_, _) => Arrange(); layout.SizeChanged += (_, _) => Arrange(); body.Children.Add(layout); return card;
    }
    private Grid UsagePie(IReadOnlyList<NativeUsageModelSlice> slices, long total)
    {
        var root = new Grid { Width = 154, Height = 154, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        var canvas = new Canvas { Width = 154, Height = 154 }; root.Children.Add(canvas); var start = -Math.PI / 2;
        foreach (var slice in slices)
        {
            var sweep = slice.Tokens / (double)Math.Max(1, total) * Math.PI * 2;
            if (sweep >= Math.PI * 2 - .000001) canvas.Children.Add(new Ellipse { Width = 154, Height = 154, Fill = UsageColor(slice.Color) });
            else
            {
                Point At(double angle) => new(77 + 77 * Math.Cos(angle), 77 + 77 * Math.Sin(angle));
                var figure = new PathFigure { StartPoint = new Point(77, 77), IsClosed = true, IsFilled = true };
                figure.Segments.Add(new LineSegment { Point = At(start) }); figure.Segments.Add(new ArcSegment { Point = At(start + sweep), Size = new Size(77, 77), IsLargeArc = sweep > Math.PI, SweepDirection = SweepDirection.Clockwise });
                var geometry = new PathGeometry(); geometry.Figures.Add(figure); canvas.Children.Add(new Microsoft.UI.Xaml.Shapes.Path { Data = geometry, Fill = UsageColor(slice.Color) });
            }
            start += sweep;
        }
        var center = new StackPanel { HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        var tokens = UsageText(NativeUsagePresentation.Tokens(total), 18, "text", true); tokens.HorizontalAlignment = HorizontalAlignment.Center; center.Children.Add(tokens);
        var label = UsageText(T("settings.usage.tokens"), 10); label.HorizontalAlignment = HorizontalAlignment.Center; center.Children.Add(label);
        root.Children.Add(new Border { Width = 94, Height = 94, CornerRadius = new CornerRadius(47), Child = center, Background = Design.Brush("bg-elev"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1) });
        AutomationProperties.SetName(root, T("settings.usage.model_share") + ": " + NativeUsagePresentation.Tokens(total)); return root;
    }
    private Border UsageTools(JsonNode overview)
    {
        var card = UsageCard("tool_rank", "tool_rank_hint", out var body); var rows = NativeUsagePresentation.Tools(overview, T("settings.usage.other_tools"));
        if (rows.Length == 0) { body.Children.Add(ModelHint(T("settings.usage.tools_empty"))); return card; }
        var total = NativeUsagePresentation.Sum(rows.Select(row => row.Calls)); var max = Math.Max(1, rows.Max(row => row.Calls)); var list = new StackPanel { Spacing = 10 }; body.Children.Add(list);
        for (var i = 0; i < rows.Length; i++)
        {
            var item = rows[i]; var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new() { Width = new GridLength(18) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            var rank = UsageText((i + 1).ToString(), 11); rank.HorizontalAlignment = HorizontalAlignment.Right; row.Children.Add(rank);
            var badge = new Border { Height = 18, Padding = new Thickness(6, 0, 6, 0), CornerRadius = new CornerRadius(4), VerticalAlignment = VerticalAlignment.Top, Background = Design.Brush("bg-sunken") };
            var caption = UsageText(T("settings.usage." + (item.Kind == "skill" ? "tool_kind_skill" : item.Kind == "mcp" ? "tool_kind_mcp" : "other_tools")).ToUpperInvariant(), 10, "text-muted", true); caption.VerticalAlignment = VerticalAlignment.Center; caption.CharacterSpacing = 20; badge.Child = caption;
            if (item.Kind is "skill" or "mcp")
            { var color = item.Kind == "skill" ? Design.Brush("clay").Color : UsageColor("#3b82f6").Color; badge.Background = new SolidColorBrush(Mix(Design.Brush("bg-sunken").Color, color, item.Kind == "skill" ? .16 : .14)); caption.Foreground = new SolidColorBrush(Mix(Design.Brush("text").Color, color, item.Kind == "skill" ? .85 : .80)); }
            Grid.SetColumn(badge, 1); row.Children.Add(badge);
            var main = new StackPanel { Spacing = 4 }; var meta = new Grid { ColumnSpacing = 7 };
            meta.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); meta.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); meta.ColumnDefinitions.Add(new() { Width = GridLength.Auto, MinWidth = 28 });
            var name = UsageText(item.Name, 11.5, "text"); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(name, item.Name); meta.Children.Add(name);
            var percent = UsageText((item.Calls / (double)Math.Max(1, total) * 100).ToString("0.0", CultureInfo.InvariantCulture) + "%", 11.5, "text", true); Grid.SetColumn(percent, 1); meta.Children.Add(percent);
            var count = UsageText(item.Calls.ToString(CultureInfo.InvariantCulture), 11.5); count.HorizontalAlignment = HorizontalAlignment.Right; Grid.SetColumn(count, 2); meta.Children.Add(count); main.Children.Add(meta);
            var track = new Grid { Height = 5, Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(3) }; var ratio = item.Calls / (double)max;
            track.ColumnDefinitions.Add(new() { Width = new GridLength(ratio, GridUnitType.Star) }); track.ColumnDefinitions.Add(new() { Width = new GridLength(1 - ratio, GridUnitType.Star) }); track.Children.Add(new Border { Background = UsageColor(item.Color), CornerRadius = new CornerRadius(3) }); main.Children.Add(track);
            Grid.SetColumn(main, 2); row.Children.Add(main); list.Children.Add(row);
        }
        return card;
    }
    private FrameworkElement UsageTable(IReadOnlyList<NativeUsageWorkspace>? workspaces = null)
    {
        var list = new StackPanel(); var rows = new List<Grid>();
        var header = UsageTableRow(UsageText(T("settings.usage." + (workspaces == null ? "session" : "workspace")), 12), null, true); rows.Add(header); list.Children.Add(header);
        if (workspaces != null)
        {
            foreach (var workspace in workspaces)
            {
                var copy = UsageSessionCopy(workspace.DisplayName, Design.Localization.Format("settings.usage.sessions", ("n", workspace.Sessions.ToString())) + " · " + NativeBrowserPresentation.RelativeTime(workspace.UpdatedAt, DateTimeOffset.Now, locale: Design.Localization), workspace.Directory);
                var row = UsageTableRow(copy, workspace.Totals); row.MinHeight = 58; row.Padding = new Thickness(0, 5, 0, 5); rows.Add(row);
                var button = new Button { Content = row, HorizontalContentAlignment = HorizontalAlignment.Stretch, HorizontalAlignment = HorizontalAlignment.Stretch, MinHeight = 0, MinWidth = 0 }; Design.QuietButton(button); button.Padding = new Thickness(0); button.CornerRadius = new CornerRadius(0); button.FocusVisualPrimaryBrush = Design.Brush("clay");
                AutomationProperties.SetName(button, Design.Localization.Format("settings.usage.open_workspace", ("name", workspace.DisplayName))); button.Click += async (_, _) => { BodyScroll.ChangeView(null, 0, null, true); if (usagePager != null) await usagePager.SelectAsync(workspace); };
                list.Children.Add(button);
            }
        }
        else foreach (var session in usagePager?.Items ?? [])
        {
            var row = UsageTableRow(UsageSessionCopy(session.Title, NativeBrowserPresentation.RelativeTime(session.UpdatedAt, DateTimeOffset.Now, locale: Design.Localization), ""), session.Totals); rows.Add(row); list.Children.Add(row);
        }
        if (rows.Count > 1) rows[^1].BorderThickness = new Thickness(0);
        var scroll = new ScrollViewer { Content = list, HorizontalScrollMode = ScrollMode.Enabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        void Arrange()
        {
            var compact = NativeUsageLayout.Compact(UsageViewport); list.MinWidth = NativeUsageLayout.TableMinimum(UsageViewport);
            foreach (var row in rows)
            {
                row.ColumnSpacing = compact ? 5 : 8; row.ColumnDefinitions[0].MinWidth = compact ? 150 : 120;
                if (row.Children[0] is StackPanel copy && copy.Children[0] is TextBlock title) Design.BindTypography(title, compact ? 11.5 : 13);
                for (var i = 1; i < 5; i++) { row.ColumnDefinitions[i].Width = new GridLength(compact ? 58 : 72); if (row.Children[i] is TextBlock number) Design.BindTypography(number, compact ? 11.5 : 13); }
            }
        }
        scroll.Loaded += (_, _) => Arrange(); scroll.SizeChanged += (_, _) => Arrange(); return scroll;
    }
    private StackPanel UsageSessionCopy(string title, string when, string path)
    {
        var copy = new StackPanel { Spacing = 1, VerticalAlignment = VerticalAlignment.Center };
        var name = UsageText(title, 13, "text"); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(name, title); copy.Children.Add(name); copy.Children.Add(UsageText(when, 11));
        if (path.Length > 0) { var location = UsageText(path, 9.5); location.FontFamily = Design.CodeFont; location.TextWrapping = TextWrapping.NoWrap; location.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(location, path); copy.Children.Add(location); }
        return copy;
    }
    private Grid UsageTableRow(FrameworkElement title, NativeUsageTotals? totals, bool heading = false)
    {
        var row = new Grid { MinHeight = 38, BorderThickness = new Thickness(0, 0, 0, 1), BorderBrush = Design.Brush("border"), ColumnSpacing = 8 };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.Children.Add(title);
        for (var i = 0; i < 4; i++)
        {
            row.ColumnDefinitions.Add(new() { Width = new GridLength(72) });
            var number = UsageText(heading ? T("settings.usage." + NativeUsagePresentation.TotalKeys[i]) : NativeUsagePresentation.Tokens(totals!.Values[i]), heading ? 12 : 13, heading ? "text-faint" : "text-muted"); number.TextAlignment = TextAlignment.Right; number.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(number, i + 1); row.Children.Add(number);
        }
        title.VerticalAlignment = VerticalAlignment.Center; return row;
    }
    private void UsageDetail(NativeUsageWorkspace workspace)
    {
        var head = new Grid { ColumnSpacing = 14, Margin = new Thickness(0, 0, 0, 8) };
        head.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); head.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        var back = new Button { Content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 } }; Design.QuietButton(back); back.Padding = new Thickness(8, 6, 8, 6);
        var backContent = (StackPanel)back.Content; backContent.Children.Add(Design.Icon("arrow-left", 14)); backContent.Children.Add(UsageText(T("settings.usage.all_workspaces"), 13, "text")); back.Click += (_, _) => BackToUsage(); head.Children.Add(back);
        var title = new StackPanel { VerticalAlignment = VerticalAlignment.Center }; var name = UsageText(workspace.DisplayName, 14, "text", true); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; title.Children.Add(name);
        if (workspace.Directory.Length > 0) { var path = UsageText(workspace.Directory, 10); path.FontFamily = Design.CodeFont; path.TextWrapping = TextWrapping.NoWrap; path.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(path, workspace.Directory); title.Children.Add(path); }
        Grid.SetColumn(title, 1); head.Children.Add(title); Form.Children.Add(head); Form.Children.Add(UsageSummary(workspace.Totals));
        if (usagePager!.Loading) { Form.Children.Add(ModelHint(T("settings.usage.loading"))); return; }
        if (usagePager.Error != null)
        {
            Form.Children.Add(Warn(L("无法读取会话用量：", "Could not load session usage: ") + usagePager.Error)); Form.Children.Add(Button(L("重试", "Retry"), () => usagePager.RetryAsync())); return;
        }
        if (usagePager.Items == null || usagePager.Items.Count == 0) { Form.Children.Add(ModelHint(T("settings.usage.sessions_empty"))); return; }
        Form.Children.Add(UsageTable());
        if (usagePager.PageCount <= 1) return;
        var pages = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 14, 0, 0) };
        var previous = Button(T("settings.usage.previous"), () => usagePager.GoAsync(usagePager.Page - 1)); Design.QuietButton(previous); previous.IsEnabled = usagePager.Page > 0; pages.Children.Add(previous);
        var current = UsageText(Design.Localization.Format("settings.usage.page", ("page", (usagePager.Page + 1).ToString()), ("pages", usagePager.PageCount.ToString())), 12, "text-muted"); current.VerticalAlignment = VerticalAlignment.Center; pages.Children.Add(current);
        var next = Button(T("settings.usage.next"), () => usagePager.GoAsync(usagePager.Page + 1)); Design.QuietButton(next); next.IsEnabled = usagePager.Page + 1 < usagePager.PageCount; pages.Children.Add(next); Form.Children.Add(pages);
    }
}
