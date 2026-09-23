using System.Globalization;
using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Streams;

namespace Wisp.Science.Preview;

internal sealed class WispDesign
{
    private Wisp.ProjectBrowser.NativeTypography typography = new();
    public event Action? TypographyChanged;
    public Wisp.ProjectBrowser.NativeTypography Typography
    {
        get => typography;
        set { if (typography == value) return; typography = value; TypographyChanged?.Invoke(); }
    }
    public double FontSize(double size, bool code = false) => Typography.Scale(size, code);
    public FontFamily Font(bool code = false) => new(code ? Typography.CodeFamily : Typography.UiFamily);
    private readonly Dictionary<string, Dictionary<string, string>> palettes = JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", "palette.json")))!;
    private readonly Dictionary<string, SvgImageSource> icons = new();
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
        var value = ColorText(token);
        if (value.StartsWith('#') && value.Length == 4)
            value = "#" + string.Concat(value.Skip(1).Select(c => new string(c, 2)));
        if (value.StartsWith('#')) return new(Windows.UI.Color.FromArgb(255,
            byte.Parse(value[1..3], NumberStyles.HexNumber), byte.Parse(value[3..5], NumberStyles.HexNumber), byte.Parse(value[5..7], NumberStyles.HexNumber)));
        var parts = value[5..^1].Split(',').Select(v => double.Parse(v, CultureInfo.InvariantCulture)).ToArray();
        return new(Windows.UI.Color.FromArgb((byte)(parts[3] * 255), (byte)parts[0], (byte)parts[1], (byte)parts[2]));
    }

    public Image Wordmark() => new()
    {
        Source = new SvgImageSource(new Uri(Path.Combine(AppContext.BaseDirectory, "Assets", $"wordmark-{(Dark ? "dark" : "light")}.svg"))),
        Width = 180, Height = 119, Stretch = Stretch.Uniform
    };

    public Image Icon(string name, int size = 18)
    {
        var key = $"{ColorText("text")}:{name}";
        if (!icons.TryGetValue(key, out var source))
        {
            source = new SvgImageSource();
            icons[key] = source;
            // Recolor the exported compose_icon SVG, retaining the shared paths.
            _ = LoadIconAsync(source, File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", $"icon-{name}.svg"))
                .Replace("#000000", ColorText("text")));
        }
        return new Image { Source = source, Width = size, Height = size, Stretch = Stretch.Uniform };
    }

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
