using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.ApplicationModel.DataTransfer;
using Windows.System;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeMemosPane : UserControl, IDisposable
{
    private readonly WorkspaceMemosModel model;
    private readonly WispDesign design;
    private readonly Func<string, bool>? insert;
    private readonly StackPanel list = new() { Spacing = 8, Padding = new Thickness(12, 2, 12, 14) };
    private readonly Grid listHost = new();
    private readonly StackPanel empty = new() { Spacing = 10, Padding = new Thickness(18, 28, 18, 28), VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
    private readonly TextBox search = new();
    private readonly Border searchCard = new();
    private readonly StackPanel error = new() { Margin = new Thickness(12, 0, 12, 8), Spacing = 6 };
    private readonly Button create;
    private readonly List<FlyoutBase> menus = [];
    private NativeMemoDraft? renderedDraft;
    private Border? editor;
    private TextBox? titleInput, bodyInput;
    private StackPanel? editorActions, editorError;
    private Grid? editorFooter;
    private readonly List<Action> refreshCards = [];
    private NativeMemo[]? renderedMemos;
    private string? renderedQuery;
    private bool disposed;
    public NativeMemosPane(WorkspaceMemosModel model, WispDesign design, Func<string, bool>? insert = null)
    {
        this.model = model; this.design = design; this.insert = insert;
        var root = new Grid { Background = design.Brush("bg-elev") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        var head = new Grid { ColumnSpacing = 10, Padding = new Thickness(12, 10, 12, 6) };
        head.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var privacy = new Grid { ColumnSpacing = 6, VerticalAlignment = VerticalAlignment.Center };
        privacy.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); privacy.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        privacy.Children.Add(design.Icon("lock", 13, design.ColorText("text-faint")));
        var caption = Text(design.T("memos.private"), 11.5, "text-faint"); caption.TextWrapping = TextWrapping.NoWrap; caption.TextTrimming = TextTrimming.CharacterEllipsis;
        Grid.SetColumn(caption, 1); privacy.Children.Add(caption); ToolTipService.SetToolTip(privacy, caption.Text); head.Children.Add(privacy);
        create = Button(design.T("memos.new"), () => model.New(), true, "plus"); Grid.SetColumn(create, 1); head.Children.Add(create); root.Children.Add(head);
        search.PlaceholderText = design.T("memos.search"); search.Text = model.Query; search.MinWidth = 0;
        StyleInput(search); search.BorderThickness = new Thickness(0); search.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        var searchGrid = new Grid { ColumnSpacing = 8, Padding = new Thickness(10, 0, 2, 0) };
        searchGrid.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); searchGrid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        searchGrid.Children.Add(design.Icon("search", 16, design.ColorText("text-faint"))); Grid.SetColumn(search, 1); searchGrid.Children.Add(search);
        searchCard.Child = searchGrid; searchCard.Margin = new Thickness(12, 0, 12, 8); searchCard.BorderThickness = new Thickness(1); searchCard.BorderBrush = design.Brush("border-strong"); searchCard.CornerRadius = new(6); searchCard.Background = design.Brush("bg-input");
        search.TextChanged += (_, _) => { model.Query = search.Text; Render(); };
        Grid.SetRow(searchCard, 1); root.Children.Add(searchCard); Grid.SetRow(error, 2); root.Children.Add(error);
        var scroll = new ScrollViewer { Content = list, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        listHost.Children.Add(scroll); listHost.Children.Add(empty); Grid.SetRow(listHost, 3); root.Children.Add(listHost);
        Content = root; model.Changed += Render;
        Loaded += (_, _) => Activate();
        Unloaded += (_, _) => { foreach (var menu in menus.ToArray()) menu.Hide(); };
        Render();
    }
    public void Activate() { if (!disposed && IsLoaded && Visibility == Visibility.Visible) { Render(); if (!model.Loaded) _ = model.LoadAsync(); } }
    private TextBlock Text(string value, double size, string color = "text")
    { var text = design.Text(value, size); text.Foreground = design.Brush(color); return text; }
    private Button Button(string label, Action action, bool primary = false, string? icon = null)
    {
        var button = new Button { Padding = new Thickness(11, 6, 11, 6), MinHeight = 30, CornerRadius = new(6),
            Background = design.Brush(primary ? "clay-strong" : "bg-elev"), Foreground = primary ? new SolidColorBrush(Microsoft.UI.Colors.White) : design.Brush("text"),
            BorderBrush = design.Brush(primary ? "clay-strong" : "border-strong"), BorderThickness = new(1), FontWeight = Microsoft.UI.Text.FontWeights.SemiBold };
        design.BindTypography(button, size: 12.5); button.Content = label;
        button.Resources["ButtonBackgroundDisabled"] = button.Background;
        button.Resources["ButtonBorderBrushDisabled"] = button.BorderBrush;
        button.Resources["ButtonForegroundDisabled"] = button.Foreground;
        button.IsEnabledChanged += (_, _) => button.Opacity = button.IsEnabled ? 1 : .55;
        button.Loaded += (_, _) => button.Opacity = button.IsEnabled ? 1 : .55;
        if (icon != null) { var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 }; row.Children.Add(design.Icon(icon, 14, primary ? "#ffffff" : design.ColorText("text"))); row.Children.Add(Text(label, 12.5, primary ? "on-clay" : "text")); button.Content = row; }
        button.Click += (_, _) => action(); AutomationProperties.SetName(button, label); return button;
    }
    private void StyleInput(TextBox input)
    {
        design.BindTypography(input, size: 12.5); input.Padding = new(9, 7, 9, 7); input.MinHeight = 34;
        input.CornerRadius = new(6); input.BorderThickness = new(1); input.BorderBrush = design.Brush("border-strong"); input.Background = design.Brush("bg-input");
        input.Foreground = design.Brush("text"); input.Resources["TextControlBackgroundFocused"] = design.Brush("bg-input"); input.Resources["TextControlBorderBrushFocused"] = design.Brush("clay");
        AutomationProperties.SetName(input, input.PlaceholderText);
    }
    private Border Card(FrameworkElement child, bool editing = false) => new() { Child = child, Padding = new(12, 10, 12, 10), CornerRadius = new(8), BorderThickness = new(1), BorderBrush = design.Brush(editing ? "clay" : "border"), Background = design.Brush(editing ? "bg-elev" : "bg-sunken") };
    private void Render()
    {
        if (disposed) return;
        create.IsEnabled = model.Draft == null && !model.Busy; searchCard.Visibility = model.Memos.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        error.Children.Clear();
        if (model.Loading) error.Children.Add(new ProgressBar { Height = 2, IsIndeterminate = true });
        if (model.Error is { } message) { error.Children.Add(Text(design.T("memos.load_error") + " " + message, 12, "err")); error.Children.Add(Button(design.L("重新读取", "Reload"), () => _ = model.LoadAsync())); }
        if (!ReferenceEquals(renderedDraft, model.Draft)) { renderedDraft = model.Draft; editor = renderedDraft == null ? null : BuildEditor(renderedDraft); renderedMemos = null; }
        UpdateEditor();
        if (!ReferenceEquals(renderedMemos, model.Memos) || renderedQuery != model.Query)
        {
            renderedMemos = model.Memos; renderedQuery = model.Query; list.Children.Clear(); refreshCards.Clear();
            if (model.Draft is { Id: null } && editor != null) list.Children.Add(editor);
            foreach (var memo in model.Filtered)
                if (memo.Id == model.Draft?.Id && editor != null) list.Children.Add(editor); else list.Children.Add(BuildCard(memo));
        }
        foreach (var refresh in refreshCards) refresh();
        empty.Children.Clear();
        empty.Visibility = !model.Loading && list.Children.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        if (model.Memos.Length == 0 && model.Draft == null)
        {
            empty.Children.Add(new Border { Width = 40, Height = 40, CornerRadius = new(8), Background = design.Brush("bg-sunken"), HorizontalAlignment = HorizontalAlignment.Center,
                Child = design.Icon("notebook", 20, design.ColorText("text-faint")) });
            var hint = Text(design.T("memos.empty"), 12.5, "text-muted"); hint.MaxWidth = 280; hint.TextAlignment = TextAlignment.Center; hint.LineHeight = 19.375; empty.Children.Add(hint);
        }
        else if (model.Filtered.Length == 0) empty.Children.Add(Text(design.T("memos.no_match"), 12.5, "text-faint"));
    }
    private Border BuildEditor(NativeMemoDraft draft)
    {
        var body = new StackPanel { Spacing = 8 };
        titleInput = new() { Text = draft.Title, PlaceholderText = design.T("memos.title_placeholder"), FontWeight = Microsoft.UI.Text.FontWeights.SemiBold };
        // WinUI truncates Text at the first newline when assigned before AcceptsReturn.
        bodyInput = new() { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, Text = draft.Body, PlaceholderText = design.T("memos.body_placeholder") };
        StyleInput(titleInput); StyleInput(bodyInput); bodyInput.Height = draft.EditorRows * 18.75 + 16;
        titleInput.TextChanged += (_, _) => draft.Title = titleInput.Text;
        bodyInput.TextChanged += (_, _) => { draft.Body = bodyInput.Text; bodyInput.Height = draft.EditorRows * 18.75 + 16; UpdateEditor(); };
        body.Children.Add(titleInput); body.Children.Add(bodyInput);
        editorError = new() { Spacing = 6 }; body.Children.Add(editorError);
        editorFooter = new Grid { ColumnSpacing = 6 };
        editorFooter.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); editorFooter.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var shortcut = Text("Ctrl+Enter", 11, "text-faint"); shortcut.VerticalAlignment = VerticalAlignment.Center; editorFooter.Children.Add(shortcut);
        editorActions = new() { Orientation = Orientation.Horizontal, Spacing = 6, HorizontalAlignment = HorizontalAlignment.Right };
        Grid.SetColumn(editorActions, 1); editorFooter.Children.Add(editorActions); body.Children.Add(editorFooter);
        // Preview handles Ctrl+Enter before the multiline TextBox consumes it as
        // a newline. Escape continues through the window-level overlay stack.
        async void SaveShortcut(object sender, KeyRoutedEventArgs e)
        {
            if (e.Key != VirtualKey.Enter || (Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(VirtualKey.Control) & Windows.UI.Core.CoreVirtualKeyStates.Down) == 0) return;
            e.Handled = true; await model.SaveAsync();
        }
        titleInput.PreviewKeyDown += SaveShortcut; bodyInput.PreviewKeyDown += SaveShortcut;
        return Card(body, true);
    }
    private void UpdateEditor()
    {
        if (model.Draft == null || editorActions == null || titleInput == null || bodyInput == null || editorError == null || editorFooter == null) return;
        titleInput.IsReadOnly = bodyInput.IsReadOnly = model.Busy;
        editorError.Children.Clear(); editorActions.Children.Clear();
        if (model.SaveError is { } message)
        {
            editorError.Children.Add(Text(message, 12, "err"));
            if (model.Uncertain) { editorError.Children.Add(Text(design.L("保存结果未确认。请重新读取列表核对后关闭这份草稿，避免重复保存。", "Save was not confirmed. Reload the list and check it before closing this draft to avoid saving twice."), 12, "text-muted")); editorError.Children.Add(Button(design.L("重新读取", "Reload"), () => _ = model.LoadAsync())); }
        }
        if (model.ConfirmDiscard)
        {
            var confirmation = new Grid { ColumnSpacing = 6 };
            confirmation.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            confirmation.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); confirmation.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var question = Text(design.T("memos.discard_q"), 12.5); question.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; question.VerticalAlignment = VerticalAlignment.Center; confirmation.Children.Add(question);
            var discard = Button(design.T("memos.discard"), model.Discard); discard.Foreground = design.Brush("err"); Grid.SetColumn(discard, 1); confirmation.Children.Add(discard);
            var keep = Button(design.T("memos.keep_editing"), model.KeepEditing); Grid.SetColumn(keep, 2); confirmation.Children.Add(keep);
            editorError.Children.Add(new Border { Child = confirmation, Padding = new(10, 8, 10, 8), CornerRadius = new(6), Background = Mix("err", "bg-elev", .08) });
        }
        else
        {
            var cancel = Button(design.T("memos.cancel"), model.Cancel); cancel.IsEnabled = !model.Busy; editorActions.Children.Add(cancel);
            var save = Button(design.T("memos.save"), () => _ = model.SaveAsync(), true); save.IsEnabled = model.Draft.CanSave && !model.Busy && !model.Uncertain; editorActions.Children.Add(save);
        }
        editorFooter.Visibility = model.ConfirmDiscard ? Visibility.Collapsed : Visibility.Visible;
        editorError.Visibility = editorError.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
    }
    private SolidColorBrush Mix(string foreground, string background, double weight)
    {
        var a = design.Brush(foreground).Color; var b = design.Brush(background).Color;
        var alpha = a.A * weight + b.A * (1 - weight);
        byte Channel(byte x, byte y) => alpha == 0 ? (byte)0 : (byte)Math.Round((x * a.A * weight + y * b.A * (1 - weight)) / alpha);
        return new(Windows.UI.Color.FromArgb((byte)Math.Round(alpha), Channel(a.R, b.R), Channel(a.G, b.G), Channel(a.B, b.B)));
    }
    private Border BuildCard(NativeMemo memo)
    {
        bool Expanded() => model.Expanded == memo.Id;
        var body = new StackPanel { Spacing = 6 };
        var head = new Grid { ColumnSpacing = 6 }; head.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); head.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        if (memo.Pinned) head.Children.Add(design.Icon("pin", 13, design.ColorText("clay-strong")));
        var title = Text((WorkspaceMemosModel.Heading(memo) ?? design.T("memos.untitled")).ReplaceLineEndings(" "), 13); title.TextWrapping = TextWrapping.NoWrap; title.MaxLines = 1; title.TextTrimming = TextTrimming.CharacterEllipsis; title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var heading = new Button { Content = title, HorizontalContentAlignment = HorizontalAlignment.Stretch, HorizontalAlignment = HorizontalAlignment.Stretch };
        design.QuietButton(heading); heading.Padding = new(0, 2, 0, 2); heading.MinWidth = 0; heading.MinHeight = 26;
        heading.Click += (_, _) => { model.Expanded = Expanded() ? null : memo.Id; Render(); }; heading.DoubleTapped += (_, e) => { e.Handled = true; model.Edit(memo); };
        AutomationProperties.SetName(heading, title.Text); Grid.SetColumn(heading, 1); head.Children.Add(heading);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2 };
        var insertButton = Button(design.T("memos.insert"), () => { }, false, "arrow-left"); insertButton.Padding = new(8, 3, 8, 3); insertButton.MinHeight = 26; insertButton.Background = design.Brush("clay-soft"); insertButton.IsEnabled = insert != null;
        var insertLabel = Text(design.T("memos.insert"), 12, "clay-strong"); insertLabel.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var insertRow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 }; insertRow.Children.Add(design.Icon("arrow-left", 14, design.ColorText("clay-strong"))); insertRow.Children.Add(insertLabel);
        insertButton.Content = insertRow; insertButton.Foreground = design.Brush("clay-strong"); insertButton.BorderBrush = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        insertButton.Click += async (_, _) => { if (insert?.Invoke(memo.Body) != true) return; insertButton.Content = design.T("memos.inserted"); await Task.Delay(1500); if (!disposed) { renderedMemos = null; Render(); } }; actions.Children.Add(insertButton);
        var edit = design.ToolButton(design.T("memos.edit"), "square-pen"); edit.Width = edit.Height = 26; edit.MinWidth = edit.MinHeight = 26; edit.Padding = new(5); edit.IsEnabled = model.Draft == null; edit.Click += (_, _) => model.Edit(memo); actions.Children.Add(edit);
        var more = design.ToolButton(design.T("memos.more"), "more"); more.Width = more.Height = 26; more.MinWidth = more.MinHeight = 26; more.Padding = new(5); actions.Children.Add(more);
        var menu = new MenuFlyout { Placement = FlyoutPlacementMode.BottomEdgeAlignedRight }; var menuStyle = new Style(typeof(MenuFlyoutPresenter));
        menuStyle.Setters.Add(new Setter(FrameworkElement.MinWidthProperty, 160d)); menuStyle.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(4)));
        menuStyle.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev"))); menuStyle.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        menuStyle.Setters.Add(new Setter(Control.BorderThicknessProperty, new Thickness(1))); menuStyle.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(8))); menu.MenuFlyoutPresenterStyle = menuStyle;
        foreach (var (key, icon, action) in new (string, string, Action)[] {
            (memo.Pinned ? "memos.unpin" : "memos.pin", "pin", () => _ = model.PinAsync(memo)),
            ("memos.copy", "copy", () => { var data = new DataPackage(); data.SetText(memo.Body); Clipboard.SetContent(data); }),
            ("memos.delete", "trash", () => _ = model.DeleteAsync(memo)) })
        {
            var item = new MenuFlyoutItem { Text = design.T(key), Icon = new ImageIcon { Source = design.Icon(icon, 14).Source }, MinHeight = 30, Padding = new(10, 7, 10, 7), Margin = new(0), CornerRadius = new(6) };
            design.BindTypography(item, size: 12.5); item.Resources["MenuFlyoutItemBackgroundPointerOver"] = design.Brush("bg-sunken");
            menu.Opening += (_, _) => item.IsEnabled = key == "memos.copy" || !model.Busy && !model.Uncertain;
            if (key == "memos.delete") item.Foreground = design.Brush("err"); item.Click += (_, _) => action(); menu.Items.Add(item);
        }
        menu.Opened += (_, _) => menus.Add(menu); menu.Closed += (_, _) => menus.Remove(menu); more.Flyout = menu;
        Grid.SetColumn(actions, 2); head.Children.Add(actions); body.Children.Add(head);
        var text = Text(memo.Body, 12.5); text.LineHeight = 18.75; text.TextTrimming = TextTrimming.CharacterEllipsis;
        text.Tapped += (_, _) => { model.Expanded = Expanded() ? null : memo.Id; Render(); }; text.DoubleTapped += (_, e) => { e.Handled = true; model.Edit(memo); }; body.Children.Add(text);
        body.Children.Add(Text(NativeBrowserPresentation.RelativeTime(memo.UpdatedAt, DateTimeOffset.Now, locale: design.Localization), 11, "text-faint"));
        var card = Card(body); var pointer = false; var focused = false;
        if (memo.Pinned) card.BorderBrush = Mix("clay", "border", .4);
        void Reveal() => actions.Opacity = Expanded() || pointer || focused ? 1 : 0;
        refreshCards.Add(() => { text.MaxLines = Expanded() ? 0 : 2; text.Foreground = design.Brush(Expanded() ? "text" : "text-muted");
            ToolTipService.SetToolTip(heading, design.T(Expanded() ? "memos.collapse" : "memos.expand")); edit.IsEnabled = model.Draft == null && !model.Busy; Reveal(); });
        card.PointerEntered += (_, _) => { pointer = true; Reveal(); }; card.PointerExited += (_, _) => { pointer = false; Reveal(); };
        card.GotFocus += (_, _) => { focused = true; Reveal(); }; card.LostFocus += (_, _) => { focused = false; Reveal(); };
        return card;
    }
    public bool HandleEscape() { if (menus.LastOrDefault() is { } menu) { menu.Hide(); return true; } return model.Escape(); }
    public void Dispose() { if (disposed) return; disposed = true; model.Changed -= Render; foreach (var menu in menus.ToArray()) menu.Hide(); }
}
