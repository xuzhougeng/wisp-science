using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeCenterFiles : UserControl, IDisposable
{
    private readonly WorkspaceArtifactTabs tabs;
    private readonly WispDesign design;
    private readonly Func<string, Task<string?>>? pickDownload;
    private readonly Func<NativeArtifactDocumentQuote, bool>? quoteDocument;
    private readonly CancellationTokenSource lifetime = new();
    public NativeCenterTabs Header { get; }
    private string conversationTitle;
    private readonly Grid body = new();
    private readonly NativeArtifactSurface surface;
    private readonly StackPanel file = new() { Spacing = 8, Margin = new(16, 0, 16, 12) };
    private readonly TextBlock status;
    private readonly Button download;
    private FrameworkElement? conversation;
    private bool disposed;
    public NativeCenterFiles(WorkspaceArtifactTabs tabs, WispDesign design, Action escape, Func<string, Task<string?>>? pickDownload,
        Func<NativeArtifactDocumentQuote, bool>? quoteDocument = null)
    {
        this.tabs = tabs; this.design = design; this.pickDownload = pickDownload; this.quoteDocument = quoteDocument;
        Header = new(design); conversationTitle = design.T("center.new_session");
        surface = new(design, escape, quoteDocument == null ? null : QuoteDocument);
        Content = body;
        var actions = new NativeActionWrap { Gap = 8 };
        download = design.ToolButton(design.L("下载", "Download"), "download");
        download.Click += async (_, _) => { if (tabs.Selected is { } selected && this.pickDownload != null) await selected.Reader.ExportAsync(this.pickDownload, NativeArtifactFileCopy.CopyAsync, lifetime.Token); };
        status = design.Text("", 12); status.Foreground = design.Brush("text-faint");
        actions.Children.Add(download); actions.Children.Add(status); file.Children.Add(actions); file.Children.Add(surface);
        tabs.Changed += Refresh; SizeChanged += (_, _) => Present();
        design.BindTypography(this); Refresh();
    }
    public bool HandleEscape() => tabs.Selected != null && surface.HandleEscape();
    private bool QuoteDocument(NativePanelArtifact item, NativePanelFileContent content, NativeDocumentSelection selection)
    {
        var selected = tabs.Selected;
        if (disposed || !IsLoaded || selected?.Reader.DocumentQuote(item, content, selection) is not { } request
            || quoteDocument?.Invoke(request) != true) return false;
        if (selection.Jump) DispatcherQueue.TryEnqueue(() => { if (!disposed && ReferenceEquals(tabs.Selected, selected)) tabs.Select(null); });
        return true;
    }
    public void SetConversationTitle(string title) { conversationTitle = title; RefreshHeader(); }
    private void RefreshHeader() => Header.Present(conversationTitle, tabs.Items, tabs.SelectedKey, () => tabs.Select(null), tabs.Select, tabs.Close);
    public void AttachConversation(FrameworkElement? value)
    {
        conversation = value; Refresh();
    }
    private void Refresh()
    {
        if (disposed) return;
        RefreshHeader();
        body.Children.Clear();
        if (tabs.Selected == null)
        {
            if (conversation != null)
            {
                if (conversation.Parent is Panel parent) parent.Children.Remove(conversation);
                body.Children.Add(conversation);
            }
        }
        else body.Children.Add(file);
        Present();
    }
    private void Present()
    {
        if (disposed || tabs.Selected is not { } selected || ActualWidth <= 0 || ActualHeight <= 0) return;
        var reader = selected.Reader;
        download.IsEnabled = pickDownload != null && !reader.ExportBusy;
        status.Text = reader.ExportError ?? (reader.ExportedPath is { } path ? design.L("已保存：", "Saved: ") + path : selected.Item.Path);
        surface.Present(selected.Item, reader.Content, reader.LoadingContent, reader.ContentError, Math.Max(160, ActualWidth - 32), Math.Max(100, ActualHeight - 66));
    }
    public void Dispose()
    {
        if (disposed) return; disposed = true; lifetime.Cancel(); tabs.Changed -= Refresh; tabs.Dispose(); surface.Dispose(); body.Children.Clear(); lifetime.Dispose();
    }
}
