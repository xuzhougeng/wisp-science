using System.Runtime.InteropServices;
using Wisp.Science.Preview;

internal static class NativeWindowEscapeTests
{
    public static void Run()
    {
        var settingsOpen = true; var workspaceEscapes = 0;
        Check(NativeWindowEscape.DispatchPages(() => settingsOpen = false, () => workspaceEscapes++), "settings handles the first Escape");
        Check(!settingsOpen && workspaceEscapes == 0, "closing settings preserves the automation form underneath");
        Check(NativeWindowEscape.DispatchPages(null, () => workspaceEscapes++) && workspaceEscapes == 1, "next Escape reaches the retained workspace");
        Check(!NativeWindowEscape.DispatchPages(null, null), "empty page stack yields to the remaining window handlers");
        var layersCalled = new List<string>();
        Action settings = () => layersCalled.Add("settings"), setup = () => layersCalled.Add("setup");
        NativeWindowEscape.DispatchPages(settings, null, () => layersCalled.Add("palette"), setup);
        NativeWindowEscape.DispatchPages(settings, null, null, setup);
        NativeWindowEscape.DispatchPages(settings, null);
        Check(layersCalled.SequenceEqual(new[] { "palette", "setup", "settings" }), "consecutive Escape presses dismiss only the topmost of command palette, setup and settings");
        if (!OperatingSystem.IsWindows()) return;
        // A message-only HWND exercises the native route without focusing any
        // control, showing an app, touching a database, or injecting desktop input.
        var window = CreateWindowEx(0, "STATIC", "", 0, 0, 0, 0, 0, new IntPtr(-3), IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
        if (window == IntPtr.Zero) throw new InvalidOperationException("Cannot create keyboard test window.");
        try
        {
            var layers = new Stack<string>(["settings", "import"]);
            using var hook = new NativeWindowEscape(window, () => layers.TryPop(out _));
            SendMessage(window, 0x0100, 0x41, 0);
            Check(layers.Count == 2, "ordinary keys must not dismiss a surface");
            SendMessage(window, 0x0100, 0x1B, 0);
            Check(layers.Count == 1 && layers.Peek() == "settings", "first Escape closes only the unfocused topmost overlay");
            SendMessage(window, 0x0101, 0x1B, 0);
            Check(layers.Count == 1, "key release must not close the parent");
            hook.Dispose();
            SendMessage(window, 0x0100, 0x1B, 0);
            Check(layers.Count == 1, "disposing a window removes its native keyboard listener");
            var history = true; var palette = true;
            using var searchHook = new NativeWindowEscape(window, () => NativeWindowEscape.DispatchPages(null,
                history ? () => history = false : null, palette ? () => palette = false : null));
            SendMessage(window, 0x0100, 0x1B, 0);
            Check(!palette && history, "immediate Escape closes the workspace palette and preserves the underlying chat search");
            SendMessage(window, 0x0101, 0x1B, 0);
            Check(history, "releasing the same key cannot dismiss chat history");
            SendMessage(window, 0x0100, 0x1B, 0);
            Check(!history, "the next Escape closes the retained chat search");
        }
        finally { DestroyWindow(window); }
    }
    internal static void PressEscapeImmediately(Func<bool> handle)
    {
        if (!OperatingSystem.IsWindows()) { handle(); return; }
        var window = CreateWindowEx(0, "STATIC", "", 0, 0, 0, 0, 0, new IntPtr(-3), IntPtr.Zero, IntPtr.Zero, IntPtr.Zero);
        if (window == IntPtr.Zero) throw new InvalidOperationException("Cannot create keyboard test window.");
        try
        {
            using var hook = new NativeWindowEscape(window, handle);
            SendMessage(window, 0x0100, 0x1B, 0);
            SendMessage(window, 0x0101, 0x1B, 0);
        }
        finally { DestroyWindow(window); }
    }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr CreateWindowEx(uint style, string className, string title, uint windowStyle, int x, int y, int width, int height, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr parameter);
    [DllImport("user32.dll")] private static extern IntPtr SendMessage(IntPtr window, uint message, nuint key, nint detail);
    [DllImport("user32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)] private static extern bool DestroyWindow(IntPtr window);
}
