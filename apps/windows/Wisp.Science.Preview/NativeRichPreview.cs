using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.Web.WebView2.Core;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// <summary>Read-only local renderer. Payloads are JSON messages, never HTML or paths.
/// Reparenting within a render keeps the view; unmounting closes its browser resources.</summary>
internal sealed class NativeRichPreview : UserControl, IDisposable
{
    private const string Page = "https://wisp-preview.local/index.html";
    private readonly Grid root = new();
    private readonly WispDesign design;
    private readonly TextBox fallback;
    private readonly string payload;
    private readonly string kind;
    private readonly string imageError;
    private readonly bool math, inline;
    private double imageWidth, imageHeight;
    private WebView2? view;
    private int generation;
    private bool disposed, ready;
    private bool hasSelection;
    private readonly Func<NativeDocumentSelection, bool>? quote;
    private readonly Action? escape;
    public bool IsPdf => kind == "pdf";

    public NativeRichPreview(WispDesign design, string kind, string value, string label, bool display = true,
        Func<NativeDocumentSelection, bool>? quote = null, Action? escape = null, string? format = null)
    {
        this.design = design;
        this.quote = quote;
        this.kind = kind;
        imageError = design.T("library.read_failed");
        this.escape = escape;
        math = kind == "math"; inline = math && !display;
        Height = math ? 90 : kind == "image" ? 160 : 460;
        SizeChanged += (_, _) => ResizeImage();
        RegisterPropertyChangedCallback(MaxHeightProperty, (_, _) => ResizeImage());
        if (inline) Width = 220;
        string Color(string token) { var c = design.Brush(token).Color; return $"#{c.R:X2}{c.G:X2}{c.B:X2}"; }
        var textual = math || kind is "structure" or "molecule" or "msa" or "fasta";
        var pdfIcons = kind == "pdf" ? new[] { "chevron-left", "chevron-right", "minus", "plus" }.ToDictionary(name => name,
            name => File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", $"icon-{name}.svg")).Replace("#000000", "currentColor")) : null;
        var quoteIcons = quote != null ? new[] { "plus", "chat" }.ToDictionary(name => name,
            name => File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "Assets", $"icon-{name}.svg")).Replace("#000000", "currentColor")) : null;
        var officeLabels = kind is "docx" or "xlsx" or "pptx" ? new {
            loading = design.T("loading"), error = design.T($"preview.{kind}_error"),
            formulaLabel = design.T("preview.xlsx_formula"), truncated = design.T("preview.xlsx_truncated") } : null;
        payload = JsonSerializer.Serialize(new { kind, text = textual ? value : null, base64 = textual ? null : value, display, format, label, imageError,
            background = Color(kind == "image" ? "bg-sunken" : "bg-app"), foreground = Color("text"), fontSize = design.FontSize(14), canQuote = quote != null,
            selectionPopupEnabled = design.SelectionPopupEnabled,
            locale = design.Localization.Locale, pdfIcons, quoteIcons, officeLabels, border = Color("border-strong"), elevated = Color("bg-elev"), muted = Color("text-muted"), hover = Color("surface-hover"),
            shadow = design.Dark ? "0 18px 40px -18px rgba(0,0,0,.45)" : "0 16px 32px -12px rgba(20,20,19,.12)" });
        fallback = new TextBox { AcceptsReturn = true, Text = math ? value : label + "\n" + (kind == "image" ? design.L("正在加载图像…", "Loading image…") : officeLabels?.loading ?? design.L("正在加载文档…", "Loading document…")), IsReadOnly = true,
            TextWrapping = TextWrapping.Wrap, FontFamily = design.Font(math) };
        root.Children.Add(fallback); Content = root;
        Loaded += async (_, _) => await LoadAsync();
        Unloaded += (_, _) => DispatcherQueue.TryEnqueue(() => { if (!IsLoaded) Release(); });
        if (quote != null) design.SelectionPopupChanged += SelectionPopupChanged;
    }

    private void SelectionPopupChanged(bool enabled)
    {
        if (!disposed && ready && view?.CoreWebView2 is { } core)
            core.PostWebMessageAsJson(JsonSerializer.Serialize(new { type = "selection-settings", enabled }));
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
            // Worker realms have their own CSP. Keep their network access local
            // too; document inputs never authorize fetching external resources.
            core.AddWebResourceRequestedFilter("*", CoreWebView2WebResourceContext.All);
            core.WebResourceRequested += (_, e) =>
            {
                if (!NativePreviewResourcePolicy.Allows(e.Request.Uri))
                    e.Response = environment.CreateWebResourceResponse(new Windows.Storage.Streams.InMemoryRandomAccessStream(), 403, "Local preview resources only", "Content-Type: text/plain");
            };
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
                        SelectionPopupChanged(design.SelectionPopupEnabled);
                    }
                    else if (kind == "image" && data.GetProperty("type").GetString() == "image-size")
                    {
                        imageWidth = data.GetProperty("width").GetDouble(); imageHeight = data.GetProperty("height").GetDouble();
                        ResizeImage();
                    }
                    else if (math && data.GetProperty("type").GetString() == "size")
                    {
                        Height = Math.Clamp(data.GetProperty("height").GetDouble(), 32, 400);
                        if (inline) Width = Math.Clamp(data.GetProperty("width").GetDouble(), 32, 600);
                    }
                    else if (!math && data.GetProperty("type").GetString() == "selection")
                        hasSelection = data.TryGetProperty("active", out var active) && active.ValueKind == JsonValueKind.True;
                    else if (!math && data.GetProperty("type").GetString() == "escape") escape?.Invoke();
                    else if (!math && data.GetProperty("type").GetString() == "quote")
                    {
                        var selected = NativeDocumentSelection.Parse(data, kind);
                        var accepted = selected != null && quote?.Invoke(selected) == true;
                        if (current == generation && !disposed)
                            core.PostWebMessageAsJson(JsonSerializer.Serialize(new { type = "quote-result", accepted }));
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
        if (!math) fallback.Text = kind == "image" ? imageError : message;
        fallback.Visibility = Visibility.Visible;
        if (view != null) view.Visibility = Visibility.Collapsed;
    }

    private void Release()
    {
        generation++; ready = false; hasSelection = false;
        if (view != null) { var previous = view; view = null; root.Children.Remove(previous); previous.Close(); }
    }
    private void ResizeImage()
    {
        if (kind != "image" || imageWidth <= 0 || imageHeight <= 0 || ActualWidth <= 0) return;
        Height = NativePreviewImageSize.Height(ActualWidth, imageWidth, imageHeight, double.IsFinite(MaxHeight) ? MaxHeight : imageHeight);
    }
    public void ResizePdf(double height)
    {
        if (!IsPdf) return;
        Height = MaxHeight = Math.Max(1, height);
    }
    public bool HandlePdfKey(int key)
    {
        var direction = NativeArtifactNavigation.PdfDirection(key);
        if (!IsPdf || !ready || disposed || view == null || direction == 0) return false;
        view.CoreWebView2.PostWebMessageAsJson(JsonSerializer.Serialize(new { type = "pdf-page", direction }));
        return true;
    }
    public bool DismissSelection()
    {
        if (!hasSelection || !ready || disposed || view == null) return false;
        hasSelection = false;
        view.CoreWebView2.PostWebMessageAsJson("{\"type\":\"clear-selection\"}");
        return true;
    }
    public void Dispose() { if (disposed) return; disposed = true; design.SelectionPopupChanged -= SelectionPopupChanged; Release(); }
}
