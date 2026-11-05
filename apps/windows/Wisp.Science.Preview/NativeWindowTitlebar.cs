using Microsoft.UI.Input;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Foundation;
using Windows.Graphics;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// Persistent window chrome. Only menu regions pass through the OS caption drag surface.
internal sealed class NativeWindowTitlebar : Grid, IDisposable
{
    private readonly Window window;
    private readonly WispDesign design;
    private readonly Action<string> run;
    private readonly Func<bool> canRun;
    private readonly Action<FlyoutBase> register;
    private readonly Grid brand = new() { ColumnSpacing = 7, Padding = new Thickness(12, 0, 12, 0) };
    private readonly StackPanel menus = new() { Orientation = Orientation.Horizontal };
    private readonly TextBlock name, version;
    private readonly ColumnDefinition captionInset = new() { Width = new GridLength(138) };
    private readonly Dictionary<string, Button> buttons = [];
    private MenuFlyout? openMenu;
    private string? openGroup;
    private bool home, hasProject, disposed;
    public NativeWindowTitlebar(Window window, WispDesign design, Action<string> run, Func<bool> canRun, Action<FlyoutBase> register)
    {
        this.window = window; this.design = design; this.run = run; this.canRun = canRun; this.register = register;
        Height = NativeWindowMenus.Height; BorderThickness = new Thickness(0, 0, 0, 1);
        AutomationProperties.SetAutomationId(this, "window-titlebar");
        ColumnDefinitions.Add(new() { Width = GridLength.Auto }); ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star), MinWidth = 16 }); ColumnDefinitions.Add(captionInset);
        brand.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); brand.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        brand.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        brand.Children.Add(new Image { Source = new SvgImageSource(new Uri(Path.Combine(AppContext.BaseDirectory, "Assets", "logo.svg"))), Width = 16, Height = 16 });
        name = design.Text("wisp science", 12); name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis;
        name.VerticalAlignment = VerticalAlignment.Center; name.MaxWidth = 300; Grid.SetColumn(name, 1); brand.Children.Add(name);
        version = design.Text("", 11); version.TextWrapping = TextWrapping.NoWrap; version.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(version, 2); brand.Children.Add(version); Children.Add(brand);
        AutomationProperties.SetName(menus, "Application menu"); Grid.SetColumn(menus, 1); Children.Add(menus);
        foreach (var group in NativeWindowMenus.Groups)
        {
            var button = new Button { Padding = new Thickness(10, 0, 10, 0), MinWidth = 0, MinHeight = 0,
                Height = NativeWindowMenus.Height - 1, BorderThickness = new Thickness(0), CornerRadius = new CornerRadius(0),
                Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
            design.BindTypography(button, size: 13); AutomationProperties.SetAutomationId(button, "window-menu-" + group);
            button.Click += (_, _) => { if (openGroup == group) openMenu?.Hide(); else Open(group); };
            button.PointerEntered += (_, _) => { if (openMenu != null && openGroup != group) Open(group); };
            buttons[group] = button; menus.Children.Add(button);
        }
        Loaded += (_, _) => UpdateRegions(); SizeChanged += (_, _) => UpdateRegions(); menus.SizeChanged += (_, _) => UpdateRegions();
        brand.SizeChanged += (_, _) => UpdateRegions();
        window.SizeChanged += WindowSizeChanged;
        window.ExtendsContentIntoTitleBar = true; window.SetTitleBar(this);
        window.AppWindow.TitleBar.PreferredHeightOption = TitleBarHeightOption.Standard;
        Update(null, null, true, false);
    }
    public void Update(string? project, string? appVersion, bool isHome, bool currentProject)
    {
        home = isHome; hasProject = currentProject;
        name.Text = NativeHomePresentation.WindowTitle(project); ToolTipService.SetToolTip(name, name.Text);
        version.Text = string.IsNullOrWhiteSpace(appVersion) ? "" : "v" + appVersion;
        name.Foreground = design.Brush("text"); version.Foreground = design.Brush("text-faint");
        Background = design.Brush("bg-app"); BorderBrush = design.Brush("border");
        RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light;
        foreach (var (group, button) in buttons)
        {
            button.Content = design.T("menu." + group); AutomationProperties.SetName(button, button.Content.ToString());
            button.Foreground = design.Brush("text-muted");
            button.Resources["ButtonBackgroundPointerOver"] = design.Brush("surface-hover");
            button.Resources["ButtonBackgroundPressed"] = design.Brush("surface-hover");
        }
        var caption = window.AppWindow.TitleBar;
        caption.ButtonBackgroundColor = caption.ButtonInactiveBackgroundColor = Microsoft.UI.Colors.Transparent;
        caption.ButtonForegroundColor = design.Brush("text").Color;
        caption.ButtonInactiveForegroundColor = design.Brush("text-faint").Color;
        caption.ButtonHoverBackgroundColor = design.Brush("surface-hover").Color;
        caption.ButtonHoverForegroundColor = design.Brush("text").Color;
        DispatcherQueue.TryEnqueue(UpdateRegions);
    }
    public void Open(string group)
    {
        if (disposed || !buttons.TryGetValue(group, out var owner)) return;
        openMenu?.Hide();
        var menu = new MenuFlyout { Placement = FlyoutPlacementMode.BottomEdgeAlignedLeft };
        foreach (var item in NativeWindowMenus.Items(group, home))
        {
            if (item.Action.Length == 0) { menu.Items.Add(new MenuFlyoutSeparator()); continue; }
            var action = new MenuFlyoutItem { Text = design.T(item.Key), KeyboardAcceleratorTextOverride = item.Shortcut,
                IsEnabled = canRun() && NativeWindowMenus.Enabled(item.Action, hasProject), MinHeight = 0, Padding = new Thickness(10, 6, 10, 6) };
            design.BindTypography(action, size: 13);
            AutomationProperties.SetAutomationId(action, "window-action-" + item.Action);
            action.Click += (_, _) => { menu.Hide(); if (canRun()) run(item.Action); };
            menu.Items.Add(action);
        }
        var style = new Style(typeof(MenuFlyoutPresenter));
        style.Setters.Add(new Setter(MinWidthProperty, 220d)); style.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(4)));
        style.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        style.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        style.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(8)));
        menu.MenuFlyoutPresenterStyle = style;
        menu.Closed += (_, _) => { if (ReferenceEquals(openMenu, menu)) { openMenu = null; openGroup = null; } };
        register(menu); openMenu = menu; openGroup = group; menu.ShowAt(owner);
    }
    private void UpdateRegions()
    {
        if (disposed || XamlRoot == null || ActualWidth <= 0 || !window.ExtendsContentIntoTitleBar) return;
        var scale = XamlRoot.RasterizationScale;
        // Auto-sized menu/brand columns can retain their previous desired width
        // while the client shrinks. Bound to the client, not that stale measure.
        var available = XamlRoot.Size.Width;
        MaxWidth = available;
        captionInset.Width = new GridLength(window.AppWindow.TitleBar.RightInset / scale);
        // Reserve the menu and OS caption controls; the brand elides first on small windows.
        brand.MaxWidth = Math.Max(24, available - menus.ActualWidth - captionInset.Width.Value - 16);
        var bounds = menus.TransformToVisual(null).TransformBounds(new Rect(0, 0, menus.ActualWidth, menus.ActualHeight));
        InputNonClientPointerSource.GetForWindowId(window.AppWindow.Id).SetRegionRects(NonClientRegionKind.Passthrough,
            [new RectInt32((int)Math.Round(bounds.X * scale), (int)Math.Round(bounds.Y * scale),
                (int)Math.Round(bounds.Width * scale), (int)Math.Round(bounds.Height * scale))]);
    }
    private void WindowSizeChanged(object sender, WindowSizeChangedEventArgs args) => DispatcherQueue.TryEnqueue(UpdateRegions);
    public void Dispose() { disposed = true; window.SizeChanged -= WindowSizeChanged; openMenu?.Hide(); }
}
