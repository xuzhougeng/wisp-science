using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Windows.Foundation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>Read-only RpCodeView counterpart: fixed gutter, unwrapped source and shared syntax colors.</summary>
internal sealed class NativeCodeView : ContentControl, IDisposable
{
    private readonly WispDesign design;
    private readonly NativeCodePreview source;
    private readonly TextBlock body, numbers;
    private readonly ScrollViewer scroll;
    private readonly Border gutter;
    private readonly Grid grid;
    private readonly TranslateTransform gutterOffset = new();
    private readonly TextCommandBarFlyout menu = new();
    private readonly NativeSourceSelectionActions? selectionActions;
    private bool menuOpen, disposed;

    public NativeCodeView(string value, string? language, WispDesign design, double maxHeight,
        Func<NativeDocumentSelection, bool>? quote = null)
    {
        this.design = design; source = NativeCodePreview.Create(value, language);
        HorizontalContentAlignment = HorizontalAlignment.Stretch;
        body = new TextBlock { IsTextSelectionEnabled = true, TextWrapping = TextWrapping.NoWrap,
            Foreground = design.Brush("syntax-text"), Padding = new(14, 12, 14, 12), ContextFlyout = menu };
        TranscriptView.ApplyHighlight(body.Inlines, source.Text, source.Language, design, syntax: true);
        if (quote != null) selectionActions = new(body, source.Text, design, () => !disposed, quote);
        AutomationProperties.SetName(body, design.L("源代码", "Source code"));
        numbers = new TextBlock { Text = source.Gutter, TextWrapping = TextWrapping.NoWrap,
            TextAlignment = TextAlignment.Right, Foreground = design.Brush("text-faint"),
            Padding = new(10, 12, 12, 12), VerticalAlignment = VerticalAlignment.Top, RenderTransform = gutterOffset };
        AutomationProperties.SetAccessibilityView(numbers, AccessibilityView.Raw);
        // Canvas measures the complete gutter. A tall TextBlock directly in the
        // constrained Border gets a layout clip before its scroll translation.
        var gutterLayer = new Canvas(); gutterLayer.Children.Add(numbers);
        gutter = new Border { Child = gutterLayer, Background = design.Brush("bg-sunken"),
            BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 1, 0), IsHitTestVisible = false };
        gutter.SizeChanged += (_, _) => gutter.Clip = new RectangleGeometry { Rect = new Rect(0, 0, gutter.ActualWidth, gutter.ActualHeight) };
        scroll = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollMode = ScrollMode.Enabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            VerticalScrollMode = ScrollMode.Enabled, MaxHeight = Math.Max(40, maxHeight - 2) };
        scroll.ViewChanged += (_, _) => { gutterOffset.Y = -scroll.VerticalOffset; selectionActions?.DismissPopup(); };
        grid = new Grid { MaxHeight = Math.Max(40, maxHeight - 2) };
        grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); grid.ColumnDefinitions.Add(new());
        grid.Children.Add(gutter); Grid.SetColumn(scroll, 1); grid.Children.Add(scroll);
        Content = new Border { Child = grid, BorderBrush = design.Brush("border"), BorderThickness = new(1),
            Background = design.Brush("bg-app"), CornerRadius = new(10) };
        menu.Opened += (_, _) => menuOpen = true; menu.Closed += (_, _) => menuOpen = false;
        design.TypographyChanged += ApplyTypography; ApplyTypography();
    }
    private void ApplyTypography()
    {
        var size = design.FontSize(12, true); var leading = size * 1.6;
        foreach (var text in new[] { body, numbers })
        {
            text.FontFamily = design.CodeFont; text.FontSize = size;
            text.LineStackingStrategy = LineStackingStrategy.BlockLineHeight; text.LineHeight = leading;
        }
        // TextBlock omits the final empty line's height; <pre> and the gutter do not.
        body.MinHeight = 24 + source.LineCount * leading;
        numbers.Height = body.MinHeight;
        numbers.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
        gutter.Width = Math.Ceiling(numbers.DesiredSize.Width) + 1;
    }
    public bool HandleEscape() { if (selectionActions?.HandleEscape() == true) return true; if (!menuOpen) return false; menu.Hide(); return true; }
    public bool IsMenuOpen => menuOpen || selectionActions?.IsOpen == true;
    public void SetViewportHeight(double height) => scroll.MaxHeight = grid.MaxHeight = Math.Max(0, height - 2);
    public void Dispose()
    {
        if (disposed) return; disposed = true; design.TypographyChanged -= ApplyTypography;
        selectionActions?.Dispose();
        if (menuOpen) menu.Hide(); Content = null;
    }
}
