using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;

namespace Wisp.Science.Preview;

/// <summary>Text menus participate in the settings owner's window-level Escape path.</summary>
internal sealed class SettingsTextMenus : IDisposable
{
    private readonly List<FlyoutBase> open = [];
    public void Register(TextBox input)
    {
        var menu = new TextCommandBarFlyout();
        Register(menu);
        input.ContextFlyout = menu;
    }
    public void Register(FlyoutBase menu)
    {
        menu.Opened += (_, _) => { open.Remove(menu); open.Add(menu); };
        menu.Closed += (_, _) => open.Remove(menu);
    }
    public bool CloseTop()
    {
        if (open.LastOrDefault() is not { } menu) return false;
        open.Remove(menu); menu.Hide(); return true;
    }
    public void Dispose() { while (CloseTop()) { } }
}
