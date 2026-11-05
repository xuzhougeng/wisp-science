using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

/// Project memory has its own browsing scope and inline global-habit editor.
internal sealed class NativeMemoryPage : NativeActionPage
{
    private readonly NativeMemoryModel model;
    private readonly SettingsTextMenus textMenus = new();
    private Button? projectPicker;
    private Flyout? projectMenu;
    private bool projectMenuOpen;
    private readonly List<Border> cards = [];
    private ToggleSwitch? enabled;
    private Button? save, deleteNote;
    private TextBox? noteEditor;
    private readonly TextBlock feedback = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Border feedbackBox = new();
    private Action? confirmation;
    public bool Busy => model.Busy;
    private static string S(JsonNode? row, string key) => row?[key]?.GetValue<string>() ?? "";
    public NativeMemoryPage(INativeSettingsClient client, string? project, WispDesign design, Action close)
        : this(new NativeMemoryModel(client, project, design.Localization), design, close) { _ = Load(); }
    private NativeMemoryPage(NativeMemoryModel model, WispDesign design, Action close) : base(design, design.T("settings.nav.memory"), model, close)
    {
        this.model = model; UseSettingsLayout(reserveOverflowScrollbar: true); Form.MaxWidth = double.PositiveInfinity; Form.Spacing = 14; Body.Spacing = 0;
        feedbackBox.Child = feedback; feedbackBox.Padding = new Thickness(10, 8, 10, 8); feedbackBox.CornerRadius = new CornerRadius(10);
        feedbackBox.BorderBrush = Design.Brush("border-strong"); feedbackBox.BorderThickness = new Thickness(1); feedbackBox.Background = Design.Brush("bg-input");
        Design.BindTypography(feedback, 12); feedback.LineHeight = Design.FontSize(12) * 1.45; feedback.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        model.Changed += RefreshControls;
        BodyScroll.SizeChanged += (_, _) => { ResizeEditor(); ResizeCards(); };
        Body.SizeChanged += (_, _) => ResizeEditor();
    }
    private string T(string key) => Design.T(key);
    private string L(string zh, string en) => Design.L(zh, en);
    private async Task Load() { await model.LoadAsync(); if (!model.Closed) Render(); }
    private TextBlock Hint(string text, double size = 13, bool faint = false)
    {
        var block = Design.Text(text, size); block.Foreground = Design.Brush(faint ? "text-faint" : "text-muted");
        block.LineHeight = Design.FontSize(size) * 1.5; block.LineStackingStrategy = LineStackingStrategy.BlockLineHeight; return block;
    }
    private Button Action(string title, Func<Task> action, bool primary = false, bool compact = false)
    {
        var button = Button(title, action, primary); button.Padding = compact ? new Thickness(10, 4, 10, 4) : new Thickness(14, 8, 14, 8);
        button.MinHeight = 0; button.CornerRadius = new CornerRadius(10); button.Background = Design.Brush(primary ? "clay" : "bg-input");
        button.BorderBrush = Design.Brush(primary ? "clay" : "border-strong");
        if (button.Content is TextBlock text) { Design.BindTypography(text, compact ? 12 : 13); text.FontWeight = Microsoft.UI.Text.FontWeights.Medium; if (primary) text.Foreground = Design.Brush("on-clay"); }
        if (primary) { button.Resources["ButtonBackgroundDisabled"] = Design.Brush("clay"); button.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay"); }
        return button;
    }
    private void RefreshControls()
    {
        SetContentEnabled(!model.Busy && confirmation == null);
        if (deleteNote != null) deleteNote.IsEnabled = !model.Busy && confirmation == null;
        if (save != null) { save.IsEnabled = model.CanSave && confirmation == null; save.Opacity = save.IsEnabled ? 1 : .5; }
        if (enabled != null) enabled.IsEnabled = !model.Busy && !model.HasDraft && confirmation == null;
        feedback.Text = model.Feedback ?? ""; feedbackBox.Visibility = string.IsNullOrEmpty(model.Feedback) ? Visibility.Collapsed : Visibility.Visible;
    }
    private void Confirm(string message, Action next)
    {
        confirmation = next; Notices.Children.Clear(); Notices.Children.Add(Hint(message));
        var actions = new NativeActionWrap();
        actions.Children.Add(Action(L("确认", "Confirm"), () => { var action = confirmation; ClearConfirmation(); action?.Invoke(); return Task.CompletedTask; }));
        actions.Children.Add(Action(T("settings.cancel"), () => { ClearConfirmation(); return Task.CompletedTask; }));
        Notices.Children.Add(actions); Notices.Visibility = Visibility.Visible; RefreshControls();
    }
    private void ClearConfirmation() { confirmation = null; Notices.Children.Clear(); Notices.Visibility = Visibility.Collapsed; RefreshControls(); }
    public void RequestLeave(Action leave)
    {
        if (model.Busy) return;
        if (model.HasChanges) Confirm(L("放弃尚未保存的修改？", "Discard unsaved changes?"), leave); else leave();
    }
    public override void HandleEscape()
    {
        if (textMenus.CloseTop()) return;
        if (projectMenuOpen) { projectMenu?.Hide(); return; }
        if (confirmation != null) { ClearConfirmation(); return; }
        if (model.Busy) return;
        if (model.HasDraft) { RequestLeave(() => { model.Discard(); Render(); }); return; }
        base.HandleEscape();
    }
    private void Render()
    {
        if (model.Closed) return;
        textMenus.CloseTop(); projectMenu?.Hide(); projectMenu = null; projectMenuOpen = false; cards.Clear();
        Form.Children.Clear(); Results.Children.Clear(); Footer.Children.Clear(); HeaderActions.Children.Clear();
        projectPicker = null; enabled = null; save = deleteNote = null; noteEditor = null;
        if (model.NoteName == null) SettingsHeading(T("settings.nav.memory"));
        else SettingsBreadcrumb(T("settings.nav.memory"), model.NoteName, () => RequestLeave(() => { model.Discard(); Render(); }));
        Body.Padding = model.NoteName == null ? new Thickness(24, 18, 24, 22) : new Thickness(40, 30, 40, 10);
        // Only reserve the scrollbar when the body overflows, keeping the
        // editor and footer aligned in both short and full-height windows.
        Footer.Padding = new Thickness(40, 0, 40 + Body.Margin.Right, 36);
        Footer.Visibility = model.NoteName == null ? Visibility.Collapsed : Visibility.Visible;
        if (model.NoteName != null) NoteForm();
        else
        {
            if (model.View == null)
            {
                var refresh = Design.ToolButton(L("刷新记忆", "Reload memory"), "refresh");
                refresh.Click += (_, _) => RequestLeave(() => { model.Discard(); _ = Load(); }); HeaderActions.Children.Add(refresh);
                Form.Children.Add(Hint(model.Error == null ? L("请先创建项目，再管理项目记忆和全局习惯。", "Create a project to manage project memory and global habits.") : T("memory.load_failed")));
            }
            else Collections();
            BodyScroll.ChangeView(null, 0, null, true);
        }
        Design.ApplyTypography(Form); Design.ApplyTypography(Footer); ResizeCards(); RefreshControls();
    }
    private Grid Row(FrameworkElement left, FrameworkElement right, double breakpoint = 380)
    {
        var grid = new Grid { ColumnSpacing = 12 };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        left.VerticalAlignment = right.VerticalAlignment = VerticalAlignment.Center; grid.Children.Add(left); Grid.SetColumn(right, 1); grid.Children.Add(right);
        grid.SizeChanged += (_, e) =>
        {
            var narrow = e.NewSize.Width < breakpoint; Grid.SetColumnSpan(left, narrow ? 2 : 1); Grid.SetRow(right, narrow ? 1 : 0); Grid.SetColumn(right, narrow ? 0 : 1); Grid.SetColumnSpan(right, narrow ? 2 : 1);
            grid.RowSpacing = narrow ? 10 : 0;
            right.HorizontalAlignment = narrow ? HorizontalAlignment.Left : HorizontalAlignment.Right;
        };
        return grid;
    }
    private void Collections()
    {
        var selected = model.Projects.FirstOrDefault(row => S(row, "id") == model.ProjectId);
        var projectName = S(selected, "name"); if (string.IsNullOrWhiteSpace(projectName)) projectName = S(model.View, "project_name");
        if (string.IsNullOrWhiteSpace(projectName)) projectName = model.ProjectId;
        projectPicker = Action(T("memory.choose_project"), () => Task.CompletedTask); projectPicker.MaxWidth = 360;
        projectPicker.FocusVisualPrimaryBrush = Design.Brush("clay"); projectPicker.FocusVisualSecondaryBrush = Design.Brush("focus-ring");
        projectPicker.Padding = new Thickness(12, 7, 12, 7); projectPicker.HorizontalAlignment = HorizontalAlignment.Left;
        var trigger = new Grid { ColumnSpacing = 6 }; trigger.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); trigger.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var nameText = Design.Text(projectName + $" ({model.Files.Count})", 13); nameText.TextWrapping = TextWrapping.NoWrap; nameText.TextTrimming = TextTrimming.CharacterEllipsis; nameText.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        trigger.Children.Add(nameText); var caret = Design.Icon("chevron-down", 12, Design.ColorText("text-faint")); Grid.SetColumn(caret, 1); trigger.Children.Add(caret); projectPicker.Content = trigger;
        var menuRows = new StackPanel { Spacing = 1 };
        var menuScroll = new ScrollViewer { Content = menuRows, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        projectMenu = new Flyout { Content = menuScroll, Placement = FlyoutPlacementMode.BottomEdgeAlignedLeft };
        var presenter = new Style(typeof(FlyoutPresenter));
        presenter.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(6))); presenter.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(10)));
        presenter.Setters.Add(new Setter(Control.BorderBrushProperty, Design.Brush("border-strong"))); presenter.Setters.Add(new Setter(Control.BorderThicknessProperty, new Thickness(1)));
        presenter.Setters.Add(new Setter(Control.BackgroundProperty, Design.Brush("bg-elev"))); projectMenu.FlyoutPresenterStyle = presenter;
        foreach (var project in model.Projects)
        {
            var id = S(project, "id"); var name = S(project, "name"); if (name.Length == 0) name = id;
            var item = new Button { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, Padding = new Thickness(10, 7, 10, 7), CornerRadius = new CornerRadius(6), MinHeight = 0 };
            Design.QuietButton(item); item.Padding = new Thickness(10, 7, 10, 7); item.MinHeight = 0; item.CornerRadius = new CornerRadius(6);
            AutomationProperties.SetName(item, name); var copy = new StackPanel { Spacing = 1 };
            var title = Design.Text(name, 13); title.FontWeight = Microsoft.UI.Text.FontWeights.Medium; title.TextWrapping = TextWrapping.NoWrap; title.TextTrimming = TextTrimming.CharacterEllipsis; copy.Children.Add(title);
            if (!string.IsNullOrWhiteSpace(S(project, "description"))) { var description = Hint(S(project, "description"), 11, true); description.TextWrapping = TextWrapping.NoWrap; description.TextTrimming = TextTrimming.CharacterEllipsis; copy.Children.Add(description); }
            var option = new Grid { ColumnSpacing = 8 }; option.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); option.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); option.Children.Add(copy);
            if (id == model.ProjectId) { item.Background = Design.Brush("clay-soft"); var check = Design.Icon("check", 14, Design.ColorText("clay-strong")); Grid.SetColumn(check, 1); option.Children.Add(check); }
            item.Content = option; item.Click += (_, _) => { projectMenu?.Hide(); RequestLeave(() => { model.Discard(); _ = SwitchProject(id); }); }; menuRows.Children.Add(item);
        }
        projectMenu.Opening += (_, _) =>
        {
            menuScroll.Width = Math.Max(208, Math.Min(348, projectPicker.ActualWidth - 12));
            menuScroll.MaxHeight = Math.Max(80, Math.Min(360, (XamlRoot?.Size.Height ?? ActualHeight) * .5) - 12);
        };
        projectMenu.Opened += (_, _) => projectMenuOpen = true; projectMenu.Closed += (_, _) => projectMenuOpen = false; projectPicker.Flyout = projectMenu;
        projectPicker.Loaded += (sender, _) =>
        {
            // Removing the focused note editor can make WinUI reveal a later button.
            // Restore collection focus after layout so returning from a note starts at its project.
            if (model.HasDraft || model.Closed) return;
            ((Button)sender).Focus(FocusState.Programmatic);
            BodyScroll.ChangeView(null, 0, null, true);
        };
        enabled = new ToggleSwitch { IsOn = model.Enabled, OnContent = "", OffContent = "", MinWidth = 0 };
        PreferenceToggleSwitch.Apply(enabled, Design); AutomationProperties.SetName(enabled, T("memory.enabled_label"));
        enabled.Toggled += async (_, _) => { await model.SetEnabledAsync(enabled.IsOn); if (!model.Closed) Render(); };
        var toggle = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, VerticalAlignment = VerticalAlignment.Center };
        var toggleLabel = Hint(T("memory.enabled_label")); toggleLabel.VerticalAlignment = VerticalAlignment.Center; toggle.Children.Add(toggleLabel); toggle.Children.Add(enabled);
        Form.Children.Add(Row(projectPicker, toggle)); Form.Children.Add(feedbackBox);
        if (!model.Enabled)
        {
            var turnOn = Action(T("memory.turn_on"), async () => { await model.SetEnabledAsync(true); if (!model.Closed) Render(); }); turnOn.IsEnabled = !model.HasDraft;
            Form.Children.Add(new Border { Child = Row(Hint(T("memory.off_banner")), turnOn), Padding = new Thickness(14, 12, 14, 12), CornerRadius = new CornerRadius(10), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), Background = Design.Brush("bg-sunken") });
        }
        var projectCard = ProjectCard(); var globalCard = GlobalCard();
        var grid = new Grid { ColumnSpacing = 20, RowSpacing = 20 };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        projectCard.VerticalAlignment = globalCard.VerticalAlignment = VerticalAlignment.Top; grid.Children.Add(projectCard); grid.Children.Add(globalCard);
        void Resize()
        {
            var wide = NativeMemoryLayout.UseTwoColumns(XamlRoot?.Size.Width ?? ActualWidth);
            grid.RowSpacing = wide ? 0 : 20;
            Grid.SetColumnSpan(projectCard, wide ? 1 : 2); Grid.SetColumn(globalCard, wide ? 1 : 0); Grid.SetRow(globalCard, wide ? 0 : 1); Grid.SetColumnSpan(globalCard, wide ? 1 : 2);
        }
        grid.Loaded += (_, _) => Resize(); grid.SizeChanged += (_, _) => Resize(); Form.Children.Add(grid);
    }
    private async Task SwitchProject(string id) { await model.SelectProjectAsync(id); if (!model.Closed) Render(); }
    private Grid Empty(string message)
    {
        var content = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center };
        var icon = Design.Icon("doc", 20, Design.ColorText("text-faint")); icon.HorizontalAlignment = HorizontalAlignment.Center; content.Children.Add(icon); content.Children.Add(Hint(message, faint: true));
        var empty = new Grid(); empty.Children.Add(new Border { Child = content, Padding = new Thickness(16, 24, 16, 24), Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(10) });
        empty.Children.Add(new Rectangle { Stroke = Design.Brush("border-strong"), StrokeThickness = 1, StrokeDashArray = new DoubleCollection { 3, 2 }, RadiusX = 10, RadiusY = 10, IsHitTestVisible = false }); return empty;
    }
    private TextBlock Title(string text, double size = 15) { var title = Design.Text(text, size); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; return title; }
    private Border Card(UIElement content) { var card = Design.Card(content); cards.Add(card); return card; }
    private void ResizeCards()
    {
        var narrow = (XamlRoot?.Size.Width ?? ActualWidth) <= 850;
        foreach (var card in cards) card.Padding = narrow ? new Thickness(16, 18, 16, 18) : new Thickness(24, 22, 24, 22);
    }
    private Border ProjectCard()
    {
        var body = new StackPanel();
        var add = Action(T("memory.add"), () => { RequestLeave(() => { model.Discard(); _ = OpenNote(model.TodayFile); }); return Task.CompletedTask; }); add.IsEnabled = model.TodayFile.Length > 0;
        var heading = Row(Title(T("memory.scope_hint")), add, 290); heading.Margin = new Thickness(0, 0, 0, 16); body.Children.Add(heading);
        if (model.Files.Count == 0) body.Children.Add(Empty(T("memory.empty")));
        var files = model.Files;
        for (var index = 0; index < files.Count; index++)
        {
            var file = files[index];
            var name = S(file, "name");
            var edit = Button(L("打开记忆笔记：", "Open memory note: ") + name, () => { RequestLeave(() => { model.Discard(); _ = OpenNote(name); }); return Task.CompletedTask; });
            var copy = new Grid { ColumnSpacing = 12 }; copy.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); copy.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            copy.Children.Add(new Border { Width = 20, Child = Design.Icon("doc", 16, Design.ColorText("text-faint")) }); var fileName = EntryText(name); Grid.SetColumn(fileName, 1); copy.Children.Add(fileName);
            var detail = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 }; detail.Children.Add(Hint(NativeMemoryLayout.FormatBytes(file["bytes"]?.GetValue<long>() ?? 0), 12, true)); detail.Children.Add(Design.Icon("chevron-right", 14, Design.ColorText("text-faint")));
            edit.Content = Row(copy, detail, 0); Design.QuietButton(edit); edit.HorizontalAlignment = edit.HorizontalContentAlignment = HorizontalAlignment.Stretch; edit.Padding = new Thickness(0, 14, 0, 14);
            edit.Resources["ButtonBackgroundPointerOver"] = Design.Brush("bg-elev");
            body.Children.Add(new Border { Child = edit, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, index + 1 < files.Count ? 1 : 0) });
        }
        var clear = Action(T("memory.clear_all"), () => { RequestLeave(() => { model.Discard(); Confirm(L("清空此项目的所有记忆笔记？此操作不可撤销。", "Clear all memory notes in this project? This cannot be undone."), () => _ = Mutate(model.ClearAsync)); }); return Task.CompletedTask; }); Design.QuietButton(clear); clear.HorizontalAlignment = HorizontalAlignment.Left; clear.Padding = new Thickness(0); clear.MinHeight = 0;
        if (clear.Content is TextBlock label) { Design.BindTypography(label, 12); label.Foreground = Design.Brush("text-muted"); }
        body.Children.Add(new Border { Child = clear, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Margin = new Thickness(0, 20, 0, 0), Padding = new Thickness(0, 14, 0, 0) });
        return Card(body);
    }
    private Border GlobalCard()
    {
        var body = new StackPanel();
        var title = Title(Design.Localization.Locale == "en" ? T("memory.global_scope").ToUpperInvariant() : T("memory.global_scope"), 14); title.CharacterSpacing = 60;
        var heading = Row(title, Action(T("memory.global_add"), () => { EditGlobal(); return Task.CompletedTask; }, compact: true), 330); heading.Margin = new Thickness(0, 0, 0, 16); body.Children.Add(heading);
        var timing = Hint(T("memory.global_timing_hint")); timing.Margin = new Thickness(0, 0, 0, 16); body.Children.Add(timing);
        if (model.GlobalId != null) GlobalForm(body);
        if (model.GlobalMemories.Count == 0) body.Children.Add(Empty(T("memory.global_empty")));
        var memories = model.GlobalMemories;
        for (var index = 0; index < memories.Count; index++)
        {
            var memory = memories[index]; var id = S(memory, "id");
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
            var edit = EntryAction(T("memory.global_edit"), "edit"); edit.Click += (_, _) => EditGlobal(id); actions.Children.Add(edit);
            var remove = EntryAction(T("memory.global_delete"), "close", destructive: true); remove.Click += (_, _) => RequestLeave(() => { model.Discard(); Confirm(L("删除这条全局习惯？此操作不可撤销。", "Forget this global habit? This cannot be undone."), () => _ = Mutate(() => model.DeleteGlobalAsync(id))); }); actions.Children.Add(remove);
            var row = new Border { Child = Row(EntryText(S(memory, "content")), actions, 0), Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent),
                Padding = new Thickness(0, 14, 0, 14), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, index + 1 < memories.Count ? 1 : 0) };
            var hovered = false;
            void RefreshActions()
            {
                // Keep controls in keyboard order and reserve their width when hidden.
                // Each keyboard-focused action remains visible after the pointer leaves.
                edit.Opacity = hovered || edit.FocusState != FocusState.Unfocused ? 1 : 0;
                remove.Opacity = hovered || remove.FocusState != FocusState.Unfocused ? 1 : 0;
            }
            row.PointerEntered += (_, _) => { hovered = true; RefreshActions(); };
            row.PointerExited += (_, _) => { hovered = false; RefreshActions(); };
            edit.GotFocus += (_, _) => RefreshActions(); edit.LostFocus += (_, _) => RefreshActions();
            remove.GotFocus += (_, _) => RefreshActions(); remove.LostFocus += (_, _) => RefreshActions();
            RefreshActions(); body.Children.Add(row);
        }
        return Card(body);
    }
    private TextBlock EntryText(string content)
    {
        var text = Design.Text(content, 14); text.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        text.LineHeight = Design.FontSize(14) * 1.35; text.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        return text;
    }
    private Button EntryAction(string label, string icon, bool destructive = false)
    {
        var button = Design.ToolButton(label, icon); button.Width = button.Height = 28; button.MinWidth = button.MinHeight = 0;
        button.Padding = new Thickness(0); button.CornerRadius = new CornerRadius(6);
        button.Content = Design.Icon(icon, 15, Design.ColorText("text-faint"));
        // The pressed resource already owns the shared sunken brush. WinUI
        // rejects adding that same brush under a second key in this dictionary.
        var hover = Design.Brush(destructive ? "err" : "bg-sunken").Color;
        if (destructive) hover.A = 26;
        button.Resources["ButtonBackgroundPointerOver"] = new SolidColorBrush(hover);
        button.PointerEntered += (_, _) => button.Content = Design.Icon(icon, 15, Design.ColorText(destructive ? "err" : "text"));
        button.PointerExited += (_, _) => button.Content = Design.Icon(icon, 15, Design.ColorText("text-faint"));
        return button;
    }
    private void EditGlobal(string id = "") => RequestLeave(() => { model.Discard(); if (model.EditGlobal(id)) Render(); });
    private TextBox Editor(double height)
    {
        var input = new TextBox { AcceptsReturn = true, Text = model.Content, TextWrapping = TextWrapping.Wrap, Height = height, CornerRadius = new CornerRadius(10), Padding = new Thickness(12, 10, 12, 10),
            Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        Design.BindTypography(input, size: 13);
        ScrollViewer.SetVerticalScrollBarVisibility(input, ScrollBarVisibility.Auto); AutomationProperties.SetName(input, model.NoteName ?? T("memory.proposal.content")); textMenus.Register(input);
        input.TextChanged += (_, _) => model.EditContent(input.Text); return input;
    }
    private void GlobalForm(StackPanel target)
    {
        var body = new StackPanel { Spacing = 8 }; var input = Editor(100); body.Children.Add(NativeDialogVisuals.Input(input, Design));
        var actions = new NativeActionWrap { HorizontalAlignment = HorizontalAlignment.Right, Gap = 8 };
        actions.Children.Add(Action(T("settings.cancel"), () => { RequestLeave(() => { model.Discard(); Render(); }); return Task.CompletedTask; }));
        save = Action(T("memory.global_save"), () => Mutate(model.SaveAsync), primary: true); actions.Children.Add(save); body.Children.Add(actions);
        target.Children.Add(new Border { Child = body, Padding = new Thickness(12), Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(10), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1) });
        input.Loaded += (_, _) => input.StartBringIntoView();
    }
    private async Task OpenNote(string name) { if (await model.OpenNoteAsync(name) && !model.Closed) { Render(); BodyScroll.ChangeView(null, 0, null, true); } }
    private void NoteForm()
    {
        noteEditor = Editor(320); Form.Children.Add(NativeDialogVisuals.Input(noteEditor, Design)); Form.Children.Add(feedbackBox);
        var actions = new NativeActionWrap { HorizontalAlignment = HorizontalAlignment.Right, Gap = 8 };
        deleteNote = Action(T("memory.delete"), () => { Confirm(L("删除此记忆文件？此操作不可撤销。", "Delete this memory file? This cannot be undone."), () => _ = Mutate(model.DeleteNoteAsync)); return Task.CompletedTask; }); actions.Children.Add(deleteNote);
        save = Action(T("memory.save"), () => Mutate(model.SaveAsync), primary: true); actions.Children.Add(save);
        Footer.Children.Add(new Border { Child = actions, Padding = new Thickness(0, 14, 0, 0), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) }); ResizeEditor();
    }
    private void ResizeEditor()
    {
        if (noteEditor == null) return;
        noteEditor.Height = NativeMemoryLayout.EditorHeight(BodyScroll.ActualHeight);
        Footer.Padding = new Thickness(40, 0, 40 + Body.Margin.Right, 36);
    }
    private async Task Mutate(Func<Task<bool>> action) { if (await action() && !model.Closed) Render(); else RefreshControls(); }
    public override void Dispose() { projectMenu?.Hide(); model.Changed -= RefreshControls; textMenus.Dispose(); base.Dispose(); }
}
