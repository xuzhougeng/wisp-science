using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Windows.Foundation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

// A proxy row has one growing field and an auto margin before Clear. A Grid
// cannot put Save alone on the next line while leaving Clear on the first.
internal sealed class NativeNetworkProxyControls : Panel
{
    private readonly ComboBox mode;
    private readonly TextBox address;
    private readonly Button clear, save;
    public NativeNetworkProxyControls(ComboBox mode, TextBox address, Button clear, Button save)
    {
        this.mode = mode; this.address = address; this.clear = clear; this.save = save;
        Children.Add(mode); Children.Add(address); Children.Add(clear); Children.Add(save);
    }
    private bool Custom => address.Visibility == Visibility.Visible;
    private static NativeActionLayout.Size Desired(UIElement child) => new(child.DesiredSize.Width, child.DesiredSize.Height);
    private NativeActionLayout.Slot[] Slots(double width) => NativeSettingsLayout.NetworkProxySlots(width, Desired(mode),
        Custom ? Desired(address) : null, Desired(clear), Desired(save));
    protected override Size MeasureOverride(Size available)
    {
        foreach (var child in Children)
            child.Measure(new Size(child == address ? Math.Min(available.Width, NativeSettingsLayout.NetworkAddressBasis) : available.Width, double.PositiveInfinity));
        var slots = Slots(available.Width);
        return new(double.IsFinite(available.Width) ? available.Width : slots.Max(s => s.X + s.Width), slots.Max(s => s.Y + s.Height));
    }
    protected override Size ArrangeOverride(Size final)
    {
        var slots = Slots(final.Width);
        var i = 0;
        foreach (var child in Children)
        {
            if (child.Visibility == Visibility.Collapsed) continue;
            var s = slots[i++]; child.Arrange(new Rect(s.X, s.Y, s.Width, s.Height));
        }
        return final;
    }
}
