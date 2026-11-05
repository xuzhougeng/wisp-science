namespace Wisp.ProjectBrowser;

/// The studio's two independently resizable sidebars keep the graph available.
public readonly record struct NativeWorkflowLayout(double LibraryWidth, double InspectorWidth)
{
    public static bool StackPanels(double width) => double.IsFinite(width) && width <= 720;
    public static bool StackToolbar(double width) => !double.IsFinite(width) || width < 540;
    public static bool ShowHeaderSummary(double width) => double.IsFinite(width) && width > 980;
    public static double ConfigurationBodyMaxHeight(double width, double height, bool selected)
    {
        height = double.IsFinite(height) ? Math.Max(0, height) : 0;
        return StackPanels(width) ? 400 : selected ? Math.Min(420, height * .42) : Math.Max(0, height - 240);
    }
    public static NativeWorkflowLayout ForWidth(double width, double? library = null, double? inspector = null)
    {
        width = double.IsFinite(width) ? Math.Max(0, width) : 0;
        static double Clamp(double value, double minimum, double maximum) => Math.Clamp(value, Math.Min(minimum, maximum), maximum);
        var left = Clamp(library is { } l && double.IsFinite(l) ? l : width <= 1280 ? 210 : 240, 180, width * .32);
        var right = Clamp(inspector is { } r && double.IsFinite(r) ? r : width <= 1280 ? 260 : 300, 240, (width - left) * .52);
        return new(left, right);
    }
}
