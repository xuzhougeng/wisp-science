using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeActionWrap : Panel
{
    protected override Size MeasureOverride(Size available)
    {
        foreach (var child in Children) child.Measure(new Size(available.Width, double.PositiveInfinity));
        var slots = Layout(available.Width);
        return new Size(slots.Length == 0 ? 0 : slots.Max(s => s.X + s.Width),
            slots.Length == 0 ? 0 : slots.Max(s => s.Y + s.Height));
    }
    protected override Size ArrangeOverride(Size final)
    {
        var slots = Layout(final.Width);
        for (var i = 0; i < slots.Length; i++)
        { var s = slots[i]; Children[i].Arrange(new Rect(s.X, s.Y, s.Width, s.Height)); }
        return final;
    }
    private NativeActionLayout.Slot[] Layout(double width) => NativeActionLayout.Arrange(
        Children.Select(c => new NativeActionLayout.Size(c.DesiredSize.Width, c.DesiredSize.Height)).ToArray(), width);

    public static void GroupButtons(StackPanel stack)
    {
        for (var i = 0; i < stack.Children.Count; i++)
        {
            if (stack.Children[i] is not Button) continue;
            var end = i;
            while (end < stack.Children.Count && stack.Children[end] is Button) end++;
            if (end - i < 2) continue;
            var actions = new NativeActionWrap();
            for (var n = end - i; n > 0; n--)
            { var child = stack.Children[i]; stack.Children.RemoveAt(i); actions.Children.Add(child); }
            stack.Children.Insert(i, actions);
        }
    }
}
