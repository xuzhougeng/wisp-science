using System.Text;
using Wisp.ProjectBrowser;

internal static class NativeCustomThemeTests
{
    public static async Task RunAsync()
    {
        const string css = "/* 中文 🧪 */\n:root { --clay: #2563eb; }\r\n";
        Check(NativeCustomTheme.ByteCount(css) == Encoding.UTF8.GetByteCount(css)
            && NativeCustomTheme.ByteCount(css) > css.Length, "theme size is UTF-8 bytes, not UTF-16 characters");
        Check(await Read(css) == css, "import preserves multiline CSS and non-ASCII comments");
        Check(await Read("\0" + css + "\0") == css, "import omits NUL like shared appearance persistence");
        var split = new string('x', 2047) + "🧪" + "中文";
        Check(await Read(split) == split, "surrogate pairs crossing read buffers survive");
        var limit = new string('x', NativeCustomTheme.MaxBytes - 2);
        Check(await Read(limit + "中文") == limit, "64k UTF-8 boundary never splits a Chinese scalar");
        Check(await Read(new string('x', NativeCustomTheme.MaxBytes - 4) + "🧪suffix") == new string('x', NativeCustomTheme.MaxBytes - 4) + "🧪",
            "a four-byte scalar may exactly fill the shared limit");
        Check(await Read("\uFEFF" + css) == css, "UTF-8 file BOM is not inserted into the stylesheet");
        using var cancelled = new CancellationTokenSource(); cancelled.Cancel();
        try { using var stream = new MemoryStream(Encoding.UTF8.GetBytes(css)); await NativeCustomTheme.ReadAsync(stream, cancelled.Token); throw new Exception("Expected cancellation"); }
        catch (OperationCanceledException) { }
    }
    private static async Task<string> Read(string text)
    { using var stream = new MemoryStream(Encoding.UTF8.GetBytes(text)); return await NativeCustomTheme.ReadAsync(stream); }
    private static void Check(bool value, string message) { if (!value) throw new InvalidOperationException(message); }
}
