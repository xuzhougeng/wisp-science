namespace Wisp.ProjectBrowser;

/// <summary>WebView project-modal bounds in effective pixels, including its border.</summary>
public readonly record struct ProjectDialogLayout(double Width, double MaxHeight)
{
    public const double PreferredWidth = 560;
    public const double OuterPadding = 20;

    // .ps-textarea has 9px vertical padding, a 1px border and 1.5 line height;
    // .ps-ctx alone declares a 150px minimum.
    public static double EditorMinimum(int rows) => rows == 8 ? 150 : 20;
    public static double EditorHeight(int rows, double fontSize)
        => Math.Max(EditorMinimum(rows), 20 + rows * fontSize * 1.5);

    public static ProjectDialogLayout ForViewport(double width, double height, double preferredWidth = PreferredWidth) => new(
        Math.Min(preferredWidth, Available(width)), Available(height));

    // .proj-settings-modal is 92vw, capped by both 560px and the overlay padding.
    public static ProjectDialogLayout ForProjectForm(double width, double height) =>
        ForViewport(width, height, double.IsFinite(width) ? Math.Clamp(width * .92, 0, PreferredWidth) : 0);

    private static double Available(double size) => double.IsFinite(size)
        ? Math.Max(0, size - 2 * OuterPadding) : 0;
}
