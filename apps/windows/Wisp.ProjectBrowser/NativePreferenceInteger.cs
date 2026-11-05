using System.Globalization;

namespace Wisp.ProjectBrowser;

/// <summary>Integer drafts must remain valid before they can replace a saved setting.</summary>
public static class NativePreferenceInteger
{
    public static string Step(string text, int direction, int minimum, int maximum)
        => StepBounded(text, direction, minimum, maximum);

    // Workflow costs have the full u64 wire range. Decimal represents every such
    // integer exactly, including the increment immediately below ulong.MaxValue.
    public static string StepUnsigned(string text, int direction, ulong minimum, ulong maximum)
        => StepBounded(text, direction, minimum, maximum);

    private static string StepBounded(string text, int direction, decimal minimum, decimal maximum)
    {
        if (direction is not (1 or -1)) return text;
        var parsed = decimal.TryParse(text, NumberStyles.Float, CultureInfo.InvariantCulture, out var value);
        if (!parsed) value = 0;
        // Clamp before arithmetic, so extreme pasted decimals cannot overflow.
        if (value > maximum) return direction < 0 ? maximum.ToString(CultureInfo.InvariantCulture) : text;
        if (value < minimum) return direction > 0 || !parsed ? minimum.ToString(CultureInfo.InvariantCulture) : text;
        var next = direction > 0 ? decimal.Floor(value) + 1 : decimal.Ceiling(value) - 1;
        return Math.Clamp(next, minimum, maximum).ToString(CultureInfo.InvariantCulture);
    }
}
