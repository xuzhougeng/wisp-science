using Windows.ApplicationModel.DataTransfer;
using Windows.Graphics.Imaging;
using Windows.Storage;

namespace Wisp.Science.Preview;

/// <summary>Owns only a newly created temporary PNG until the host finishes copying it.</summary>
internal sealed class NativeClipboardImage : IDisposable
{
    public string Path { get; }
    private readonly string directory;
    private NativeClipboardImage(string directory, string path) { this.directory = directory; Path = path; }
    public static async Task<NativeClipboardImage> CaptureAsync(DataPackageView data, CancellationToken token)
    {
        token.ThrowIfCancellationRequested();
        var reference = await data.GetBitmapAsync();
        using var source = await reference.OpenReadAsync();
        const ulong limit = 100 * 1024 * 1024;
        if (source.Size > limit) throw new IOException("Image exceeds 104857600 byte limit");
        var decoder = await BitmapDecoder.CreateAsync(source);
        if ((ulong)decoder.PixelWidth * decoder.PixelHeight * 4 > limit) throw new IOException("Decoded image exceeds 104857600 byte limit");
        token.ThrowIfCancellationRequested();
        using var bitmap = await decoder.GetSoftwareBitmapAsync(BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied);
        var directory = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "wisp-paste-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(directory);
        var image = new NativeClipboardImage(directory, System.IO.Path.Combine(directory, $"pasted_image_{DateTime.UtcNow:yyyyMMddHHmmss}_1.png"));
        try
        {
            var folder = await StorageFolder.GetFolderFromPathAsync(directory);
            var file = await folder.CreateFileAsync(System.IO.Path.GetFileName(image.Path), CreationCollisionOption.FailIfExists);
            using var destination = await file.OpenAsync(FileAccessMode.ReadWrite);
            var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.PngEncoderId, destination);
            encoder.SetSoftwareBitmap(bitmap);
            await encoder.FlushAsync();
            token.ThrowIfCancellationRequested();
            return image;
        }
        catch { image.Dispose(); throw; }
    }
    public void Dispose()
    {
        // Exact owned paths only; never recurse into the shared temporary folder.
        try { File.Delete(Path); } catch (IOException) { } catch (UnauthorizedAccessException) { }
        try { Directory.Delete(directory); } catch (IOException) { } catch (UnauthorizedAccessException) { }
    }
}
