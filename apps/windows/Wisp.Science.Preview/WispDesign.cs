using System.Globalization;
using System.ComponentModel;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Microsoft.UI.Xaml.Data;
using Windows.Storage.Streams;

namespace Wisp.Science.Preview;

internal sealed class WispDesign : INotifyPropertyChanged
{
    private Wisp.ProjectBrowser.NativeTypography typography = new();
    public event Action? TypographyChanged;
    public event PropertyChangedEventHandler? PropertyChanged;
    public Wisp.ProjectBrowser.NativeTypography Typography
    {
        get => typography;
        set
        {
            if (typography == value) return;
            typography = value;
            foreach (var name in new[] { nameof(UiFont), nameof(CodeFont), nameof(UiSize), nameof(CodeSize) })
                PropertyChanged?.Invoke(this, new(name));
            TypographyChanged?.Invoke();
        }
    }
    public FontFamily UiFont => Font();
    public FontFamily CodeFont => Font(true);
    public double UiSize => Typography.UiSize;
    public double CodeSize => Typography.CodeSize;
    public void BindTypography(Control control, bool code = false)
    {
        Bind(control, Control.FontFamilyProperty, code ? nameof(CodeFont) : nameof(UiFont));
        Bind(control, Control.FontSizeProperty, code ? nameof(CodeSize) : nameof(UiSize));
    }
    public TextBlock Text(string text, double size = 14)
    {
        var block = new TextBlock { Text = text, TextWrapping = TextWrapping.Wrap };
        BindTypography(block, size);
        return block;
    }
    public void BindTypography(TextBlock block, double size = 14)
    {
        Bind(block, TextBlock.FontFamilyProperty, nameof(UiFont));
        block.SetBinding(TextBlock.FontSizeProperty, new Binding { Source = this, Path = new PropertyPath(nameof(UiSize)),
            Mode = BindingMode.OneWay, Converter = new ScaledFontSize(), ConverterParameter = size });
    }
    private void Bind(FrameworkElement target, DependencyProperty property, string path)
        => target.SetBinding(property, new Binding { Source = this, Path = new PropertyPath(path), Mode = BindingMode.OneWay });
    // Walk only app-owned content, never generated control templates or icon glyphs.
    // Existing bindings and explicit font families belong to their renderer.
    public void ApplyTypography(DependencyObject? element, DependencyObject? exclude = null)
    {
        if (element == null || ReferenceEquals(element, exclude)) return;
        if (element is Control control && control.ReadLocalValue(Control.FontFamilyProperty) == DependencyProperty.UnsetValue)
        {
            var size = control.ReadLocalValue(Control.FontSizeProperty) is double local ? local : 14;
            Bind(control, Control.FontFamilyProperty, nameof(UiFont));
            control.SetBinding(Control.FontSizeProperty, new Binding { Source = this, Path = new PropertyPath(nameof(UiSize)),
                Mode = BindingMode.OneWay, Converter = new ScaledFontSize(), ConverterParameter = size });
        }
        else if (element is TextBlock text && text.ReadLocalValue(TextBlock.FontFamilyProperty) == DependencyProperty.UnsetValue)
            BindTypography(text, text.ReadLocalValue(TextBlock.FontSizeProperty) is double local ? local : 14);
        switch (element)
        {
            case Panel panel: foreach (var child in panel.Children) ApplyTypography(child, exclude); break;
            case Border border: ApplyTypography(border.Child, exclude); break;
            case ContentControl content: ApplyTypography(content.Content as DependencyObject, exclude); break;
            case ItemsControl items: foreach (var item in items.Items) ApplyTypography(item as DependencyObject, exclude); break;
        }
    }
    private sealed class ScaledFontSize : IValueConverter
    {
        public object Convert(object value, Type targetType, object parameter, string language) => (double)value * (double)parameter / 14;
        public object ConvertBack(object value, Type targetType, object parameter, string language) => throw new NotSupportedException();
    }
    public double FontSize(double size, bool code = false) => Typography.Scale(size, code);
    public FontFamily Font(bool code = false) => new(code ? Typography.CodeFamily : Typography.UiFamily);
    private readonly Dictionary<string, Dictionary<string, string>> palettes = JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", "palette.json")))!;
    private readonly Dictionary<string, SvgImageSource> icons = new();
    private readonly Dictionary<string, SolidColorBrush> brushes = new();
    private string? renderedIconColor;
    public bool Dark { get; set; }
    public string LightPalette { get; set; } = "paper";
    public string DarkPalette { get; set; } = "charcoal";
    public string ColorText(string token)
    {
        var theme = Dark ? "dark" : "light";
        return (palettes.TryGetValue(theme + "-" + (Dark ? DarkPalette : LightPalette), out var palette)
            ? palette : palettes[theme])[token];
    }
    public SolidColorBrush Brush(string token)
    {
        if (!brushes.TryGetValue(token, out var brush))
            brushes[token] = brush = new(ParseColor(token));
        else brush.Color = ParseColor(token);
        return brush;
    }

    // Pages are retained while the shell rerenders; update their existing resources
    // instead of rebuilding controls and losing focus, drafts or open previews.
    public void RefreshPalette()
    {
        foreach (var (token, brush) in brushes) brush.Color = ParseColor(token);
        var color = ColorText("text");
        if (renderedIconColor == color) return;
        renderedIconColor = color;
        foreach (var (name, source) in icons) _ = RecolorIconAsync(source, name, color);
    }

    private Windows.UI.Color ParseColor(string token)
    {
        var value = ColorText(token);
        if (value.StartsWith('#') && value.Length == 4)
            value = "#" + string.Concat(value.Skip(1).Select(c => new string(c, 2)));
        if (value.StartsWith('#')) return Windows.UI.Color.FromArgb(255,
            byte.Parse(value[1..3], NumberStyles.HexNumber), byte.Parse(value[3..5], NumberStyles.HexNumber), byte.Parse(value[5..7], NumberStyles.HexNumber));
        var parts = value[5..^1].Split(',').Select(v => double.Parse(v, CultureInfo.InvariantCulture)).ToArray();
        return Windows.UI.Color.FromArgb((byte)(parts[3] * 255), (byte)parts[0], (byte)parts[1], (byte)parts[2]);
    }

    public void QuietButton(Button button)
    {
        button.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        button.Foreground = Brush("text");
        button.BorderThickness = new Thickness(0);
        button.CornerRadius = new CornerRadius(8);
        button.MinHeight = 32;
        button.Padding = new Thickness(8, 6, 8, 6);
        button.Resources["ButtonBackgroundPointerOver"] = Brush("surface-hover");
        button.Resources["ButtonBackgroundPressed"] = Brush("bg-sunken");
        button.Resources["ButtonBorderBrushPointerOver"] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        button.Resources["ButtonBorderBrushPressed"] = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
    }

    public Button ToolButton(string label, string icon, bool showLabel = false)
    {
        var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        content.Children.Add(Icon(icon, 16));
        if (showLabel) content.Children.Add(Text(label, 12));
        var button = new Button { Content = content, VerticalAlignment = VerticalAlignment.Center };
        QuietButton(button);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(button, label);
        ToolTipService.SetToolTip(button, label);
        return button;
    }

    public void ActionButton(Button button, bool primary = false)
    {
        BindTypography(button);
        button.Padding = new Thickness(14, 8, 14, 8);
        button.MinHeight = 36; button.CornerRadius = new CornerRadius(8);
        button.Background = Brush(primary ? "clay-strong" : "bg-elev");
        button.Foreground = primary ? new SolidColorBrush(Microsoft.UI.Colors.White) : Brush("text");
        button.BorderBrush = Brush(primary ? "clay-strong" : "border");
        button.BorderThickness = new Thickness(1);
        if (button.Content is string label) button.Content = Text(label, 13);
        button.Resources["ButtonBackgroundPointerOver"] = Brush(primary ? "clay" : "surface-hover");
        button.Resources["ButtonBackgroundPressed"] = Brush(primary ? "clay-strong" : "bg-sunken");
    }

    public Border Card(UIElement content) => new()
    {
        Child = content, Padding = new Thickness(18), CornerRadius = new CornerRadius(12),
        Background = Brush("bg-elev"), BorderBrush = Brush("border"), BorderThickness = new Thickness(1)
    };

    public Border EmptyState(string icon, string title, string description)
    {
        var content = new StackPanel { Spacing = 10, Padding = new Thickness(6, 14, 6, 14) };
        var symbol = Icon(icon, 24); symbol.HorizontalAlignment = HorizontalAlignment.Left; symbol.Opacity = 0.65;
        content.Children.Add(symbol);
        var heading = Text(title, 15); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        content.Children.Add(heading);
        var detail = Text(description, 12); detail.Foreground = Brush("text-muted"); content.Children.Add(detail);
        var card = Card(content); card.Background = Brush("bg-sunken"); card.BorderThickness = new Thickness(0);
        return card;
    }

    public Image Wordmark() => new()
    {
        Source = new SvgImageSource(new Uri(Path.Combine(AppContext.BaseDirectory, "Assets", $"wordmark-{(Dark ? "dark" : "light")}.svg"))),
        Width = 180, Height = 119, Stretch = Stretch.Uniform
    };

    public Image Icon(string name, int size = 18)
    {
        if (!icons.TryGetValue(name, out var source))
        {
            source = new SvgImageSource();
            icons[name] = source;
            // Recolor the exported compose_icon SVG, retaining the shared paths.
            _ = RecolorIconAsync(source, name, ColorText("text"));
        }
        return new Image { Source = source, Width = size, Height = size, Stretch = Stretch.Uniform };
    }

    private static Task RecolorIconAsync(SvgImageSource source, string name, string color) =>
        LoadIconAsync(source, File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", $"icon-{name}.svg"))
            .Replace("#000000", color));

    private static async Task LoadIconAsync(SvgImageSource source, string svg)
    {
        using var stream = new InMemoryRandomAccessStream();
        using (var writer = new DataWriter(stream.GetOutputStreamAt(0)))
        {
            writer.WriteString(svg);
            await writer.StoreAsync();
        }
        stream.Seek(0);
        await source.SetSourceAsync(stream);
    }
}
