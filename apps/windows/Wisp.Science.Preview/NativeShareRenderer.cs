using System.Globalization;
using System.Text;
using System.Text.Json;
using Microsoft.UI.Xaml.Controls;
using Microsoft.Web.WebView2.Core;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>Local, single-use artifact renderer. No host objects, network or navigation.</summary>
internal sealed class NativeShareRenderer : Grid, IDisposable
{
    private const string Origin = "wisp-share.local";
    private const string Page = "https://wisp-share.local/index.html";
    private readonly WebView2 browser = new() { IsTabStop = false, IsHitTestVisible = false };
    private readonly TaskCompletionSource<string> result = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private bool disposed;

    public NativeShareRenderer() => Children.Add(browser);

    public async Task<NativeShareFile> RenderAsync(NativeShareDocument document, WispDesign design, CancellationToken token)
    {
        using var timeout = CancellationTokenSource.CreateLinkedTokenSource(token);
        timeout.CancelAfter(TimeSpan.FromSeconds(45));
        using var registration = timeout.Token.Register(() => result.TrySetCanceled(timeout.Token));
        var stamp = DateTime.UtcNow.ToString("yyyy-MM-dd", CultureInfo.InvariantCulture);
        var variables = new Dictionary<string, string>();
        foreach (var key in new[] { "bg-app", "bg-elev", "bg-sunken", "text", "text-muted", "text-faint", "border", "border-strong", "clay", "clay-strong" })
            variables["--" + key] = design.ColorText(key);
        variables["--bg-panel"] = design.ColorText("bg-panel");
        variables["--font-ui"] = design.Typography.UiFamily + ", 'Microsoft YaHei UI', sans-serif";
        variables["--font-response"] = design.Localization.Locale == "zh" ? variables["--font-ui"] : "'Source Serif 4', Georgia, serif";
        variables["--font-mono"] = design.Typography.CodeFamily + ", Consolas, monospace";
        variables["--ui-font-size"] = design.Typography.UiSize.ToString(CultureInfo.InvariantCulture) + "px";
        var payload = JsonSerializer.Serialize(new { format = document.Format.ToString().ToLowerInvariant(), html = document.Html,
            width = document.Width, rows = document.Rows.Select(row => new { role = row.Role, text = row.Text }),
            locale = design.Localization.Locale, dark = design.Dark, variables, title = "wisp-science", stamp,
            footer = design.T("share.image_footer"), labels = new { user = design.T("share.role_user"), assistant = design.T("share.role_assistant"), reasoning = design.T("share.role_thinking") } });
        _ = InitializeAsync(payload);
        try
        {
            var output = await result.Task;
            token.ThrowIfCancellationRequested();
            var png = document.Format == NativeShareFormat.Png;
            var bytes = png ? Convert.FromBase64String(output) : Encoding.UTF8.GetBytes(output);
            if (png && (bytes.Length < 8 || !bytes.AsSpan(0, 8).SequenceEqual(new byte[] { 137, 80, 78, 71, 13, 10, 26, 10 })))
                throw new InvalidDataException(design.L("PNG 图像生成失败", "PNG rendering failed"));
            var extension = png ? ".png" : ".html";
            return new("wisp-share-" + stamp + extension, extension, bytes);
        }
        catch (OperationCanceledException) when (!token.IsCancellationRequested)
        { throw new TimeoutException(design.L("导出渲染超时，请重试。", "Export rendering timed out. Please try again.")); }
    }
    private async Task InitializeAsync(string payload)
    {
        try
        {
            var environment = await CoreWebView2Environment.CreateWithOptionsAsync(null,
                Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "WispScience", "NativeWebView2"), null);
            if (disposed) return;
            await browser.EnsureCoreWebView2Async(environment);
            if (disposed) return;
            var core = browser.CoreWebView2;
            core.Settings.AreHostObjectsAllowed = false; core.Settings.AreDevToolsEnabled = false;
            core.Settings.AreDefaultContextMenusEnabled = false; core.Settings.IsStatusBarEnabled = false;
            core.SetVirtualHostNameToFolderMapping(Origin, Path.Combine(AppContext.BaseDirectory, "Assets", "Share"), CoreWebView2HostResourceAccessKind.DenyCors);
            core.AddWebResourceRequestedFilter("*", CoreWebView2WebResourceContext.All);
            core.WebResourceRequested += (_, e) =>
            {
                if (!Uri.TryCreate(e.Request.Uri, UriKind.Absolute, out var uri) || uri.Scheme != "https" || uri.Host != Origin || !uri.IsDefaultPort)
                    e.Response = environment.CreateWebResourceResponse(new Windows.Storage.Streams.InMemoryRandomAccessStream(), 403, "Local resources only", "Content-Type: text/plain");
            };
            core.NavigationStarting += (_, e) => { if (e.Uri != Page) e.Cancel = true; };
            core.NewWindowRequested += (_, e) => e.Handled = true;
            core.PermissionRequested += (_, e) => e.State = CoreWebView2PermissionState.Deny;
            core.ProcessFailed += (_, _) => result.TrySetException(new IOException("Export renderer stopped"));
            core.NavigationCompleted += (_, e) => { if (!e.IsSuccess) result.TrySetException(new IOException("Export renderer: " + e.WebErrorStatus)); };
            core.WebMessageReceived += (_, e) =>
            {
                if (disposed || e.Source != Page) return;
                try
                {
                    using var message = JsonDocument.Parse(e.WebMessageAsJson);
                    var data = message.RootElement;
                    switch (data.GetProperty("type").GetString())
                    {
                        case "ready": core.PostWebMessageAsJson(payload); break;
                        case "done": result.TrySetResult(data.GetProperty("output").GetString() ?? ""); break;
                        case "error": result.TrySetException(new IOException(data.GetProperty("error").GetString())); break;
                    }
                }
                catch (Exception ex) { result.TrySetException(ex); }
            };
            core.Navigate(Page);
        }
        catch (Exception ex) { if (!disposed) result.TrySetException(ex); }
    }
    public void Dispose()
    {
        if (disposed) return; disposed = true;
        result.TrySetCanceled(); Children.Clear(); browser.Close();
    }
}
