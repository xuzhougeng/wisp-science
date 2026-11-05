using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage.Streams;

namespace Wisp.Science.Preview;

internal sealed class NativeLibraryPage : UserControl, IWorkspaceSheet
{
    private readonly WorkspaceLibraryModel model;
    private readonly WispDesign design;
    private readonly Action<string>? insert;
    private readonly Func<LibraryItemSummary, Task> source;
    private readonly Action close;
    private readonly Action<FlyoutBase> register;
    private readonly Grid layers = new(), toolbar = new();
    private readonly StackPanel page = new() { HorizontalAlignment = HorizontalAlignment.Left }, list = new() { Spacing = 10 };
    private readonly Grid searchBox = new() { ColumnSpacing = 9 };
    private readonly NativeActionWrap filters = new() { Gap = 3 };
    private readonly Dictionary<string, Button> filterButtons = new();
    private readonly List<FrameworkElement> wideOnly = [];
    private readonly TextBox query;
    private readonly Border searchBorder, filterBorder;
    private readonly ScrollViewer scroll;
    private readonly StackPanel errors = new(), detailErrors = new();
    private readonly DispatcherTimer debounce = new() { Interval = TimeSpan.FromMilliseconds(150) };
    private IReadOnlyList<LibraryItemSummary>? renderedItems;
    private string? detailKey;
    private Grid? overlay;
    private Border? detailCard;
    private StackPanel? detailBody;
    private NativeRichPreview? figurePreview;
    private Button? saveButton, removeButton;
    private readonly List<Control> detailActions = [];
    private TextBox? draft;
    private Button? openedCard;
    private bool disposed, compact;

    public NativeLibraryPage(WorkspaceLibraryModel model, WispDesign design, Action<string>? insert,
        Func<LibraryItemSummary, Task> source, Action close, Action<FlyoutBase> register)
    {
        this.model = model; this.design = design; this.insert = insert; this.source = source; this.close = close; this.register = register;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        design.BindTypography(this); AutomationProperties.SetAutomationId(this, "library-screen");
        var header = new Grid { ColumnSpacing = 20, Margin = new Thickness(0, 0, 0, 26) };
        Columns(header, new(1, GridUnitType.Star), GridLength.Auto);
        var labels = new StackPanel { Spacing = 7 };
        var title = Text(T("library.title"), 34); title.FontFamily = new("ms-appx:///Assets/Fonts/wisp-serif.ttf#Wisp Serif, Noto Serif SC, SimSun, Georgia, Times New Roman");
        labels.Children.Add(title); labels.Children.Add(Text(T("library.subtitle"), 13, "text-faint")); header.Children.Add(labels);
        var closeLibrary = Tool(T("library.close"), "close", HandleEscape); closeLibrary.VerticalAlignment = VerticalAlignment.Top;
        Put(header, closeLibrary, 0, 1); page.Children.Add(header);
        toolbar.ColumnSpacing = 16; toolbar.RowSpacing = 12; toolbar.Margin = new Thickness(0, 0, 0, 18);
        Columns(toolbar, new(1, GridUnitType.Star), GridLength.Auto); Rows(toolbar, GridLength.Auto, GridLength.Auto);
        Columns(searchBox, GridLength.Auto, new(1, GridUnitType.Star));
        var searchIcon = design.Icon("search", 15, design.ColorText("text-faint")); searchIcon.VerticalAlignment = VerticalAlignment.Center; searchBox.Children.Add(searchIcon);
        query = new TextBox { PlaceholderText = T("library.search"), Text = model.Query, MinWidth = 0, MinHeight = 0,
            BorderThickness = new Thickness(0), Padding = new Thickness(0), Background = null, Foreground = design.Brush("text") };
        design.BindTypography(query, size: 13); AutomationProperties.SetName(query, T("library.search")); ContextMenu(query);
        foreach (var key in new[] { "TextControlBackgroundFocused", "TextControlBackgroundPointerOver" }) query.Resources[key] = design.Brush("bg-input");
        query.Resources["TextControlBorderThemeThicknessFocused"] = new Thickness(0);
        query.TextChanged += (_, _) => { model.Query = query.Text; debounce.Stop(); debounce.Start(); };
        debounce.Tick += async (_, _) => { debounce.Stop(); await model.SearchAsync(); };
        Put(searchBox, query, 0, 1);
        searchBorder = new Border { Child = searchBox, MaxWidth = 560, HorizontalAlignment = HorizontalAlignment.Stretch,
            Background = design.Brush("bg-input"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new(8), Padding = new(12, 9, 12, 9) };
        toolbar.Children.Add(searchBorder);
        foreach (var (value, key) in new[] { ("all", "library.all"), ("figure", "library.figures"), ("code", "library.code"), ("text", "library.texts") })
        {
            var button = Action(T(key), () => { debounce.Stop(); model.Kind = value; _ = model.SearchAsync(); }, 12);
            button.Padding = new Thickness(11, 6, 11, 6); filterButtons[value] = button; filters.Children.Add(button);
        }
        filterBorder = new Border { Child = filters, Padding = new(3), BorderThickness = new(1), BorderBrush = design.Brush("border"), Background = design.Brush("bg-sunken"), CornerRadius = new(8), VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetName(filterBorder, T("library.filter")); filterBorder.SizeChanged += (_, _) => Resize(); Put(toolbar, filterBorder, 0, 1); page.Children.Add(toolbar);
        page.Children.Add(errors); page.Children.Add(list);
        scroll = new ScrollViewer { Content = page, HorizontalContentAlignment = HorizontalAlignment.Left,
            HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, Background = design.Brush("bg-app") };
        layers.Children.Add(scroll); Content = layers;
        model.Changed += Refresh; SizeChanged += (_, _) => Resize(); Loaded += (_, _) => Resize();
        Refresh(); _ = model.SearchAsync();
    }
    private string T(string key) => design.T(key);
    private TextBlock Text(string value, double size = 14, string color = "text", bool bold = false)
    { var t = design.Text(value, size); t.Foreground = design.Brush(color); t.FontWeight = bold ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal; return t; }
    private static void Columns(Grid grid, params GridLength[] widths) { foreach (var width in widths) grid.ColumnDefinitions.Add(new() { Width = width }); }
    private static void Rows(Grid grid, params GridLength[] heights) { foreach (var height in heights) grid.RowDefinitions.Add(new() { Height = height }); }
    private static void Put(Grid grid, FrameworkElement child, int row = 0, int column = 0) { Grid.SetRow(child, row); Grid.SetColumn(child, column); grid.Children.Add(child); }
    private Button Action(string label, Action action, double size = 13)
    {
        var button = new Button { Content = Text(label, size), Padding = new(12, 8, 12, 8), CornerRadius = new(6), MinWidth = 0, MinHeight = 0 };
        design.QuietButton(button); button.Padding = new(12, 8, 12, 8); button.MinHeight = 0;
        AutomationProperties.SetName(button, label); button.Click += (_, _) => action(); return button;
    }
    private Button Tool(string label, string icon, Action action)
    { var button = design.ToolButton(label, icon); design.QuietButton(button); button.Click += (_, _) => action(); return button; }
    private void ContextMenu(TextBox editor) { var menu = new TextCommandBarFlyout(); editor.ContextFlyout = menu; register(menu); }
    private void Resize()
    {
        var layout = NativeLibraryLayout.ForViewport(ActualWidth); compact = layout.Compact;
        toolbar.RowSpacing = compact ? 12 : 0;
        toolbar.ColumnSpacing = compact ? 0 : 16;
        // An explicit content width avoids ScrollContentPresenter measuring a
        // MaxWidth-constrained StackPanel wider than its centered arrangement.
        page.Width = layout.ContentWidth; page.Margin = new Thickness(layout.CenterOffset, 0, 0, 0);
        scroll.Padding = new Thickness(layout.Inset, 38, layout.Inset, 56);
        toolbar.ColumnDefinitions[1].Width = compact ? new(0) : GridLength.Auto;
        Grid.SetRow(filterBorder, compact ? 1 : 0); Grid.SetColumn(filterBorder, compact ? 0 : 1);
        filterBorder.HorizontalAlignment = compact ? HorizontalAlignment.Left : HorizontalAlignment.Stretch;
        searchBorder.MaxWidth = compact ? double.PositiveInfinity : 560;
        searchBorder.HorizontalAlignment = HorizontalAlignment.Left;
        searchBorder.Width = Math.Min(compact ? double.PositiveInfinity : 560, Math.Max(0, Math.Min(1040, ActualWidth - layout.Inset * 2) - (compact ? 0 : filterBorder.ActualWidth + 16)));
        foreach (var control in wideOnly) control.Visibility = compact ? Visibility.Collapsed : Visibility.Visible;
        if (detailCard != null) { detailCard.Width = Math.Max(0, Math.Min(900, ActualWidth - 40)); detailCard.MaxHeight = Math.Max(0, ActualHeight - 40); }
        if (figurePreview != null) figurePreview.MaxHeight = Math.Max(120, ActualHeight * .54);
    }
    private void Refresh()
    {
        if (disposed || model.Closed) return;
        foreach (var (kind, button) in filterButtons)
        {
            var active = (model.Kind ?? "all") == kind;
            button.Background = active ? design.Brush("bg-elev") : null;
            if (button.Content is TextBlock label) label.Foreground = design.Brush(active ? "text" : "text-faint");
        }
        errors.Children.Clear();
        if (model.Error != null) Error(errors, model.Error, () => _ = model.SearchAsync());
        if (model.DetailError != null && model.Selected == null && !model.LoadingDetail) Error(errors, T("library.read_failed") + "\n" + model.DetailError);
        if (!ReferenceEquals(renderedItems, model.Items)) { renderedItems = model.Items; RenderItems(); }
        // Empty/loading status changes without replacing mounted filter/search inputs.
        if (model.Items.Count == 0) RenderItems();
        RenderDetail(); Resize();
    }
    private void RenderItems()
    {
        list.Children.Clear(); wideOnly.Clear();
        if (model.Items.Count == 0)
        {
            if (model.Busy) { list.Children.Add(Text(T("loading"), 13, "text-faint")); return; }
            if (model.Error != null) return;
            var empty = new StackPanel { Padding = new Thickness(20, 80, 20, 80), HorizontalAlignment = HorizontalAlignment.Stretch };
            var icon = design.Icon("star", 30, design.ColorText("text-faint")); icon.HorizontalAlignment = HorizontalAlignment.Center; empty.Children.Add(icon);
            var title = Text(T("library.empty.title"), 16, bold: true); title.TextAlignment = TextAlignment.Center; title.Margin = new(0, 12, 0, 5); empty.Children.Add(title);
            var hint = Text(T("library.empty.body"), 13, "text-faint"); hint.TextAlignment = TextAlignment.Center; empty.Children.Add(hint); list.Children.Add(empty); return;
        }
        foreach (var item in model.Items)
        {
            var row = new Grid(); Columns(row, new(1, GridUnitType.Star), GridLength.Auto);
            var content = new Grid { ColumnSpacing = 13 }; Columns(content, GridLength.Auto, new(1, GridUnitType.Star), GridLength.Auto);
            var icon = new Border { Width = 36, Height = 36, Background = design.Brush("clay-soft"), CornerRadius = new(6), VerticalAlignment = VerticalAlignment.Top,
                Child = design.Icon(item.Kind == "figure" ? "image" : "doc", 18, design.ColorText("clay-strong")) }; content.Children.Add(icon);
            var body = new StackPanel { Spacing = 5 }; var title = Text(item.Title, 14, bold: true); title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis; body.Children.Add(title);
            var excerpt = NativeLibraryLayout.Excerpt(item.CodePreview);
            if (excerpt.Length > 0) { var preview = Text(excerpt, 11.5, "text-muted"); preview.FontFamily = design.CodeFont; preview.MaxHeight = 78; preview.Margin = new(0, 1, 0, 1); body.Children.Add(preview); }
            var meta = Text(item.SourceProjectName + " / " + item.SourceSessionTitle, 12, "text-faint"); meta.TextWrapping = TextWrapping.NoWrap; meta.TextTrimming = TextTrimming.CharacterEllipsis; body.Children.Add(meta); Put(content, body, 0, 1);
            var kindText = item.Kind == "figure" ? T("library.figure") : item.Kind == "text" ? T("library.text") : item.Language ?? "code";
            var badge = new Border { Child = Text(kindText, 11, "text-faint"), Background = design.Brush("bg-sunken"), CornerRadius = new(99), Padding = new(8, 3, 8, 3), Margin = new(10, 0, 0, 0), VerticalAlignment = VerticalAlignment.Top };
            Put(content, badge, 0, 2); wideOnly.Add(badge);
            var open = new Button { Content = content, HorizontalContentAlignment = HorizontalAlignment.Stretch, Padding = new(16, 14, 16, 14), MinWidth = 0 };
            design.QuietButton(open); open.Padding = new(16, 14, 16, 14); open.HorizontalAlignment = HorizontalAlignment.Stretch;
            AutomationProperties.SetName(open, item.Title); open.Click += async (_, _) => { openedCard = open; await model.OpenAsync(item.Id); }; row.Children.Add(open);
            var sourceButton = Action(T("library.open_source"), () => _ = source(item), 12); sourceButton.Width = 98; sourceButton.CornerRadius = new(0); sourceButton.BorderThickness = new(1, 0, 0, 0); sourceButton.BorderBrush = design.Brush("border");
            sourceButton.Padding = new(4); if (sourceButton.Content is TextBlock sourceLabel) sourceLabel.TextWrapping = TextWrapping.NoWrap;
            sourceButton.VerticalAlignment = VerticalAlignment.Stretch; Put(row, sourceButton, 0, 1); wideOnly.Add(sourceButton);
            var card = new Border { Child = row, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(8) };
            card.PointerEntered += (_, _) => card.BorderBrush = design.Brush("clay"); card.PointerExited += (_, _) => card.BorderBrush = design.Brush("border"); list.Children.Add(card);
        }
    }
    private void Error(StackPanel panel, string message, Action? retry = null)
    {
        var body = new StackPanel { Spacing = 8 }; body.Children.Add(Text(message, 13, "err"));
        if (retry != null) body.Children.Add(Action(design.L("重试", "Retry"), retry));
        panel.Children.Add(new Border { Child = body, Padding = new(12, 9, 12, 9), BorderThickness = new(1), BorderBrush = design.Brush("err"), CornerRadius = new(8), Margin = new(0, 0, 0, 12) });
    }
    private void RenderDetail()
    {
        var item = model.Selected;
        var key = model.LoadingDetail ? "loading" : item == null ? null : $"{item.Id}:{model.Version?.Id}:{model.Editing}";
        if (key != detailKey)
        {
            figurePreview?.Dispose(); figurePreview = null;
            detailKey = key; detailBody?.Children.Remove(detailErrors); if (overlay != null) layers.Children.Remove(overlay); overlay = null; detailCard = null; draft = null; detailBody = null; detailActions.Clear(); saveButton = removeButton = null;
            detailErrors.Children.Clear();
            if (key != null)
            {
                overlay = new Grid(); NativeDialogVisuals.Backdrop(overlay, design); layers.Children.Add(overlay);
                overlay.Tapped += (_, e) => { if (ReferenceEquals(e.OriginalSource, overlay)) model.CloseDetail(); };
                if (model.LoadingDetail) overlay.Children.Add(new ProgressRing { IsActive = true, Width = 36, Height = 36, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center });
                else if (item != null) BuildDetail(item);
            }
            scroll.IsEnabled = key == null;
            if (key == null && openedCard != null) openedCard.Focus(FocusState.Programmatic);
        }
        detailErrors.Children.Clear(); if (item != null && model.DetailError != null) Error(detailErrors, model.DetailError);
        foreach (var control in detailActions) control.IsEnabled = !model.MutationBusy;
        if (saveButton != null) saveButton.IsEnabled = model.CanSave;
        if (draft != null) draft.IsReadOnly = model.MutationBusy;
    }
    private void BuildDetail(LibraryItemDetail item)
    {
        detailBody = new StackPanel { Padding = new(24), Spacing = 16, TabFocusNavigation = KeyboardNavigationMode.Cycle };
        var header = new Grid { ColumnSpacing = 16 }; Columns(header, new(1, GridUnitType.Star), GridLength.Auto);
        var labels = new StackPanel { Spacing = 4 }; var title = Text(item.Title, 20, bold: true); title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis; labels.Children.Add(title);
        labels.Children.Add(Text(item.SourceProjectName + " / " + item.SourceSessionTitle, 12, "text-faint")); header.Children.Add(labels);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        removeButton = Tool(T("library.remove"), "star-filled", () => _ = model.DeleteAsync(item.Id)); removeButton.Content = design.Icon("star-filled", 16, design.ColorText("clay-strong")); detailActions.Add(removeButton); actions.Children.Add(removeButton);
        var dismiss = Tool(T("library.close"), "close", () => model.CloseDetail()); actions.Children.Add(dismiss); Put(header, actions, 0, 1); detailBody.Children.Add(header); detailBody.Children.Add(detailErrors);
        if (item.Kind == "figure")
        {
            var imageHost = new Border { Background = design.Brush("bg-sunken"), CornerRadius = new(8), Padding = new(10) }; detailBody.Children.Add(imageHost);
            if (item.ContentType == "image/svg+xml" && item.Base64 != null)
            {
                // WinUI's SVG decoder omits SVG text. An inert browser image uses
                // the same text-capable renderer as WebView without mounting SVG DOM.
                figurePreview = new NativeRichPreview(design, "image", item.Base64, item.Title, escape: HandleEscape, format: item.ContentType) { MaxHeight = Math.Max(120, ActualHeight * .54) };
                imageHost.Child = figurePreview;
            }
            else
            {
                var image = new Image { Stretch = Stretch.Uniform, MaxHeight = Math.Max(120, ActualHeight * .54) };
                imageHost.Child = image; _ = LoadImage(imageHost, image, item);
            }
            if (item.Code.Length > 0) { detailBody.Children.Add(Text(T("library.generating_code"), 13, bold: true)); BuildCode(detailBody, item); }
        }
        else if (item.Kind == "text") { var text = Text(item.Code); text.IsTextSelectionEnabled = true; detailBody.Children.Add(text); }
        else BuildCode(detailBody, item);
        var openSource = Action(T("library.open_source"), () => _ = source(item.Summary)); openSource.HorizontalAlignment = HorizontalAlignment.Right; detailActions.Add(openSource); detailBody.Children.Add(openSource);
        var detailScroll = new ScrollViewer { Content = detailBody, HorizontalContentAlignment = HorizontalAlignment.Stretch, HorizontalScrollMode = ScrollMode.Disabled, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        detailCard = new Border { Child = detailScroll, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(16),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
        overlay!.Children.Add(detailCard); detailCard.Tapped += (_, e) => e.Handled = true;
        detailCard.Loaded += (_, _) => { if (model.Editing && draft != null) draft.Focus(FocusState.Programmatic); else dismiss.Focus(FocusState.Programmatic); };
    }
    private void BuildCode(StackPanel parent, LibraryItemDetail item)
    {
        if (model.Version is not { } version) return;
        if (model.Versions.Count > 1)
        {
            var versions = new NativeActionWrap { Gap = 3 };
            foreach (var row in model.Versions)
            {
                var button = Action(row.VersionNumber == 1 ? "v1 · " + T("library.version_original") : $"v{row.VersionNumber}", () => model.SelectVersion(row.Id), 12);
                button.Background = row.Id == version.Id ? design.Brush("bg-elev") : null; detailActions.Add(button); versions.Children.Add(button);
            }
            parent.Children.Add(new Border { Child = versions, Background = design.Brush("bg-sunken"), BorderThickness = new(1), BorderBrush = design.Brush("border"), CornerRadius = new(8), Padding = new(3), HorizontalAlignment = HorizontalAlignment.Left });
        }
        if (model.Editing)
        {
            draft = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, Text = model.Draft, MinHeight = 240, Padding = new(10), Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), Foreground = design.Brush("text"), CornerRadius = new(8) };
            draft.Resources["TextControlBackgroundFocused"] = draft.Resources["TextControlBackgroundPointerOver"] = design.Brush("bg-sunken");
            draft.Resources["TextControlBorderThemeThicknessFocused"] = new Thickness(1);
            draft.Resources["TextControlBorderBrushFocused"] = design.Brush("clay");
            design.BindTypography(draft, code: true); ContextMenu(draft); AutomationProperties.SetName(draft, T("library.edit"));
            draft.TextChanged += (_, _) => { model.Draft = draft.Text.ReplaceLineEndings("\n"); if (saveButton != null) saveButton.IsEnabled = model.CanSave; }; parent.Children.Add(draft);
            var actions = new NativeActionWrap { RightAligned = true, Gap = 8 };
            var cancel = Action(T("library.cancel"), model.CancelEdit); detailActions.Add(cancel); actions.Children.Add(cancel);
            saveButton = Action(T("library.save"), () => _ = model.SaveAsync()); saveButton.Background = design.Brush("clay"); if (saveButton.Content is TextBlock saveLabel) saveLabel.Foreground = new SolidColorBrush(Microsoft.UI.Colors.White); actions.Children.Add(saveButton); parent.Children.Add(actions); return;
        }
        var head = new Grid { ColumnSpacing = 8 }; Columns(head, new(1, GridUnitType.Star), GridLength.Auto);
        var label = Text($"v{version.VersionNumber}", 13, bold: true); label.VerticalAlignment = VerticalAlignment.Center; head.Children.Add(label);
        var controls = new NativeActionWrap { Gap = 4, RightAligned = true };
        if (insert != null) { var button = Action(T("library.insert"), () => insert(WorkspaceLibraryModel.ComposerText(item, version, T("library.insert_prompt")))); controls.Children.Add(button); detailActions.Add(button); }
        controls.Children.Add(Tool(T("tool.copy_code"), "copy", () => { try { var package = new DataPackage(); package.SetText(version.Code); Clipboard.SetContent(package); } catch (Exception ex) { Error(detailErrors, ex.Message); } }));
        var edit = Tool(T("library.edit"), "edit", model.BeginEdit); detailActions.Add(edit); controls.Children.Add(edit); Put(head, controls, 0, 1); parent.Children.Add(head);
        var code = Text("", 12); code.FontFamily = design.CodeFont; code.IsTextSelectionEnabled = true; code.TextWrapping = TextWrapping.NoWrap; code.Padding = new(12);
        TranscriptView.ApplyHighlight(code.Inlines, version.Code, NativeCodeHighlight.NormalizeLanguage(version.Language), design);
        parent.Children.Add(new Border { Background = design.Brush("bg-sunken"), CornerRadius = new(8), Child = new ScrollViewer { Content = code, MaxHeight = item.Kind == "figure" ? Math.Max(120, ActualHeight * .28) : 420, HorizontalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalScrollBarVisibility = ScrollBarVisibility.Auto } });
    }
    private async Task LoadImage(Border host, Image image, LibraryItemDetail item)
    {
        try
        {
            if (item.Base64 == null) throw new InvalidDataException();
            using var memory = new InMemoryRandomAccessStream(); using (var writer = new DataWriter(memory)) { writer.WriteBytes(Convert.FromBase64String(item.Base64)); await writer.StoreAsync(); writer.DetachStream(); }
            memory.Seek(0);
            var bitmap = new BitmapImage(); await bitmap.SetSourceAsync(memory); image.Source = bitmap;
        }
        catch { if (!disposed && model.Selected?.Id == item.Id) host.Child = Text(T("library.read_failed"), 13, "err"); }
    }
    public void HandleEscape() { if (model.Selected != null || model.LoadingDetail) model.CloseDetail(); else close(); }
    public void Dispose() { disposed = true; debounce.Stop(); figurePreview?.Dispose(); model.Changed -= Refresh; model.Dispose(); }
}
