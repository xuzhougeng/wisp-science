using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

// Like .composer-actions: keep the leading tools and send controls at their
// natural size, and let the two text pickers share any shortage of space.
internal sealed class NativeComposerToolbar(UIElement leading, UIElement modelPicker, UIElement effortPicker) : Panel
{
    private double[] natural = [];
    private bool compact;
    public bool Compact
    {
        get => compact;
        set { if (compact != value) { compact = value; InvalidateMeasure(); } }
    }

    protected override Size MeasureOverride(Size available)
    {
        natural = new double[Children.Count];
        for (var i = 0; i < Children.Count; i++)
        {
            Children[i].Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
            natural[i] = Children[i].DesiredSize.Width;
        }
        return Layout(available.Width, false);
    }

    protected override Size ArrangeOverride(Size final) { Layout(final.Width, true); return final; }

    private Size Layout(double available, bool arrange)
    {
        var trailing = Children.Where(c => c != leading && c.Visibility == Visibility.Visible).ToArray();
        double WidthOf(UIElement c) => c.Visibility == Visibility.Visible ? natural[Children.IndexOf(c)] : 0;
        var left = WidthOf(leading);
        var between = compact ? 5 : 12;
        var gaps = Math.Max(0, trailing.Length - 1) * (compact ? 4 : 6);
        var fixedWidth = gaps + trailing.Where(c => c != modelPicker && c != effortPicker).Sum(WidthOf);
        var model = WidthOf(modelPicker); var effort = WidthOf(effortPicker);
        var width = double.IsFinite(available) ? available : left + between + fixedWidth + model + effort;
        var slots = PreviewLayout.FitComposerPickers(width, left + between, fixedWidth, model, effort);
        double Assigned(UIElement c) => c == modelPicker ? slots.Model : c == effortPicker ? slots.Effort : WidthOf(c);
        if (!arrange)
            foreach (var c in trailing)
                if (c == modelPicker || c == effortPicker) c.Measure(new Size(Assigned(c), double.PositiveInfinity));
        var leftHeight = leading.DesiredSize.Height;
        var rightHeight = trailing.Select(c => c.DesiredSize.Height).DefaultIfEmpty(0).Max();
        var row = Math.Max(leftHeight, rightHeight);
        if (arrange)
        {
            leading.Arrange(new Rect(0, slots.Wrap ? 0 : (row - leftHeight) / 2, left, leftHeight));
            var x = Math.Max(0, width - gaps - trailing.Sum(Assigned));
            foreach (var c in trailing)
            {
                var y = slots.Wrap ? leftHeight + 8 + (rightHeight - c.DesiredSize.Height) / 2 : (row - c.DesiredSize.Height) / 2;
                c.Arrange(new Rect(x, y, Assigned(c), c.DesiredSize.Height)); x += Assigned(c) + (compact ? 4 : 6);
            }
        }
        return new Size(width, slots.Wrap ? leftHeight + 8 + rightHeight : row);
    }
}
