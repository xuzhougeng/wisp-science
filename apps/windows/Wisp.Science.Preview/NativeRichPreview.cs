using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.Web.WebView2.Core;

namespace Wisp.Science.Preview;

/// <summary>Read-only local renderer. Payloads are JSON messages, never HTML or paths.
/// Reparenting within a render keeps the view; unmounting closes its browser resources.</summary>
internal sealed class NativeRichPreview : UserControl, IDisposable
{
    private const string Page = "https://wisp-preview.local/index.html";
    private readonly Grid root = new();
    private readonly TextBox fallback;
    private readonly string payload;
    private readonly bool math, inline;
    private WebView2? view;
    private int generation;
    private bool disposed, ready;

    public NativeRichPreview(WispDesign design, string kind, string value, string label, bool display = true)
    {
        math = kind == "math"; inline = math && !display;
        Height = math ? 90 : 460;
        if (inline) Width = 220;
        string Color(string token) { var c = design.Brush(token).Color; return $"#{c.R:X2}{c.G:X2}{c.B:X2}"; }
        payload = JsonSerializer.Serialize(new { kind, text = math ? value : null, base64 = math ? null : value, display,
            background = Color("bg-app"), foreground = Color("text"), fontSize = design.FontSize(14) });
        fallback = new TextBox { Text = math ? value : label + "\n正在加载 PDF…", IsReadOnly = true,
            AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(math) };
        root.Children.Add(fallback); Content = root;
        Loaded += async (_, _) => await LoadAsync();
        Unloaded += (_, _) => DispatcherQueue.TryEnqueue(() => { if (!IsLoaded) Release(); });
    }

    private async Task LoadAsync()
    {
        if (disposed || view != null) return;
        var current = ++generation;
        var browser = new WebView2(); view = browser; ready = false;
        root.Children.Insert(0, browser); fallback.Visibility = Visibility.Visible;
        try
        {
            var environment = await CoreWebView2Environment.CreateWithOptionsAsync(null,
                Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "WispScience", "NativeWebView2"), null);
            if (current != generation) return;
            await browser.EnsureCoreWebView2Async(environment);
            if (current != generation) return;
            var core = browser.CoreWebView2;
            core.Settings.AreHostObjectsAllowed = false;
            core.Settings.AreDevToolsEnabled = false;
            core.Settings.AreDefaultContextMenusEnabled = false;
            core.Settings.IsStatusBarEnabled = false;
            core.SetVirtualHostNameToFolderMapping("wisp-preview.local", Path.Combine(AppContext.BaseDirectory, "Assets", "RichPreview"), CoreWebView2HostResourceAccessKind.DenyCors);
            core.NavigationStarting += (_, e) => { if (e.Uri != Page) e.Cancel = true; };
            core.NewWindowRequested += (_, e) => e.Handled = true;
            core.PermissionRequested += (_, e) => e.State = CoreWebView2PermissionState.Deny;
            core.WebMessageReceived += (_, e) =>
            {
                if (current != generation || e.Source != Page) return;
                try
                {
                    using var json = JsonDocument.Parse(e.WebMessageAsJson);
                    var data = json.RootElement;
                    if (data.GetProperty("type").GetString() == "ready")
                    {
                        ready = true; fallback.Visibility = Visibility.Collapsed; core.PostWebMessageAsJson(payload);
                    }
                    else if (math && data.GetProperty("type").GetString() == "size")
                    {
                        Height = Math.Clamp(data.GetProperty("height").GetDouble(), 32, 400);
                        if (inline) Width = Math.Clamp(data.GetProperty("width").GetDouble(), 32, 600);
                    }
                }
                catch (Exception ex) { Fail("预览消息无效：" + ex.Message); }
            };
            core.ProcessFailed += (_, _) => { if (current == generation) Fail("预览进程已停止。"); };
            core.NavigationCompleted += (_, e) => { if (current == generation && !e.IsSuccess) Fail("预览加载失败：" + e.WebErrorStatus); };
            core.Navigate(Page);
            await Task.Delay(TimeSpan.FromSeconds(10));
            if (current == generation && !ready) Fail("预览未能就绪。");
        }
        catch (Exception ex) { if (current == generation) Fail("无法加载预览：" + ex.Message); }
    }

    private void Fail(string message)
    {
        if (!math) fallback.Text = message;
        fallback.Visibility = Visibility.Visible;
        if (view != null) view.Visibility = Visibility.Collapsed;
    }

    private void Release()
    {
        generation++; ready = false;
        if (view != null) { var previous = view; view = null; root.Children.Remove(previous); previous.Close(); }
    }
    public void Dispose() { if (disposed) return; disposed = true; Release(); }
}
