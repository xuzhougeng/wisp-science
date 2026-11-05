using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Data;
using Microsoft.UI.Input;
using Windows.System;
using Windows.UI.Core;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using Windows.ApplicationModel.DataTransfer;

namespace Wisp.Science.Preview;

internal sealed class NativeArtifactDialog : IDisposable
{
    private readonly WorkspaceArtifactModel model;
    private readonly WispDesign design;
    private readonly ArtifactContentDialog dialog;
    private readonly TextBlock title;
    private readonly Button previous, next, copy, edit, download, center, star, retry;
    private readonly NativeArtifactSurface figure;
    private NativeCodeView? code;
    private readonly NativeArtifactEditor editor = new();
    private readonly Func<string, bool>? insertDraft;
    private readonly Func<NativeArtifactDocumentQuote, bool>? quoteDocument;
    private readonly Func<string, Task<string?>>? pickDownload;
    private readonly TextBlock actionStatus;
    private readonly XamlRoot root;
    private readonly StackPanel layout;
    private readonly Grid header;
    private readonly Border figureBox, tabBorder;
    private readonly ScrollViewer figureScroll, detailScroll;
    public string? CenterRequestedId { get; private set; }
    public string? InputRequestedPath { get; private set; }
    private readonly StackPanel details = new() { Spacing = 6 };
    private readonly Dictionary<string, Button> tabs = [];
    private readonly CancellationTokenSource lifetime = new();
    private readonly List<FlyoutBase> openMenus = [];
    private string tab = "code", selected = "";
    private double detailHeight;
    private bool disposed, fitting, closeRequested;
    public NativeArtifactDialog(WorkspaceArtifactModel model, WispDesign design, XamlRoot root,
        Func<string, Task<string?>>? pickDownload = null, Func<string, bool>? insertDraft = null, bool canOpenCenter = false,
        Func<NativeArtifactDocumentQuote, bool>? quoteDocument = null)
    {
        this.model = model; this.design = design; this.root = root;
        this.pickDownload = pickDownload; this.insertDraft = insertDraft; this.quoteDocument = quoteDocument;
        figure = new(design, () => HandleEscape(), quoteDocument == null ? null : QuoteDocument);
        detailHeight = root.Size.Height * .3;
        layout = new StackPanel { Spacing = 10 };
        header = new Grid { ColumnSpacing = 8 }; header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        title = design.Text("", 14); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.TextTrimming = TextTrimming.CharacterEllipsis; title.TextWrapping = TextWrapping.NoWrap; title.VerticalAlignment = VerticalAlignment.Center;
        header.Children.Add(title);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        previous = design.ToolButton(design.L("上一张图片", "Previous image"), "chevron-left"); previous.Click += async (_, _) => await model.NavigateAsync(-1, lifetime.Token);
        next = design.ToolButton(design.L("下一张图片", "Next image"), "chevron-right"); next.Click += async (_, _) => await model.NavigateAsync(1, lifetime.Token);
        var close = design.ToolButton(design.T("right.close"), "close"); close.Click += (_, _) => Hide();
        center = design.ToolButton(design.L("在主区域打开", "Open in main area"), "expand"); center.IsEnabled = canOpenCenter;
        center.Click += (_, _) => { CenterRequestedId = model.Selected?.Id; Hide(); };
        download = design.ToolButton(design.L("下载", "Download"), "download"); download.IsEnabled = pickDownload != null;
        download.Click += async (_, _) => { if (this.pickDownload != null) await model.ExportAsync(this.pickDownload, NativeArtifactFileCopy.CopyAsync, lifetime.Token); };
        star = design.ToolButton(design.T("library.add"), "star");
        star.Click += async (_, _) => await model.ToggleLibraryAsync(lifetime.Token);
        actions.Children.Add(previous); actions.Children.Add(next); actions.Children.Add(star); actions.Children.Add(center); actions.Children.Add(download); actions.Children.Add(close); Grid.SetColumn(actions, 1); header.Children.Add(actions); layout.Children.Add(header);
        actionStatus = Note(""); actionStatus.Visibility = Visibility.Collapsed; layout.Children.Add(actionStatus);
        retry = new Button { Content = design.L("重试", "Retry"), Visibility = Visibility.Collapsed, HorizontalAlignment = HorizontalAlignment.Left };
        design.ActionButton(retry);
        retry.Click += async (_, _) => { if (!model.LoadingContent && !model.LoadingProvenance && model.Selected is { } item) await model.OpenAsync(item.Id, lifetime.Token); };
        layout.Children.Add(retry);
        figureScroll = new ScrollViewer { Content = figure, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        figureBox = new Border { Child = figureScroll, Background = design.Brush("bg-sunken"), Padding = new(8), CornerRadius = new(10) };
        layout.Children.Add(figureBox);
        var tabRow = new Grid(); tabRow.ColumnDefinitions.Add(new()); tabRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var tabButtons = new NativeActionWrap { Gap = 6 };
        foreach (var key in new[] { "code", "log", "inputs", "env" })
        {
            var caption = design.T("artifact.tab." + key);
            var label = design.Text(caption, 12); label.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            var button = new Button { Content = label, Padding = new(10, 6, 10, 6), MinWidth = 0, MinHeight = 0, CornerRadius = new(5) };
            AutomationProperties.SetName(button, caption); button.Click += (_, _) => { tab = key; RenderDetails(); };
            tabs[key] = button; tabButtons.Children.Add(button);
        }
        tabRow.Children.Add(tabButtons); copy = design.ToolButton(design.L("复制代码", "Copy code"), "copy");
        copy.Click += (_, _) => { if (model.Provenance is { } value) { var data = new DataPackage(); data.SetText(value.Code); Clipboard.SetContent(data); } };
        edit = design.ToolButton(design.L("编辑代码重跑", "Edit code and re-run"), "edit");
        edit.Click += (_, _) => { editor.Toggle(model.Provenance?.Code ?? ""); RenderDetails(); };
        var codeActions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 }; codeActions.Children.Add(copy); codeActions.Children.Add(edit);
        Grid.SetColumn(codeActions, 1); tabRow.Children.Add(codeActions);
        tabBorder = new Border { Child = tabRow, BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1), Padding = new(0, 0, 0, 8) };
        layout.Children.Add(tabBorder);
        detailScroll = new ScrollViewer { Content = details, MaxHeight = detailHeight, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        layout.Children.Add(detailScroll);
        dialog = new ArtifactContentDialog { XamlRoot = root, Content = layout, RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light, DefaultButton = ContentDialogButton.None };
        dialog.Resources["ContentDialogBackground"] = design.Brush("bg-elev");
        dialog.Resources["ContentDialogPadding"] = new Thickness(22);
        dialog.Resources["ContentDialogMinWidth"] = dialog.Resources["ContentDialogMinHeight"] = 0d;
        dialog.Resources["ContentDialogBorderWidth"] = new Thickness(1);
        dialog.BorderBrush = design.Brush("border-strong"); dialog.CornerRadius = new(16);
        model.Changed += Refresh;
        root.Changed += RootChanged; design.TypographyChanged += TypographyChanged;
        details.SizeChanged += (_, _) => Fit(); Fit();
        // Read-only provenance is not a text-entry surface. The preview event
        // must see arrows before the native TextBox consumes them for its caret.
        dialog.PreviewKeyDown += ArrowKeyDown;
        // ContentDialog handles Escape inside its separate popup tree before
        // the app root accelerator can run. Cancel that implicit close when
        // a selection surface or menu above the dialog consumes the key.
        dialog.Closing += DialogClosing;
    }
    public async Task ShowAsync(string id)
    {
        var started = false;
        dialog.Loaded += async (_, _) =>
        {
            if (started) return; started = true;
            try { await model.OpenAsync(id, lifetime.Token); }
            catch (Exception ex) { if (!disposed) figure.Content = Note(ex.Message); }
        };
        await dialog.ShowAsync();
    }
    public void Hide() { closeRequested = true; dialog.Hide(); }
    private void DialogClosing(ContentDialog _, ContentDialogClosingEventArgs e)
    {
        if (!disposed && !closeRequested && (model.ExportBusy || DismissLayer())) e.Cancel = true;
    }
    private bool DismissLayer()
    {
        if (openMenus.LastOrDefault() is { } menu) { menu.Hide(); return true; }
        return code?.HandleEscape() == true || figure.HandleEscape();
    }
    private bool QuoteDocument(NativePanelArtifact item, NativePanelFileContent content, NativeDocumentSelection selection)
    {
        if (disposed || lifetime.IsCancellationRequested || model.DocumentQuote(item, content, selection) is not { } request
            || quoteDocument?.Invoke(request) != true) return false;
        if (selection.Jump) dialog.DispatcherQueue.TryEnqueue(() => { if (!disposed && ReferenceEquals(model.Selected, item) && ReferenceEquals(model.Content, content)) Hide(); });
        return true;
    }
    private async void ArrowKeyDown(object sender, KeyRoutedEventArgs e)
    {
        var focus = Microsoft.UI.Xaml.Input.FocusManager.GetFocusedElement(dialog.XamlRoot);
        var editable = focus is TextBox { IsReadOnly: false } or RichEditBox;
        var modifiers = new[] { VirtualKey.Control, VirtualKey.Menu, VirtualKey.Shift, VirtualKey.LeftWindows, VirtualKey.RightWindows }
            .Any(key => (InputKeyboardSource.GetKeyStateForCurrentThread(key) & CoreVirtualKeyStates.Down) != 0);
        var blocked = e.Handled || disposed || model.ExportBusy || openMenus.Count > 0 || code?.IsMenuOpen == true;
        if (!blocked && !editable && !modifiers && figure.HandlePdfKey((int)e.Key)) { e.Handled = true; return; }
        var direction = NativeArtifactNavigation.Direction((int)e.Key, blocked, editable, modifiers);
        if (direction == 0 || model.Adjacent(direction) == null) return;
        e.Handled = true; await model.NavigateAsync(direction, lifetime.Token);
    }
    public bool HandleEscape() { if (model.ExportBusy || DismissLayer()) return true; Hide(); return true; }
    private void Refresh()
    {
        if (disposed || model.Selected is not { } item) return;
        if (selected != item.Id) { selected = item.Id; tab = "code"; editor.Reset(); }
        title.Text = item.Name; AutomationProperties.SetName(dialog, item.Name);
        previous.IsEnabled = !model.ExportBusy && model.Adjacent(-1) != null; next.IsEnabled = !model.ExportBusy && model.Adjacent(1) != null;
        previous.Visibility = next.Visibility = model.Adjacent(-1) != null || model.Adjacent(1) != null ? Visibility.Visible : Visibility.Collapsed;
        download.IsEnabled = !model.ExportBusy && pickDownload != null;
        star.Visibility = model.CanUseLibrary ? Visibility.Visible : Visibility.Collapsed;
        star.IsEnabled = !model.LibraryBusy;
        var starLabel = !model.LibraryKnown && !model.LibraryBusy ? design.L("重试读取收藏状态", "Retry library status") : design.T(model.LibraryItemId == null ? "library.add" : "library.remove");
        star.Content = design.Icon(model.LibraryItemId == null ? "star" : "star-filled", 16,
            design.ColorText(model.LibraryItemId == null ? "text" : "clay-strong"));
        AutomationProperties.SetName(star, starLabel); ToolTipService.SetToolTip(star, starLabel);
        actionStatus.Text = model.LibraryError ?? model.ExportError ?? (model.ExportedPath is { } path ? design.L("已保存：", "Saved: ") + path : "");
        actionStatus.Visibility = actionStatus.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        retry.Visibility = model.ContentError != null || model.ProvenanceError != null ? Visibility.Visible : Visibility.Collapsed;
        retry.IsEnabled = !model.LoadingContent && !model.LoadingProvenance && !model.ExportBusy;
        RenderDetails();
    }
    private void RootChanged(XamlRoot _, XamlRootChangedEventArgs __) => Fit();
    private void TypographyChanged() => dialog.DispatcherQueue.TryEnqueue(Fit);
    private void Fit()
    {
        if (disposed || fitting) return; fitting = true;
        try
        {
            var item = model.Selected;
            var kind = NativeGeneratedArtifacts.FileKind(item?.Name ?? "") ?? item?.Kind ?? "file";
            var initial = NativeArtifactLayout.Fit(root.Size.Width, root.Size.Height, kind, 0, 0);
            layout.Width = initial.Width; layout.MaxHeight = initial.MaxHeight;
            dialog.SetBounds(initial.Width + NativeArtifactLayout.Inset, initial.MaxHeight + NativeArtifactLayout.Inset);
            detailHeight = root.Size.Height * .3; code?.SetViewportHeight(detailHeight);
            var measure = new Windows.Foundation.Size(initial.Width, double.PositiveInfinity);
            header.Measure(measure); tabBorder.Measure(measure); details.Measure(measure);
            var chrome = header.DesiredSize.Height + tabBorder.DesiredSize.Height + 30;
            foreach (var extra in new FrameworkElement[] { actionStatus, retry })
                if (extra.Visibility == Visibility.Visible) { extra.Measure(measure); chrome += extra.DesiredSize.Height + 10; }
            var geometry = NativeArtifactLayout.Fit(root.Size.Width, root.Size.Height, kind, chrome, details.DesiredSize.Height);
            detailScroll.MaxHeight = geometry.DetailsHeight; code?.SetViewportHeight(geometry.DetailsHeight);
            var padding = kind is "docx" or "xlsx" or "pptx" ? 0 : 8;
            figureBox.Padding = new(padding); figureBox.MaxHeight = geometry.FigureHeight;
            figureScroll.MaxHeight = Math.Max(0, geometry.FigureHeight - padding * 2);
            if (item != null) figure.Present(item, model.Content, model.LoadingContent, model.ContentError,
                Math.Max(1, geometry.Width - padding * 2), Math.Max(1, geometry.FigureHeight - padding * 2));
        }
        finally { fitting = false; }
    }
    private TextBlock Note(string value) { var text = design.Text(value, 12.5); text.Foreground = design.Brush("text-faint"); text.Margin = new(16); return text; }
    private TextBox Text(string value, double maxHeight)
    {
        var editor = new TextBox { AcceptsReturn = true, Text = value, IsReadOnly = true,
            TextWrapping = TextWrapping.Wrap, MaxHeight = maxHeight, MinHeight = 40, FontFamily = design.CodeFont,
            FontSize = design.FontSize(12, true), BorderThickness = new(1), Padding = new(10), CornerRadius = new(10),
            BorderBrush = design.Brush("border"), Foreground = design.Brush("text"), Background = design.Brush("bg-sunken") };
        design.BindTypography(editor, code: true);
        var menu = new TextCommandBarFlyout(); menu.Opened += (_, _) => { openMenus.Remove(menu); openMenus.Add(menu); };
        menu.Closed += (_, _) => openMenus.Remove(menu); editor.ContextFlyout = menu; return editor;
    }
    private void RenderDetails()
    {
        BuildDetails(); Fit();
    }
    private void BuildDetails()
    {
        foreach (var (key, button) in tabs)
        {
            var active = key == tab; button.Background = active ? design.Brush("bg-elev") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            button.BorderBrush = active ? design.Brush("border-strong") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            ((TextBlock)button.Content).Foreground = design.Brush(active ? "text" : "text-faint");
        }
        copy.Visibility = tab == "code" && model.Provenance?.Code.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        edit.Visibility = copy.Visibility; edit.IsEnabled = insertDraft != null;
        edit.Content = design.Icon("edit", 16, design.ColorText(editor.Editing ? "clay-strong" : "text"));
        code?.Dispose(); code = null;
        foreach (var menu in openMenus.ToArray()) menu.Hide();
        details.Children.Clear(); details.Spacing = tab == "inputs" ? 4 : 0;
        if (model.LoadingProvenance) { details.Children.Add(Note(design.T("loading"))); return; }
        if (model.Provenance is not { } p) { details.Children.Add(Note(model.ProvenanceError ?? design.T("artifact.none"))); return; }
        if (tab == "code" && editor.Editing)
        {
            var draft = Text(editor.Draft, 180); draft.IsReadOnly = false; draft.MinHeight = draft.Height = 180;
            AutomationProperties.SetName(draft, design.L("修改生成代码", "Edited generating code"));
            var actions = new NativeActionWrap { Gap = 8, RightAligned = true, Margin = new(0, 8, 0, 0) };
            var cancel = new Button { Content = design.L("取消", "Cancel") }; design.ActionButton(cancel); cancel.Click += (_, _) => { editor.Cancel(); RenderDetails(); };
            var insert = new Button { Content = design.L("填入对话框", "Insert into chat"), IsEnabled = !string.IsNullOrWhiteSpace(editor.Draft) };
            design.ActionButton(insert, primary: true);
            draft.TextChanged += (_, _) => { editor.Draft = draft.Text; insert.IsEnabled = !string.IsNullOrWhiteSpace(editor.Draft); };
            insert.Click += (_, _) =>
            {
                if (model.Selected is not { } item || editor.Message(item.Name, p.Language, design.Localization.Locale) is not { } message) return;
                if (insertDraft?.Invoke(message) == true) Hide();
                else { actionStatus.Text = design.L("当前对话无法接收草稿，请稍后再试。", "The conversation cannot accept a draft right now."); actionStatus.Visibility = Visibility.Visible; }
            };
            actions.Children.Add(cancel); actions.Children.Add(insert); details.Children.Add(draft); details.Children.Add(actions);
        }
        else if (tab == "code") { code = new(p.Code, p.Language, design, detailHeight); details.Children.Add(code); }
        else if (tab == "log")
        {
            var log = Mono(p.Output, codeSize: true); log.IsTextSelectionEnabled = true;
            var menu = new TextCommandBarFlyout(); menu.Opened += (_, _) => { openMenus.Remove(menu); openMenus.Add(menu); };
            menu.Closed += (_, _) => openMenus.Remove(menu); log.ContextFlyout = menu;
            AutomationProperties.SetName(log, design.L("执行日志", "Execution log")); details.Children.Add(log);
        }
        else if (tab == "inputs")
        {
            foreach (var input in p.Inputs)
            {
                var label = Mono(input.Path); label.Foreground = design.Brush(input.ProducedHere ? "clay-strong" : "text-muted");
                if (input.ProducedHere)
                { var button = new Button { Content = label, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left,
                        Padding = new(9, 6, 9, 6), CornerRadius = new(10), Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), MinHeight = 0 };
                    AutomationProperties.SetName(button, input.Path);
                    button.Click += (_, _) => { InputRequestedPath = input.Path; Hide(); }; details.Children.Add(button); }
                else details.Children.Add(new Border { Child = label, Padding = new(9, 6, 9, 6), CornerRadius = new(10),
                    Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new(1) });
            }
        }
        else if (p.Env is { } env)
        {
            foreach (var package in env.Packages)
            { var row = new Grid { ColumnSpacing = 16, Padding = new(8, 3, 8, 3) }; row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                row.Children.Add(Mono(package.Name)); var version = Mono(package.Version); version.Foreground = design.Brush("text-faint");
                version.TextAlignment = TextAlignment.Right; Grid.SetColumn(version, 1); row.Children.Add(version);
                details.Children.Add(new Border { Child = row, BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) }); }
        }
        else details.Children.Add(Note(design.L("未记录环境快照。", "No environment snapshot recorded.")));
    }
    private TextBlock Mono(string value, bool codeSize = false)
    {
        var text = design.Text(value, 12);
        text.SetBinding(TextBlock.FontFamilyProperty, new Binding { Source = design, Path = new PropertyPath(nameof(WispDesign.CodeFont)), Mode = BindingMode.OneWay });
        if (codeSize) text.SetBinding(TextBlock.FontSizeProperty, new Binding { Source = design, Path = new PropertyPath(nameof(WispDesign.CodeSize)), Mode = BindingMode.OneWay });
        return text;
    }
    public void Dispose() { if (disposed) return; disposed = true; root.Changed -= RootChanged; design.TypographyChanged -= TypographyChanged; dialog.PreviewKeyDown -= ArrowKeyDown; lifetime.Cancel(); model.Changed -= Refresh; model.Close(); code?.Dispose(); figure.Dispose(); dialog.Hide(); lifetime.Dispose(); }

    // ContentDialog's template resolves its maximum-size resources when opened.
    // Updating only Resources later resizes the content but leaves its frame
    // clipped at the old size. Resize the existing frame without closing the
    // dialog, replacing the preview or losing a code draft/focus.
    private sealed class ArtifactContentDialog : ContentDialog
    {
        private double frameWidth = 880, frameHeight = 756;
        public void SetBounds(double width, double height)
        { frameWidth = width; frameHeight = height; ApplyBounds(); }
        protected override void OnApplyTemplate() { base.OnApplyTemplate(); ApplyBounds(); }
        private void ApplyBounds()
        {
            if (GetTemplateChild("BackgroundElement") is FrameworkElement frame)
            { frame.MaxWidth = frameWidth; frame.MaxHeight = frameHeight; }
        }
    }
}
