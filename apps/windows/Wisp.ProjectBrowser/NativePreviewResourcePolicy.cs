namespace Wisp.ProjectBrowser;

public static class NativePreviewResourcePolicy
{
    public static bool Allows(string value) => Uri.TryCreate(value, UriKind.Absolute, out var uri)
        && (uri.Scheme is "data" or "blob"
            || uri.Scheme == "https" && uri.Host == "wisp-preview.local" && uri.IsDefaultPort && uri.UserInfo.Length == 0);
}
