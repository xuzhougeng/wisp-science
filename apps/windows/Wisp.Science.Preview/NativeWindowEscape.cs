using System.Runtime.InteropServices;

namespace Wisp.Science.Preview;

/// <summary>Routes Escape when a foreign picker leaves focus on the top-level HWND
/// instead of its XAML input site. XAML-focused keys still use the root accelerator.</summary>
internal sealed class NativeWindowEscape : IDisposable
{
    private readonly IntPtr window;
    private readonly SubclassProcedure procedure;
    private readonly Func<bool> handle;
    private bool disposed;
    // A palette may cover chat history; settings may cover a retained draft.
    // One keypress dismisses only the topmost surface.
    internal static bool DispatchPages(Action? settings, Action? workspace, Action? palette = null, Action? setup = null)
    {
        var top = palette ?? setup ?? settings ?? workspace;
        if (top == null) return false;
        top(); return true;
    }
    public NativeWindowEscape(IntPtr window, Func<bool> handle)
    {
        this.window = window; this.handle = handle;
        procedure = Dispatch;
        if (!SetWindowSubclass(window, procedure, 1, 0)) throw new InvalidOperationException("Cannot register window keyboard handling.");
    }
    private IntPtr Dispatch(IntPtr hwnd, uint message, nuint key, nint detail, nuint id, nuint data)
    {
        // A consumed key never reaches DefSubclassProc or the XAML accelerator,
        // so one press dismisses only the topmost registered surface.
        if (!disposed && message == 0x0100 && key == 0x1B && handle()) return IntPtr.Zero;
        return DefSubclassProc(hwnd, message, key, detail);
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; RemoveWindowSubclass(window, procedure, 1);
    }
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    private delegate IntPtr SubclassProcedure(IntPtr window, uint message, nuint key, nint detail, nuint id, nuint data);
    [DllImport("comctl32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)] private static extern bool SetWindowSubclass(IntPtr window, SubclassProcedure procedure, nuint id, nuint data);
    [DllImport("comctl32.dll")]
    [return: MarshalAs(UnmanagedType.Bool)] private static extern bool RemoveWindowSubclass(IntPtr window, SubclassProcedure procedure, nuint id);
    [DllImport("comctl32.dll")] private static extern IntPtr DefSubclassProc(IntPtr window, uint message, nuint key, nint detail);
}
