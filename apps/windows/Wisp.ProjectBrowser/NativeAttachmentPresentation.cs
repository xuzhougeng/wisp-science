namespace Wisp.ProjectBrowser;

public sealed record NativeAttachmentUpload(Guid Id, string Name, string? Error = null)
{
    public bool Uploading => Error == null;
    public string MetaKey => Uploading ? "composer.uploading" : Error!.Contains("byte limit", StringComparison.OrdinalIgnoreCase)
        ? "composer.upload_too_large" : "composer.upload_failed";
}

public static class NativeAttachmentPresentation
{
    public static bool ValidPath(string path) => path.StartsWith("uploads/", StringComparison.Ordinal)
        && !path.Contains('\\') && !path.Contains(':') && !path.Any(char.IsControl)
        && path.Split('/').All(part => part is not ("" or "." or ".."));
    public static bool Image(string path) => Path.GetExtension(path).ToLowerInvariant()
        is ".png" or ".jpg" or ".jpeg" or ".gif" or ".webp" or ".svg";
}
