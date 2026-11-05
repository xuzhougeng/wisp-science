using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>One read-only output preview shared by workspace and bundled examples.</summary>
internal sealed class NativeGeneratedPreview : UserControl, IDisposable
{
    private readonly NativeGeneratedPreviewModel model;
    private readonly WispDesign design;
    private readonly Action close;
    private readonly Func<string, NativeDocumentSelection, bool>? quoteDocument;
    private readonly Grid root = new();
    private readonly TextBlock title, kind;
    private readonly StackPanel body = new() { Spacing = 10 };
    private NativeArtifactSurface? file;
    private NativeRichPreview? rich;
    private bool disposed;
    public NativeGeneratedPreview(NativeGeneratedPreviewModel model, WispDesign design, Action close,
        Func<string, NativeDocumentSelection, bool>? quoteDocument = null)
    {
        this.model = model; this.design = design; this.close = close; this.quoteDocument = quoteDocument;
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new());
        var header = new Grid { Padding = new(12, 8, 12, 8), ColumnSpacing = 8 };
        header.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        kind = design.Text("", 10); kind.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; kind.CharacterSpacing = 40; kind.Foreground = design.Brush("text-faint");
        header.Children.Add(new Border { Child = kind, Padding = new(6, 2, 6, 2), CornerRadius = new(4), BorderThickness = new(1),
            BorderBrush = design.Brush("border"), Background = design.Brush("bg-elev"), VerticalAlignment = VerticalAlignment.Center });
        title = design.Text("", 13); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.VerticalAlignment = VerticalAlignment.Center;
        title.TextTrimming = TextTrimming.CharacterEllipsis; title.TextWrapping = TextWrapping.NoWrap; Grid.SetColumn(title, 1); header.Children.Add(title);
        var dismiss = design.ToolButton(design.T("right.close_preview"), "close"); dismiss.Click += (_, _) => close(); Grid.SetColumn(dismiss, 2); header.Children.Add(dismiss);
        root.Children.Add(new Border { Child = header, BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) });
        var scroll = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch, Padding = new(12) };
        Grid.SetRow(scroll, 1); root.Children.Add(scroll); root.Background = design.Brush("bg-app"); Content = root;
        model.Changed += Render; SizeChanged += (_, _) => Render(); Render();
    }
    public bool HandleEscape()
    {
        if (file?.HandleEscape() == true || rich?.DismissSelection() == true) return true;
        close(); return true;
    }
    private void Render()
    {
        if (disposed) return;
        file?.Dispose(); file = null; rich?.Dispose(); rich = null; body.Children.Clear();
        if (model.Selected is not { } selected) return;
        title.Text = selected.Name; kind.Text = selected.Kind.ToUpperInvariant(); ToolTipService.SetToolTip(title, selected.DisplayPath ?? selected.Name);
        kind.Foreground = design.Brush(selected.Kind == "table" ? "clay-strong" : "text-faint");
        if (kind.Parent is Border badge)
        {
            badge.Background = design.Brush(selected.Kind == "table" ? "clay-soft" : "bg-elev");
            badge.BorderThickness = new(selected.Kind == "table" ? 0 : 1);
        }
        AutomationProperties.SetName(root, selected.Name);
        var height = Math.Max(160, ActualHeight > 0 ? ActualHeight - 76 : 500);
        if (selected.DisplayPath is { } path)
        {
            var hint = design.Text(path, 11); hint.Foreground = design.Brush("text-faint"); hint.FontFamily = design.CodeFont;
            hint.TextWrapping = TextWrapping.Wrap; hint.IsTextSelectionEnabled = true; body.Children.Add(hint);
        }
        if (model.Loading) { body.Children.Add(new ProgressRing { IsActive = true, Width = 28, Height = 28, Margin = new(20) }); return; }
        if (model.Error != null || model.FileUnavailable)
        {
            body.Children.Add(design.Text(model.Error ?? design.L("示例文件未包含在此只读录制中。", "This file is not included in the read-only recording."), 13));
            if (model.Error != null) { var retry = new Button { Content = design.L("重试", "Retry") }; retry.Click += async (_, _) => await model.RetryAsync(); body.Children.Add(retry); }
            return;
        }
        if (selected.Output?.Table is { } table) body.Children.Add(NativeCsvPreview.Create(table, design, height, markdown: true));
        else if (selected.DisplayPath == null && selected.Output?.Text is { } text)
        {
            rich = new NativeRichPreview(design, selected.Kind == "latex" ? "math" : selected.Kind, text, selected.Name, escape: () => HandleEscape()) { MaxHeight = height };
            body.Children.Add(rich);
        }
        else if (model.Content is { } content)
        {
            file = new NativeArtifactSurface(design, () => HandleEscape(), quoteDocument == null ? null : (_, value, selection) =>
            {
                if (disposed || !IsLoaded || model.DocumentQuotePath(selected, value, selection) is not { } source
                    || !quoteDocument(source, selection)) return false;
                if (selection.Jump) DispatcherQueue.TryEnqueue(() =>
                {
                    if (!disposed && ReferenceEquals(model.Selected, selected) && ReferenceEquals(model.Content, value)) close();
                });
                return true;
            }); body.Children.Add(file);
            file.Present(new(selected.Id, selected.Name, selected.Kind, selected.DisplayPath!, null, 0, null), content, false, null, Math.Max(100, ActualWidth - 24), height);
        }
    }
    public void Dispose() { if (disposed) return; disposed = true; model.Changed -= Render; file?.Dispose(); rich?.Dispose(); Content = null; }
}
