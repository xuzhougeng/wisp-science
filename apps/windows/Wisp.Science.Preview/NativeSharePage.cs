using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// <summary>Native selection UI; only the exported artifact uses the shared canvas renderer.</summary>
internal sealed class NativeSharePage : WorkspaceSheet
{
    private readonly WorkspaceShareModel model;
    private readonly Func<NativeShareFile, Task<string?>> save;
    private readonly Action close;
    private readonly StackPanel messages = new() { Spacing = 8, Padding = new Thickness(2) };
    private readonly Dictionary<int, (CheckBox Check, TextBlock Text, Border Card, StackPanel Body)> rows = new();
    private readonly TextBlock count, error, status;
    private readonly TextBox keywords, width;
    private readonly Button export, cancel, all, none, png, html;
    private readonly StackPanel widthField;
    private readonly ScrollViewer list;
    private readonly Grid rendererHost = new() { Width = 1, Height = 1, Opacity = 0, IsHitTestVisible = false };
    private bool disposed, updating;
    private NativeShareRenderer? renderer;

    public NativeSharePage(WorkspaceShareModel model, WispDesign design, Func<NativeShareFile, Task<string?>> save, Action close)
        : base(design, design.T("share.title"), close, modal: true, modalWidth: 560)
    {
        this.model = model; this.save = save; this.close = close;
        AutomationProperties.SetAutomationId(this, "share-overlay");
        var hint = Label("share.hint", 12.5, "text-muted"); Body.Children.Add(hint);
        var selection = new Grid { ColumnSpacing = 12 };
        selection.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        selection.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        selection.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
        all = Link("share.select_all", () => model.SelectAll(true)); selection.Children.Add(all);
        none = Link("share.select_none", () => model.SelectAll(false)); Grid.SetColumn(none, 1); selection.Children.Add(none);
        count = Label("", 12, "text-faint"); count.HorizontalAlignment = HorizontalAlignment.Right; count.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(count, 2); selection.Children.Add(count); Body.Children.Add(selection);
        list = new ScrollViewer { Content = messages, HorizontalScrollMode = ScrollMode.Disabled,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalContentAlignment = HorizontalAlignment.Stretch, MinHeight = 120, MaxHeight = 360 };
        AutomationProperties.SetAutomationId(list, "share-list"); Body.Children.Add(list);
        status = Label("", 12.5, "text-muted"); messages.Children.Add(status);
        keywords = Input("share.redact_ph", "share-redact-input"); Body.Children.Add(Field("share.redact_label", keywords));
        keywords.TextChanged += (_, _) => { model.Keywords = keywords.Text; Refresh(); };
        var format = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10 };
        var formatLabel = Label("share.format_label", 12, "text-muted"); formatLabel.VerticalAlignment = VerticalAlignment.Center; format.Children.Add(formatLabel);
        var segments = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 };
        png = Segment("share.format_png", NativeShareFormat.Png); html = Segment("share.format_html", NativeShareFormat.Html);
        segments.Children.Add(png); segments.Children.Add(html);
        format.Children.Add(new Border { Child = segments, Padding = new Thickness(2), CornerRadius = new(10),
            BorderThickness = new(1), BorderBrush = design.Brush("border-strong"), Background = design.Brush("bg-sunken") }); Body.Children.Add(format);
        width = Input("", "share-width-input"); width.PlaceholderText = "840";
        width.TextChanged += (_, _) => model.Width = width.Text;
        widthField = Field("share.width_label", width); Body.Children.Add(widthField);
        error = Label("", 12.5, "text"); error.Visibility = Visibility.Collapsed; Body.Children.Add(error);
        var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Right };
        cancel = new() { Content = design.T("share.cancel") }; design.ActionButton(cancel); cancel.Click += (_, _) => HandleEscape();
        export = new(); design.ActionButton(export, primary: true); AutomationProperties.SetAutomationId(export, "share-export");
        export.Click += async (_, _) => { if (await model.ExportAsync(design.Dark, SaveAsync) && !disposed) close(); };
        buttons.Children.Add(cancel); buttons.Children.Add(export); Footer.Children.Add(buttons); Footer.Visibility = Visibility.Visible;
        // Keep the renderer mounted but outside keyboard/pointer navigation. It never displays UI.
        if (Content is Grid overlay) overlay.Children.Insert(1, rendererHost);
        SizeChanged += (_, _) => DispatcherQueue.TryEnqueue(FitList);
        BodyScroll.SizeChanged += (_, _) => DispatcherQueue.TryEnqueue(FitList);
        model.Changed += Refresh;
        Refresh(); _ = LoadAsync();
    }
    private async Task LoadAsync()
    {
        await model.LoadAsync(); if (disposed) return;
        messages.Children.Clear(); rows.Clear();
        foreach (var row in model.Rows)
        {
            var role = Design.T(row.Row.Role == "user" ? "share.role_user" : row.Row.Role == "reasoning" ? "share.role_thinking" : "share.role_assistant");
            var checkbox = new CheckBox { IsChecked = row.Selected, MinWidth = 20, MinHeight = 20, Padding = new(0), VerticalAlignment = VerticalAlignment.Top };
            AutomationProperties.SetName(checkbox, role); AutomationProperties.SetAutomationId(checkbox, $"share-message-{row.Id}");
            checkbox.Checked += (_, _) => { if (!updating) model.SetSelected(row.Id, true); };
            checkbox.Unchecked += (_, _) => { if (!updating) model.SetSelected(row.Id, false); };
            var content = new StackPanel { Spacing = 6 };
            var badgeText = Design.Text(role, 10.5); badgeText.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; badgeText.Foreground = Design.Brush("text-muted");
            var badge = new Border { Child = badgeText, Padding = new(8, 1, 8, 1), CornerRadius = new(20), BorderThickness = new(1), BorderBrush = Design.Brush("border"), HorizontalAlignment = HorizontalAlignment.Left };
            if (row.Row.Role == "user") { badgeText.Foreground = Design.Brush("clay-strong"); badge.Background = Design.Brush("clay-soft"); }
            else if (row.Row.Role == "reasoning")
            {
                badgeText.Foreground = Design.Brush("warn"); var color = Design.Brush("warn").Color; color.A = 25; badge.Background = new SolidColorBrush(color);
            }
            content.Children.Add(badge);
            var preview = Design.Text(row.Row.Text, 13); preview.MaxLines = 3; preview.TextTrimming = TextTrimming.CharacterEllipsis;
            content.Children.Add(preview);
            var grid = new Grid { ColumnSpacing = 10 }; grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); grid.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) });
            grid.Children.Add(checkbox); Grid.SetColumn(content, 1); grid.Children.Add(content);
            var card = new Border { Child = grid, Padding = new(10), CornerRadius = new(10), BorderThickness = new(1) };
            card.Tapped += (_, e) => { if (e.OriginalSource is not CheckBox && !model.Exporting) { model.SetSelected(row.Id, !model.Rows.First(r => r.Id == row.Id).Selected); e.Handled = true; } };
            // CheckBox consumes its own tap; body taps offer the same full-row target as WebView.
            checkbox.Tapped += (_, e) => e.Handled = true;
            rows.Add(row.Id, (checkbox, preview, card, content)); messages.Children.Add(card);
        }
        if (rows.Count == 0) messages.Children.Add(status);
        Refresh();
    }
    private void Refresh()
    {
        if (disposed) return;
        updating = true;
        var words = NativeShareDraft.Keywords(model.Keywords);
        foreach (var row in model.Rows)
            if (rows.TryGetValue(row.Id, out var view))
            {
                view.Check.IsChecked = row.Selected; view.Check.IsEnabled = !model.Exporting;
                view.Text.Text = NativeShareDraft.Redact(row.Row.Text, words); view.Body.Opacity = row.Selected ? 1 : .55;
                view.Card.BorderBrush = Design.Brush(row.Selected ? "clay" : "border");
                var tint = Design.Brush("clay").Color; tint.A = row.Selected ? (byte)10 : (byte)0; view.Card.Background = new SolidColorBrush(tint);
            }
        updating = false;
        count.Text = Design.T("share.selected").Replace("{count}", $"{model.Rows.Count(r => r.Selected)}/{model.Rows.Length}");
        status.Text = model.Loading ? Design.L("正在加载…", "Loading…") : Design.L("没有可分享的消息", "No messages to share");
        error.Text = model.Error ?? ""; error.Visibility = model.Error == null ? Visibility.Collapsed : Visibility.Visible;
        export.Content = Design.T(model.Exporting ? "share.exporting" : model.Format == NativeShareFormat.Html ? "share.export_html" : "share.export");
        export.IsEnabled = model.CanExport; cancel.IsEnabled = !model.Exporting;
        all.IsEnabled = none.IsEnabled = keywords.IsEnabled = width.IsEnabled = png.IsEnabled = html.IsEnabled = !model.Exporting && !model.Loading;
        widthField.Visibility = model.Format == NativeShareFormat.Png ? Visibility.Visible : Visibility.Collapsed;
        foreach (var (button, selected) in new[] { (png, model.Format == NativeShareFormat.Png), (html, model.Format == NativeShareFormat.Html) })
        {
            button.Background = Design.Brush(selected ? "bg-elev" : "bg-sunken");
            button.FontWeight = selected ? Microsoft.UI.Text.FontWeights.SemiBold : Microsoft.UI.Text.FontWeights.Normal;
        }
        DispatcherQueue.TryEnqueue(FitList);
    }
    private void FitList()
    {
        if (disposed || BodyScroll.ActualHeight <= 0) return;
        var others = Body.Children.OfType<FrameworkElement>().Where(child => child != list && child.Visibility == Visibility.Visible).ToArray();
        var remaining = BodyScroll.ActualHeight - others.Sum(child => child.DesiredSize.Height) - Body.Spacing * others.Length;
        list.MaxHeight = Math.Clamp(remaining, 120, 400);
    }
    private async Task<string?> SaveAsync(NativeShareDocument document, CancellationToken token)
    {
        renderer = new NativeShareRenderer(); rendererHost.Children.Add(renderer);
        try
        {
            var file = await renderer.RenderAsync(document, Design, token);
            token.ThrowIfCancellationRequested();
            return await save(file);
        }
        finally { renderer?.Dispose(); rendererHost.Children.Clear(); renderer = null; }
    }
    private TextBlock Label(string key, double size, string color)
    { var label = Design.Text(key.Length == 0 ? "" : Design.T(key), size); label.Foreground = Design.Brush(color); return label; }
    private Button Link(string key, Action action)
    {
        var button = new Button { Content = Design.T(key), FontSize = 12.5 }; Design.QuietButton(button);
        button.Padding = new(0); button.MinHeight = 0; button.Foreground = Design.Brush("clay"); button.Click += (_, _) => action(); return button;
    }
    private Button Segment(string key, NativeShareFormat format)
    {
        var button = new Button { Content = Design.T(key), FontSize = 12.5 }; Design.QuietButton(button);
        button.Padding = new(12, 4, 12, 4); button.MinHeight = 0; button.CornerRadius = new(8);
        AutomationProperties.SetAutomationId(button, format == NativeShareFormat.Png ? "share-format-png" : "share-format-html");
        button.Click += (_, _) => { model.Format = format; Refresh(); }; return button;
    }
    private TextBox Input(string placeholder, string id)
    {
        var box = new TextBox { PlaceholderText = placeholder.Length == 0 ? "" : Design.T(placeholder), Padding = new(11, 9, 11, 9), CornerRadius = new(10),
            BorderThickness = new(1), BorderBrush = Design.Brush("border-strong"), Background = Design.Brush("bg-input"), Foreground = Design.Brush("text") };
        Design.BindTypography(box); AutomationProperties.SetAutomationId(box, id); return box;
    }
    private StackPanel Field(string key, TextBox input)
    {
        AutomationProperties.SetName(input, Design.T(key));
        var panel = new StackPanel { Spacing = 6 }; panel.Children.Add(Label(key, 12, "text-muted")); panel.Children.Add(NativeDialogVisuals.Input(input, Design)); return panel;
    }
    public override void Dispose()
    {
        if (disposed) return; disposed = true; model.Changed -= Refresh; model.Dispose(); renderer?.Dispose(); base.Dispose();
    }
}
