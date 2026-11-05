using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal static class NativeBranchResultsView
{
    public static FrameworkElement Links(NativeConversationBranch[] branches, WispDesign design, bool expanded,
        Action<bool> changed, Action<string> open, Action<string, string> result)
    {
        var group = new StackPanel { Spacing = 5, MaxWidth = 320, HorizontalAlignment = HorizontalAlignment.Left, Margin = new(28, 8, 0, 4) };
        FollowParentWidth(group, 320, 28);
        var entries = new StackPanel { Spacing = 7 };
        var toggle = new Button { HorizontalAlignment = HorizontalAlignment.Left };
        design.QuietButton(toggle); toggle.MinHeight = 0; toggle.Padding = new(4, 2, 4, 2);
        void Update()
        {
            entries.Visibility = expanded ? Visibility.Visible : Visibility.Collapsed;
            var heading = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            heading.Children.Add(design.Icon(expanded ? "chevron-down" : "chevron-right", 13, design.ColorText("text-faint")));
            var title = design.Text(design.T("branch.group") + " " + branches.Length, 11); title.Foreground = design.Brush("text-faint"); title.FontWeight = FontWeights.SemiBold; heading.Children.Add(title); toggle.Content = heading;
            AutomationProperties.SetName(toggle, design.T(expanded ? "branch.collapse" : "branch.expand") + " " + branches.Length);
        }
        toggle.Click += (_, _) => { expanded = !expanded; changed(expanded); Update(); };
        group.Children.Add(toggle); group.Children.Add(entries);
        foreach (var branch in branches)
        {
            var entry = new StackPanel { Spacing = 5 };
            var title = NativeBranchResults.Title(branch.Title, design.T("sidebar.untitled"));
            var link = Button(design); link.Padding = new(10, 7, 10, 7);
            var content = new Grid { ColumnSpacing = 7 }; content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.ColumnDefinitions.Add(new());
            content.Children.Add(design.Icon("branch", 14, design.ColorText("text-muted")));
            var name = design.Text(title, 11); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; name.Foreground = design.Brush("text-muted"); Grid.SetColumn(name, 1); content.Children.Add(name);
            link.Content = content; AutomationProperties.SetName(link, title); ToolTipService.SetToolTip(link, title);
            link.Click += (_, _) => open(branch.Id); entry.Children.Add(link);
            if (branch.MergeSummary is { } summary)
            {
                var merged = Card(title, design, () => result(title, summary), nested: true);
                var connector = new Grid { Margin = new(6, 0, 0, 0) }; connector.ColumnDefinitions.Add(new() { Width = new GridLength(12) }); connector.ColumnDefinitions.Add(new());
                connector.Children.Add(new Border { BorderBrush = design.Brush("border-strong"), BorderThickness = new(1, 0, 0, 1), CornerRadius = new(0, 0, 0, 6), Height = 16, Margin = new(0, -6, 3, 0), VerticalAlignment = VerticalAlignment.Top });
                Grid.SetColumn(merged, 1); connector.Children.Add(merged); entry.Children.Add(connector);
            }
            entries.Children.Add(entry);
        }
        Update(); return group;
    }

    public static Button Card(string? title, WispDesign design, Action open, bool nested = false)
    {
        var card = Button(design); card.MaxWidth = nested ? double.PositiveInfinity : 620;
        card.HorizontalAlignment = nested ? HorizontalAlignment.Stretch : HorizontalAlignment.Left; card.Padding = nested ? new(9, 6, 9, 6) : new(14, 12, 14, 12);
        if (!nested) FollowParentWidth(card, 620, 0);
        card.MinHeight = nested ? 32 : 0; card.CornerRadius = new(nested ? 6 : 10);
        var row = new Grid { ColumnSpacing = nested ? 8 : 11 }; row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var icon = design.Icon(nested ? "check" : "branch", nested ? 14 : 20, design.ColorText(nested ? "text-faint" : "clay"));
        row.Children.Add(nested ? icon : new Border { Child = icon, Width = 36, Height = 36, CornerRadius = new(10), Background = design.Brush("bg-sunken") });
        var copy = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
        var caption = design.Text(design.T("branch.merged_result"), nested ? 11 : 13); caption.FontWeight = FontWeights.SemiBold; copy.Children.Add(caption);
        if (!nested)
        {
            var subtitle = design.Text(title ?? "", 11); subtitle.Foreground = design.Brush("text-faint"); subtitle.TextWrapping = TextWrapping.NoWrap; subtitle.TextTrimming = TextTrimming.CharacterEllipsis; copy.Children.Add(subtitle);
        }
        Grid.SetColumn(copy, 1); row.Children.Add(copy);
        var arrow = design.Icon("chevron-right", 14, design.ColorText("text-faint")); Grid.SetColumn(arrow, 2); row.Children.Add(arrow); card.Content = row;
        AutomationProperties.SetName(card, design.T("branch.merged_result") + (string.IsNullOrWhiteSpace(title) ? "" : ": " + title));
        card.Click += (_, _) => open(); return card;
    }

    private static Button Button(WispDesign design)
    {
        var button = new Button { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            Background = design.Brush("bg-elev"), Foreground = design.Brush("text"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(6) };
        button.Resources["ButtonBackgroundPointerOver"] = design.Brush("surface-hover");
        button.Resources["ButtonBorderBrushPointerOver"] = design.Brush("clay");
        return button;
    }

    private static void FollowParentWidth(FrameworkElement element, double preferred, double inset)
    {
        FrameworkElement? parent = null;
        void Resize(object? sender, SizeChangedEventArgs args) => element.Width = Math.Min(preferred, Math.Max(0, (parent?.ActualWidth ?? preferred + inset) - inset));
        element.Loaded += (_, _) =>
        {
            parent = element.Parent as FrameworkElement;
            if (parent != null) { parent.SizeChanged += Resize; element.Width = Math.Min(preferred, Math.Max(0, parent.ActualWidth - inset)); }
        };
        element.Unloaded += (_, _) => { if (parent != null) parent.SizeChanged -= Resize; parent = null; };
    }
}

internal sealed class NativeBranchResultPage : UserControl, IWorkspaceSheet
{
    private readonly Action close;
    private bool disposed;
    public bool IsModal => true;
    public NativeBranchResultPage(string title, string summary, WispDesign design, Action close, Action<FlyoutBase> register)
    {
        this.close = close;
        var overlay = new Grid(); NativeDialogVisuals.Backdrop(overlay, design); Content = overlay;
        var root = new Grid { Padding = new(16), RowSpacing = 12, TabFocusNavigation = KeyboardNavigationMode.Cycle };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new());
        var header = new Grid { ColumnSpacing = 10, MinHeight = 30 };
        header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var name = design.Text(NativeBranchResults.Title(title, design.T("branch.merged_result")), 14); name.FontWeight = FontWeights.SemiBold; name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; name.VerticalAlignment = VerticalAlignment.Center;
        header.Children.Add(name); ToolTipService.SetToolTip(name, name.Text);
        var tagText = design.Text(design.T("branch.merged_result"), 10); tagText.Foreground = design.Brush("text-faint"); tagText.FontWeight = FontWeights.SemiBold;
        var tag = new Border { Child = tagText, Padding = new(7, 2, 7, 2), CornerRadius = new(999), BorderThickness = new(1), BorderBrush = design.Brush("border"), Background = design.Brush("bg-sunken"), VerticalAlignment = VerticalAlignment.Center };
        Grid.SetColumn(tag, 1); header.Children.Add(tag);
        var dismiss = design.ToolButton(design.T("right.close"), "close"); dismiss.Click += (_, _) => HandleEscape(); Grid.SetColumn(dismiss, 2); header.Children.Add(dismiss); root.Children.Add(header);
        var markdown = TranscriptView.RenderMarkdown(summary, design, skillDocument: true);
        var document = new Border { Child = markdown, MaxWidth = 720, Padding = new(42), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(6), HorizontalAlignment = HorizontalAlignment.Stretch };
        var scroll = new ScrollViewer { Content = document, Padding = new(28), Background = design.Brush("bg-sunken"), HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Grid.SetRow(scroll, 1); root.Children.Add(scroll);
        var card = new Border { Child = root, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(14), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
        overlay.Children.Add(card); AutomationProperties.SetName(card, name.Text + ": " + design.T("branch.merged_result"));
        void Resize()
        {
            var narrow = ActualWidth <= 640;
            card.Width = Math.Min(880, Math.Max(0, ActualWidth - (narrow ? 24 : 40)));
            card.Height = Math.Min(760, Math.Max(0, Math.Min(ActualHeight * (narrow ? .86 : .82), ActualHeight - 40)));
            root.Padding = new(narrow ? 12 : 16); scroll.Padding = new(narrow ? 8 : 28); document.Padding = new(narrow ? 22 : 42);
            tag.Visibility = card.Width < 410 ? Visibility.Collapsed : Visibility.Visible;
        }
        SizeChanged += (_, _) => Resize();
        Loaded += (_, _) => { Resize(); design.ApplyTypography(this); InstallSelectionMenus(markdown, register); dismiss.Focus(FocusState.Programmatic); };
    }
    private static void InstallSelectionMenus(DependencyObject node, Action<FlyoutBase> register)
    {
        if (node is RichTextBlock rich) { var menu = new TextCommandBarFlyout(); rich.SelectionFlyout = menu; rich.ContextFlyout = menu; register(menu); }
        else if (node is TextBlock { IsTextSelectionEnabled: true } text) { var menu = new TextCommandBarFlyout(); text.SelectionFlyout = menu; text.ContextFlyout = menu; register(menu); }
        for (var i = 0; i < VisualTreeHelper.GetChildrenCount(node); i++) InstallSelectionMenus(VisualTreeHelper.GetChild(node, i), register);
    }
    public void HandleEscape() { if (!disposed) close(); }
    public void Dispose() { disposed = true; }
}
