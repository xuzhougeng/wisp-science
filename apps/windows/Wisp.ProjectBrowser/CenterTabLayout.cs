namespace Wisp.ProjectBrowser;

/// <summary>CSS center-tabs flex sizing in client DIPs; overflow scrolls horizontally.</summary>
public static class CenterTabLayout
{
    public static double[] Widths(double available, double paneWidth, int fileCount)
    {
        available = double.IsFinite(available) ? Math.Max(0, available) : 0;
        paneWidth = double.IsFinite(paneWidth) ? Math.Max(0, paneWidth) : 0;
        var count = Math.Max(0, fileCount) + 1;
        var maximum = Enumerable.Repeat(220d, count).ToArray();
        maximum[0] = Math.Min(280, paneWidth * .36);
        var minimum = Enumerable.Repeat(72d, count).ToArray();
        minimum[0] = Math.Min(30, maximum[0]);
        var basis = Enumerable.Repeat(140d, count).ToArray(); basis[0] = Math.Min(160, maximum[0]);
        var widths = basis.ToArray();
        var target = Math.Clamp(available - (count - 1) * 4, minimum.Sum(), maximum.Sum());
        for (var pass = 0; pass < count + 1; pass++)
        {
            var delta = target - widths.Sum();
            if (Math.Abs(delta) < .001) break;
            var growing = delta > 0;
            var indices = Enumerable.Range(0, count).Where(i => growing ? widths[i] < maximum[i] : widths[i] > minimum[i]).ToArray();
            var weight = indices.Sum(i => growing ? 1 : basis[i]);
            if (weight <= 0) break;
            foreach (var i in indices)
                widths[i] = Math.Clamp(widths[i] + delta * (growing ? 1 : basis[i]) / weight, minimum[i], maximum[i]);
        }
        return widths;
    }
}
