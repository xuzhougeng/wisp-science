using System.Text;

namespace Wisp.ProjectBrowser;

/// <summary>Local CSS import; remote-load stripping remains in the shared Rust save command.</summary>
public static class NativeCustomTheme
{
    // src-tauri/src/configure.rs and ui/src/app_support/prefs.rs use the same byte limit.
    public const int MaxBytes = 64_000;
    public static int ByteCount(string css) => Encoding.UTF8.GetByteCount(css);

    public static async Task<string> ReadFileAsync(string path, CancellationToken token = default)
    {
        await using var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.Read,
            4096, FileOptions.Asynchronous | FileOptions.SequentialScan);
        return await ReadAsync(file, token);
    }

    public static async Task<string> ReadAsync(Stream stream, CancellationToken token = default)
    {
        using var reader = new StreamReader(stream, Encoding.UTF8, true, 4096, leaveOpen: true);
        var output = new StringBuilder(); var bytes = 0; var carry = "";
        var buffer = new char[2048];
        while (true)
        {
            token.ThrowIfCancellationRequested();
            var count = await reader.ReadAsync(buffer.AsMemory(), token);
            var chunk = carry + new string(buffer, 0, count); carry = "";
            // Preserve a surrogate pair split across StreamReader buffers.
            if (count > 0 && chunk.Length > 0 && char.IsHighSurrogate(chunk[^1]))
            { carry = chunk[^1].ToString(); chunk = chunk[..^1]; }
            foreach (var rune in chunk.EnumerateRunes())
            {
                if (rune.Value == 0) continue;
                if (bytes + rune.Utf8SequenceLength > MaxBytes) return output.ToString();
                output.Append(rune.ToString()); bytes += rune.Utf8SequenceLength;
            }
            if (count == 0) return output.ToString();
        }
    }
}
