using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using static Wisp.ProjectBrowser.NativeSkillsModel;

namespace Wisp.Science.Preview;

internal sealed class NativeSkillsPage : NativeActionPage
{
    private readonly NativeSkillsModel model;
    private readonly SettingsTextMenus textMenus = new();
    private readonly Func<bool, Task<string?>>? pickPath;
    private readonly Action<bool> openStore;
    private readonly StackPanel rows = new();
    private TextBlock summary = new();
    private NativeActionWrap filters = new();
    private readonly List<ComboBox> choices = [];
    private Flyout? addMenu;
    private Grid? removeCopyDialog;
    private Button? removeCopyOpener;
    public event Action<FrameworkElement?>? ModalChanged;
    private string installScope = "global";
    private bool menuOpen, picking;
    private Action? confirmation;
    private Button? enableVisible, disableVisible, retryTags;
    private TextBox? tagInput;
    private ContentControl? fileBody;
    private ContentControl? originBody;
    private Border? filePreview;
    private TextBlock? promoteHint;
    private TextBlock? detailTitle;
    private ScrollViewer? listScroll;
    private int renderGeneration;
    public bool Busy => model.Busy || picking;
    public string Query => model.Query;
    public string Filter => model.Filter;
    public string? ProjectId => model.ProjectId;
    public NativeSkillsPage(INativeSettingsClient client, string? project, WispDesign design, Action close, Action<bool> openStore, Func<bool, Task<string?>>? pickPath, string query = "", string filter = "", string? preferredProject = null)
        : this(new NativeSkillsModel(client, project, preferredProject) { Query = query, Filter = filter }, design, close, openStore, pickPath) { _ = Load(); }
    private NativeSkillsPage(NativeSkillsModel model, WispDesign design, Action close, Action<bool> openStore, Func<bool, Task<string?>>? pickPath)
        : base(design, design.T("settings.nav.skills"), model, close)
    {
        this.model = model; this.openStore = openStore; this.pickPath = pickPath;
        UseSettingsLayout(reserveOverflowScrollbar: true); Body.Padding = new Thickness(24, 18, 24, 22); Form.MaxWidth = double.PositiveInfinity; Form.Spacing = 18; Footer.Visibility = Visibility.Collapsed;
        model.Changed += RefreshControls;
        BodyScroll.SizeChanged += (_, _) => ResizeDocument();
        Body.SizeChanged += (_, _) => ResizeDocument();
    }
    private TextBlock Hint(string text, double size = 13) { var label = Design.Text(text, size); label.Foreground = Design.Brush("text-muted"); return label; }
    private string T(string key) => Design.T(key);
    private string L(string chinese, string english) => Design.L(chinese, english);
    private Button Quiet(string title, Func<Task> action) { var button = Button(title, action); Design.QuietButton(button); return button; }
    private async Task Load(bool rescan = false) { await model.LoadAsync(rescan); if (!model.Closed) Render(); }
    private void RefreshControls()
    {
        SetContentEnabled(!Busy && confirmation == null);
        if (enableVisible != null) enableVisible.IsEnabled = !Busy && model.Visible.Any(r => !B(r, "managed") && !B(r, "enabled"));
        if (disableVisible != null) disableVisible.IsEnabled = !Busy && model.Visible.Any(r => !B(r, "managed") && B(r, "enabled"));
        if (retryTags != null) { retryTags.IsEnabled = !Busy && model.TagsChanged; retryTags.Visibility = model.TagsChanged && model.Error != null ? Visibility.Visible : Visibility.Collapsed; }
        DrawOrigin();
    }
    private void Confirm(string message, Action next)
    {
        confirmation = next; Notices.Children.Clear(); Notices.Children.Add(Hint(message));
        var actions = new NativeActionWrap();
        actions.Children.Add(Button(L("确认", "Confirm"), () => { var action = confirmation; ClearConfirmation(); action?.Invoke(); return Task.CompletedTask; }));
        actions.Children.Add(Button(T("store.cancel"), () => { ClearConfirmation(); return Task.CompletedTask; }));
        Notices.Children.Add(actions); Notices.Visibility = Visibility.Visible; RefreshControls();
    }
    private void ClearConfirmation() { confirmation = null; Notices.Children.Clear(); Notices.Visibility = Visibility.Collapsed; RefreshControls(); }
    public void RequestLeave(Action leave)
    {
        if (Busy || removeCopyDialog != null) return;
        if (model.TagsChanged) Confirm(L("标签尚未保存，放弃这些修改？", "Discard unsaved tag changes?"), leave); else leave();
    }
    public override void HandleEscape()
    {
        if (removeCopyDialog != null) { CloseCopyConfirmation(); return; }
        if (textMenus.CloseTop()) return;
        if (menuOpen) { addMenu?.Hide(); return; }
        if (choices.LastOrDefault(c => c.IsDropDownOpen) is { } choice) { choice.IsDropDownOpen = false; return; }
        if (confirmation != null) { ClearConfirmation(); return; }
        if (Busy) return;
        if (model.Selected != null) { RequestLeave(Back); return; }
        base.HandleEscape();
    }
    private void Back() { model.Back(); Render(resetScroll: true); }
    private void Render(bool resetScroll = false)
    {
        if (model.Closed) return;
        var generation = ++renderGeneration;
        if (listScroll != null) { listScroll.Content = null; listScroll = null; }
        Form.Children.Clear(); Results.Children.Clear(); HeaderActions.Children.Clear(); choices.Clear();
        promoteHint = null; tagInput = null; retryTags = null; fileBody = null; originBody = null; filePreview = null; detailTitle = null; enableVisible = disableVisible = null;
        if (model.Selected == null) SettingsHeading(T("settings.nav.skills"));
        else SettingsBreadcrumb(T("settings.nav.skills"), S(model.Selected, "name"), () => RequestLeave(Back));
        Form.Spacing = model.Selected == null ? 12 : 18;
        Body.Padding = new Thickness(24, model.Selected == null ? 14 : 18, 24, 22);
        if (model.Selected == null) List(); else Detail();
        Design.ApplyTypography(Form); RefreshControls();
        if (resetScroll) DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
        {
            if (model.Closed || generation != renderGeneration) return;
            // ChangeView before the replacement content is measured can retain
            // the old extent/anchor and jump to the bottom of the new list.
            BodyScroll.UpdateLayout(); BodyScroll.ChangeView(null, 0, null, true);
        });
    }
    private Grid Row(FrameworkElement left, FrameworkElement right, double breakpoint = 0)
    {
        var grid = new Grid { ColumnSpacing = 14 };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        left.VerticalAlignment = right.VerticalAlignment = VerticalAlignment.Center; grid.Children.Add(left); Grid.SetColumn(right, 1); grid.Children.Add(right);
        if (breakpoint > 0) grid.SizeChanged += (_, e) =>
        {
            var narrow = e.NewSize.Width < breakpoint;
            grid.RowSpacing = narrow ? 10 : 0;
            Grid.SetColumnSpan(left, narrow ? 2 : 1); Grid.SetRow(right, narrow ? 1 : 0); Grid.SetColumn(right, narrow ? 0 : 1); Grid.SetColumnSpan(right, narrow ? 2 : 1);
            // Keep the second row's available width so fractional native text
            // measurements cannot wrap an action again during arrangement.
            right.HorizontalAlignment = narrow ? HorizontalAlignment.Stretch : HorizontalAlignment.Right;
        };
        return grid;
    }
    private FrameworkElement Badge(string text) => new Border { Child = Hint(text, 10.5), Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(12), Padding = new Thickness(6, 1, 6, 1), VerticalAlignment = VerticalAlignment.Center };
    private string Scope(JsonObject row) => S(row, "scope") is "bundled" or "global" or "project" or "extra" or "plugin" or "custom" ? T("skills.scope." + S(row, "scope")) : S(row, "scope");
    private ToggleSwitch Switch(JsonObject row)
    {
        var toggle = new ToggleSwitch { IsOn = B(row, "enabled"), OnContent = "", OffContent = "", MinWidth = 0 };
        PreferenceToggleSwitch.Apply(toggle, Design); AutomationProperties.SetName(toggle, S(row, "name"));
        toggle.Toggled += async (_, _) => { await model.SetEnabledAsync(S(row, "name"), toggle.IsOn); if (!model.Closed) { if (model.Selected == null) DrawRows(); else Render(); } };
        return toggle;
    }
    private void List()
    {
        summary = new(); filters = new() { Gap = 6 };
        if (!model.Loaded || model.ProjectId == null)
        {
            Form.Children.Add(Hint(model.Error != null ? L("技能尚未载入，请重试。", "Skills could not be loaded. Please retry.") : L("请先创建项目，再配置技能。", "Create a project to configure skills."))); Form.Children.Add(Button(T("skills.reload"), () => Load())); return;
        }
        var search = new TextBox { PlaceholderText = T("skills.search_ph"), Text = model.Query, CornerRadius = new CornerRadius(10), Padding = new Thickness(12, 8, 12, 8), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        AutomationProperties.SetName(search, T("skills.search_ph")); textMenus.Register(search);
        search.TextChanged += (_, _) => { model.Query = search.Text; DrawRows(); };
        var primary = new NativeActionWrap();
        primary.Children.Add(Button(T("store.browse"), () => { openStore(false); return Task.CompletedTask; }));
        primary.Children.Add(Button(T("store.github"), () => { openStore(true); return Task.CompletedTask; }));
        var add = Button(T("skills.add"), () => Task.CompletedTask);
        var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 }; content.Children.Add(Design.Icon("plus", 16, Design.ColorText("clay-strong"))); var addLabel = Design.Text(T("skills.add"), 13); addLabel.Foreground = Design.Brush("clay-strong"); content.Children.Add(addLabel); content.Children.Add(Design.Icon("chevron-down", 16, Design.ColorText("clay-strong"))); add.Content = content;
        add.Background = Design.Brush("clay-soft"); addMenu = InstallMenu(add);
        add.Flyout = addMenu; add.IsEnabled = pickPath != null; primary.Children.Add(add);
        Form.Children.Add(Row(NativeDialogVisuals.Input(search, Design), primary, 800));
        if (model.InstallReceipt is { } receipt)
        {
            var message = ReceiptText(receipt);
            Form.Children.Add(SuccessMessage(message));
        }
        var bulk = new NativeActionWrap();
        enableVisible = Quiet(T("skills.enable_visible"), async () => { await model.SetVisibleEnabledAsync(true); if (!model.Closed) DrawRows(); });
        disableVisible = Quiet(T("skills.disable_visible"), async () => { await model.SetVisibleEnabledAsync(false); if (!model.Closed) DrawRows(); });
        var reload = Quiet(T("skills.reload"), () => Load(true));
        var reloadContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 7 }; reloadContent.Children.Add(Design.Icon("refresh", 16)); reloadContent.Children.Add(Hint(T("skills.reload"), 14)); reload.Content = reloadContent;
        bulk.Children.Add(enableVisible); bulk.Children.Add(disableVisible); bulk.Children.Add(reload);
        foreach (var action in bulk.Children.OfType<Button>()) { action.Padding = new Thickness(8, 5, 8, 5); action.MinHeight = 0; }
        Form.Children.Add(Row(summary, bulk, 630));
        filters.Children.Clear();
        foreach (var (tag, label) in new[] { ("", T("skills.all")), ("__untagged", T("skills.untagged")), ("__enabled", T("skills.enabled")), ("__disabled", T("skills.disabled")) }.Concat(model.AllTags.Select(t => (t, t))))
        {
            var button = Quiet(label, () => { model.Filter = tag; DrawRows(); return Task.CompletedTask; }); button.Tag = tag;
            button.Padding = new Thickness(9, 4, 9, 4); button.CornerRadius = new CornerRadius(999); button.MinHeight = 0;
            button.BorderThickness = new Thickness(1); button.BorderBrush = Design.Brush("border");
            if (button.Content is TextBlock caption) Design.BindTypography(caption, 12.5); filters.Children.Add(button);
        }
        Form.Children.Add(new Border { Child = new ScrollViewer { Content = filters, MaxHeight = 96, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled }, Padding = new Thickness(0, 0, 0, 10), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
        Form.Children.Add(Hint(T("settings.auto_saved_new_session"), 12));
        listScroll = new ScrollViewer { Content = rows, HorizontalContentAlignment = HorizontalAlignment.Stretch, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollMode = ScrollMode.Enabled, IsVerticalScrollChainingEnabled = false };
        listScroll.Loaded += (_, _) => ResizeDocument(); Form.Children.Add(listScroll); DrawRows();
    }
    private void DrawRows()
    {
        rows.Children.Clear(); var visible = model.Visible;
        summary.Text = Design.Localization.Format("skills.summary", ("visible", visible.Length.ToString()), ("enabled", model.Skills.Count(r => B(r, "enabled")).ToString()), ("total", model.Skills.Count.ToString()));
        summary.Foreground = Design.Brush("text-muted"); summary.FontSize = Design.FontSize(12);
        foreach (var button in filters.Children.OfType<Button>())
        {
            var active = (string)button.Tag == model.Filter;
            button.Background = Design.Brush(active ? "clay-soft" : "bg-input");
            button.BorderBrush = active ? ActiveFilterBorder() : Design.Brush("border");
            button.Foreground = Design.Brush(active ? "clay-strong" : "text-muted");
            button.Resources["ButtonBackgroundPointerOver"] = button.Resources["ButtonBackgroundPressed"] = button.Background;
            button.Resources["ButtonBorderBrushPointerOver"] = button.Resources["ButtonBorderBrushPressed"] = active ? button.BorderBrush : Design.Brush("border-strong");
            button.Resources["ButtonForegroundPointerOver"] = button.Resources["ButtonForegroundPressed"] = Design.Brush(active ? "clay-strong" : "text");
            if (button.Content is TextBlock caption) caption.ClearValue(TextBlock.ForegroundProperty);
        }
        foreach (var skill in visible)
        {
            var labels = new NativeActionWrap(); var name = Design.Text(S(skill, "name"), 14); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            labels.Children.Add(name); var scope = Badge(Scope(skill)); ToolTipService.SetToolTip(scope, S(skill, "dir")); labels.Children.Add(scope);
            var copies = Copies(skill).Items.Length;
            if (copies > 1) labels.Children.Add(Badge(Design.Localization.Format("skills.copies_badge", ("count", copies.ToString()))));
            foreach (var tag in Strings(skill["tags"])) labels.Children.Add(Badge(tag));
            var open = Quiet(S(skill, "name"), async () =>
            {
                var name = S(skill, "name");
                var opening = model.OpenAsync(name); Render(resetScroll: true);
                await opening;
                if (model.Closed || S(model.Selected, "name") != name) return;
                Render(resetScroll: true); // File browsing is available while optional metadata loads.
                if (Copies(skill).Promote)
                {
                    await model.ReadInstallLocationsAsync();
                    if (model.Closed || S(model.Selected, "name") != name) return;
                    if (promoteHint != null && model.InstallLocations is { } locations)
                        promoteHint.Text = Design.Localization.Format("skills.promote_hint", ("path", locations.Global));
                }
                await model.LoadInstallSourceAsync();
            });
            open.Content = Row(labels, Design.Icon("chevron-right", 16)); open.HorizontalAlignment = HorizontalAlignment.Stretch; open.HorizontalContentAlignment = HorizontalAlignment.Stretch; open.Padding = new Thickness(0, 16, 0, 16);
            FrameworkElement control = B(skill, "managed") ? Badge(S(skill, "managed_by")) : Switch(skill);
            var row = Row(open, control); row.MinHeight = 66;
            rows.Children.Add(new Border { Child = row, Padding = new Thickness(14, 0, 14, 0), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
        }
        if (visible.Length == 0) rows.Children.Add(Hint(T("skills.empty"))); Design.ApplyTypography(rows); RefreshControls();
    }
    private Microsoft.UI.Xaml.Media.SolidColorBrush ActiveFilterBorder()
    {
        // WebView: color-mix(in srgb, var(--clay) 28%, var(--border-strong)).
        var accent = Design.Brush("clay").Color; var border = Design.Brush("border-strong").Color;
        byte Channel(byte first, byte second) => (byte)Math.Round(first * .28 + second * .72);
        return new(Windows.UI.Color.FromArgb(255, Channel(accent.R, border.R), Channel(accent.G, border.G), Channel(accent.B, border.B)));
    }
    private Flyout InstallMenu(Button opener)
    {
        var menu = new Flyout { Placement = FlyoutPlacementMode.BottomEdgeAlignedRight };
        var style = new Style(typeof(FlyoutPresenter));
        style.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(10)));
        style.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(8)));
        style.Setters.Add(new Setter(Control.BackgroundProperty, Design.Brush("bg-elev")));
        style.Setters.Add(new Setter(Control.BorderBrushProperty, Design.Brush("border-strong")));
        menu.FlyoutPresenterStyle = style;
        var panel = new StackPanel { Spacing = 6 }; menu.Content = panel;
        panel.Children.Add(Hint(T("skills.install_location"), 12));
        var radios = new List<(string Scope, RadioButton Button, Border Row, TextBlock Path)>();
        var group = Guid.NewGuid().ToString();
        var noteText = Hint(T("skills.install_scope.project_warning"), 12); noteText.TextWrapping = TextWrapping.Wrap;
        var note = new Border { Child = noteText, CornerRadius = new(6), Padding = new(9,7,9,7), Background = Design.Brush("warn-soft") };
        void ShowScope()
        {
            foreach (var choice in radios)
            {
                choice.Button.IsChecked = choice.Scope == installScope;
                choice.Row.Background = Design.Brush(choice.Scope == installScope ? "clay-soft" : "bg-elev");
            }
            note.Visibility = installScope == "project" ? Visibility.Visible : Visibility.Collapsed;
        }
        foreach (var scope in new[] { "global", "project" })
        {
            var caption = T("skills.install_scope." + scope);
            var title = Design.Text(caption,13); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            var path = Design.Text("",11,"text-faint"); path.TextWrapping = TextWrapping.Wrap;
            var labels = new StackPanel { Spacing = 2 }; labels.Children.Add(title); labels.Children.Add(path);
            var radio = new RadioButton { GroupName = group, Content = labels,
                HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                Padding = new Thickness(4,3,4,3), MinWidth = 0 };
            AutomationProperties.SetName(radio,caption);
            AutomationProperties.SetAutomationId(radio,"skill-install-" + scope);
            var row = new Border { Child = radio, CornerRadius = new(6), Padding = new(8,2,8,2) };
            radios.Add((scope,radio,row,path)); panel.Children.Add(row);
            radio.Checked += (_,_) => { if (installScope != scope) { installScope = scope; ShowScope(); } };
        }
        panel.Children.Add(note); ShowScope();
        var error = Hint(T("skills.locations_error"),12); error.TextWrapping = TextWrapping.Wrap;
        error.Visibility = Visibility.Collapsed; panel.Children.Add(error);
        var actions = new List<Button>();
        foreach (var (folder,key) in new[] { (false,"skills.add_file"), (true,"skills.add_folder") })
        {
            var action = Button(T(key), async () =>
            {
                var scope = installScope; menu.Hide(); await Import(folder,scope);
            });
            action.HorizontalAlignment = HorizontalAlignment.Stretch;
            action.HorizontalContentAlignment = HorizontalAlignment.Left;
            action.Padding = new(12,8,12,8); action.MinHeight = 0;
            action.Background = Design.Brush("bg-input"); actions.Add(action); panel.Children.Add(action);
        }
        var openCount = 0;
        menu.Opening += async (_,_) =>
        {
            menuOpen = true; var ticket = ++openCount;
            panel.Width = Math.Clamp((opener.XamlRoot?.Size.Width ?? 400) - 54, 100, 338);
            error.Visibility = Visibility.Collapsed;
            foreach (var row in radios) row.Path.Text = "";
            foreach (var button in actions) button.IsEnabled = false;
            var ready = await model.ReadInstallLocationsAsync();
            if (model.Closed || openCount != ticket || addMenu != menu) return;
            if (ready && model.InstallLocations is { } paths)
            {
                foreach (var row in radios)
                {
                    row.Path.Text = row.Scope == "global" ? paths.Global : paths.Project;
                    AutomationProperties.SetHelpText(row.Button, row.Path.Text);
                }
                foreach (var button in actions) button.IsEnabled = true;
            }
            else error.Visibility = Visibility.Visible;
        };
        menu.Closed += (_,_) => { ++openCount; if (addMenu == menu) menuOpen = false; };
        return menu;
    }
    private async Task Import(bool folder, string scope)
    {
        if (Busy || pickPath == null) return;
        picking = true; RefreshControls(); string? path;
        try { path = await pickPath(folder); } catch (Exception ex) { model.Fail(ex.Message); return; } finally { picking = false; RefreshControls(); }
        if (model.Closed || string.IsNullOrWhiteSpace(path)) return;
        if (!folder && Path.GetFileName(path) != "SKILL.md" && !Path.GetExtension(path).Equals(".zip", StringComparison.OrdinalIgnoreCase)) { model.Fail(L("请选择 SKILL.md 或 ZIP 文件。", "Choose a SKILL.md or ZIP file.")); return; }
        await model.InstallAsync(path, scope); if (!model.Closed) Render();
    }
    private void Detail()
    {
        var skill = model.Selected!; var heading = new NativeActionWrap { Gap = 12 }; var name = Design.Text(S(skill, "name"), 22); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        detailTitle = name;
        heading.Children.Add(name); heading.Children.Add(Badge(Scope(skill)));
        var actions = new NativeActionWrap(); if (!B(skill, "managed")) actions.Children.Add(Switch(skill));
        if (CanRemove(skill) && !model.ScopeNeedsRead) actions.Children.Add(Quiet(T("skills.remove"), () => { Confirm(Design.Localization.Format("skills.remove_confirm", ("skill", S(skill, "name"))), () => _ = Remove()); return Task.CompletedTask; }));
        Form.Children.Add(Row(heading, actions, 400));
        if (B(skill, "managed")) Form.Children.Add(Hint(Design.Localization.Format("skills.managed_by", ("plugin", S(skill, "managed_by")))));
        var description = Hint(S(skill, "description"), 14); description.LineHeight = Design.FontSize(14) * 1.65; description.LineStackingStrategy = LineStackingStrategy.BlockLineHeight; Form.Children.Add(description);
        var directory = Hint(S(skill, "dir"), 12); directory.Foreground = Design.Brush("text-faint"); Form.Children.Add(directory);
        DrawScope(skill);
        originBody = new ContentControl { HorizontalContentAlignment = HorizontalAlignment.Stretch }; Form.Children.Add(originBody); DrawOrigin();
        var tags = new TextBox { PlaceholderText = T("skills.tags_placeholder"), Text = model.TagsText, Width = 360, HorizontalAlignment = HorizontalAlignment.Left,
            Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(8), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), Padding = new Thickness(12, 10, 12, 10) };
        Design.BindTypography(tags, size: 12); AutomationProperties.SetName(tags, T("skills.edit_tags"));
        tagInput = tags;
        textMenus.Register(tags); tags.TextChanged += (_, _) => { if (tagInput == tags) model.EditTags(tags.Text); };
        tags.LostFocus += async (_, _) => { if (tagInput == tags && model.TagsChanged && confirmation == null) await SaveTags(); };
        var tagsField = new StackPanel { Spacing = 8, Margin = new Thickness(0, 18, 0, 0) }; tagsField.Children.Add(Hint(T("skills.edit_tags"), 12)); tagsField.Children.Add(tags);
        tagsField.SizeChanged += (_, e) => tags.Width = Math.Min(360, e.NewSize.Width); Form.Children.Add(tagsField); retryTags = Button(T("skills.retry"), SaveTags); Form.Children.Add(retryTags);
        var select = new ComboBox { Header = Hint(T("skills.select_file"), 12), HorizontalAlignment = HorizontalAlignment.Right, Background = Design.Brush("bg-sunken"),
            BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Padding = new Thickness(12, 8, 12, 8) };
        AutomationProperties.SetName(select, T("skills.select_file")); Design.BindTypography(select, size: 13);
        foreach (var path in model.Files) select.Items.Add(path); select.SelectedItem = model.File;
        select.SelectionChanged += async (_, _) => { if (select.SelectedItem is string path) { var reading = model.ReadAsync(path); DrawFile(); await reading; if (!model.Closed) DrawFile(); } };
        choices.Add(select); var filesLabel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 }; var filesTitle = Design.Text(T("skills.files"), 15); filesTitle.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; filesLabel.Children.Add(filesTitle); filesLabel.Children.Add(Hint(model.Files.Count.ToString(), 12.5));
        var filesHeading = Row(filesLabel, select); filesHeading.Margin = new Thickness(0, 12, 0, 0); filesHeading.SizeChanged += (_, e) => select.MaxWidth = e.NewSize.Width * .65; Form.Children.Add(filesHeading);
        fileBody = new ContentControl { HorizontalContentAlignment = HorizontalAlignment.Stretch }; Form.Children.Add(fileBody); DrawFile();
    }
    private string ReceiptText(JsonObject receipt)
    {
        var scope = S(receipt, "scope");
        var message = Design.Localization.Format("skills.receipt", ("skill", S(receipt, "name")),
            ("scope", T("skills.scope." + scope)), ("path", S(receipt, "dir")), ("reach", T(scope == "project" ? "skills.receipt.project_only" : "skills.receipt.all_projects")));
        if (receipt["shadowed_by"] is JsonObject winner)
            message += " " + Design.Localization.Format("skills.receipt.shadowed", ("scope", Scope(winner)), ("path", S(winner, "dir")));
        return message;
    }
    private void DrawScope(JsonObject skill)
    {
        var scope = S(skill, "scope"); var copies = Copies(skill);
        var section = new StackPanel { Spacing = 10 };
        if (scope is "project" or "global" or "bundled")
        {
            var lifetime = Hint(T("skills.lifetime." + scope), 12.5);
            section.Children.Add(scope == "project" ? new Border { Child = lifetime, Background = Design.Brush("warn-soft"), CornerRadius = new(6), Padding = new(9, 7, 9, 7) } : lifetime);
        }
        if (copies.Items.Length > 1)
        {
            var title = Design.Text(T("skills.copies"), 14); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            section.Children.Add(title); section.Children.Add(Hint(T("skills.copies_hint"), 12.5));
            var list = new StackPanel { Spacing = 6 };
            foreach (var copy in copies.Items)
            {
                var row = new StackPanel { Spacing = 4 };
                var badges = new NativeActionWrap { Gap = 10, VerticallyCentered = true };
                badges.Children.Add(Badge(T("skills.scope." + copy.Scope)));
                var status = Hint(T(copy.Effective ? "skills.copy.active" : "skills.copy.shadowed"), 12); status.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
                if (copy.Effective) status.Foreground = Design.Brush("clay-strong"); badges.Children.Add(status);
                if (!copy.Effective && copy.SameAsEffective) badges.Children.Add(Hint(T("skills.copy.identical"), 12));
                row.Children.Add(badges); var path = Hint(copy.Directory, 12); path.IsTextSelectionEnabled = true; path.TextWrapping = TextWrapping.Wrap; row.Children.Add(path);
                if (copy.Scope == "project") row.Children.Add(Hint(T("skills.lifetime.project"), 11));
                list.Children.Add(new Border { Child = row, BorderThickness = new(1), BorderBrush = copy.Effective ? Tint("clay", "border", .4) : Design.Brush("border"), Padding = new(10,8,10,8), CornerRadius = new(8) });
            }
            section.Children.Add(list);
        }
        void ActionRow(string key, string icon, string? hint, Func<Task> action)
        {
            var row = new NativeActionWrap { Gap = 12, VerticallyCentered = true };
            var button = Quiet(T(key), action); var label = new StackPanel { Spacing = 6, Orientation = Orientation.Horizontal };
            label.Children.Add(Design.Icon(icon, 15)); label.Children.Add(Design.Text(T(key), 13)); button.Content = label;
            AutomationProperties.SetName(button, T(key)); button.BorderThickness = new(1); button.BorderBrush = Design.Brush("border-strong"); button.CornerRadius = new(8); button.Padding = new(12,6,12,6);
            if (key == "skills.remove_project_copy") removeCopyOpener = button;
            row.Children.Add(button);
            if (key == "skills.promote") { promoteHint = Hint(hint ?? "", 12); row.Children.Add(promoteHint); }
            else if (!string.IsNullOrWhiteSpace(hint)) row.Children.Add(Hint(hint, 12));
            section.Children.Add(row);
        }
        if (model.ScopeNeedsRead)
            ActionRow("skills.reload", "refresh", null, async () => { await model.RefreshScopeAsync(); if (!model.Closed) Render(); });
        else
        {
            if (copies.Promote) ActionRow("skills.promote", "upload", model.InstallLocations is { } paths ? Design.Localization.Format("skills.promote_hint", ("path", paths.Global)) : null, () => ChangeScope(true));
            if (copies.RemoveProject) ActionRow("skills.remove_project_copy", "trash", T(copies.Duplicate ? "skills.duplicate_hint" : "skills.promoted_remove_hint"), ConfirmProjectCopyRemoval);
        }
        if (model.PromotionReceipt is { } receipt) section.Children.Add(SuccessMessage(ReceiptText(receipt)));
        if (model.ProjectCopyRemoved) section.Children.Add(SuccessMessage(T("skills.project_copy_removed")));
        if (section.Children.Count > 0) Form.Children.Add(section);
    }
    private async Task ChangeScope(bool promote)
    {
        await model.ChangeScopeAsync(promote, ex =>
        {
            var error = ex.Message;
            const string conflict = "skill_global_exists:";
            if (error.StartsWith(conflict, StringComparison.Ordinal))
            {
                var path = error[conflict.Length..];
                return L($"全局目录中已有同名技能：{path}。未复制任何文件，请先处理该副本。", $"A global skill with this name already exists at {path}. Nothing was copied; resolve that copy first.");
            }
            return error switch
            {
                "skill_not_in_project" => L("当前项目中没有这个技能的项目副本。", "This project has no project copy of this skill."),
                "skill_project_copy_is_only_copy" => L("项目副本是唯一的一份，请先提升为全局再删除。", "The project copy is the only copy. Promote it to global before removing it."),
                _ => L("无法确认操作结果。请重新加载副本列表。", "Could not confirm the change. Reload the copies before trying again.") + " " + error
            };
        }); if (!model.Closed) Render();
    }
    private Microsoft.UI.Xaml.Media.SolidColorBrush Tint(string foreground, string background, double weight)
    {
        var fg = Design.Brush(foreground).Color; var bg = Design.Brush(background).Color;
        byte Mix(byte f, byte b) => (byte)Math.Round(weight * f + (1 - weight) * b);
        return new(Windows.UI.Color.FromArgb(Mix(fg.A, bg.A), Mix(fg.R, bg.R), Mix(fg.G, bg.G), Mix(fg.B, bg.B)));
    }
    private FrameworkElement SuccessMessage(string message)
    {
        var label = Hint(message, 12); label.Foreground = Design.Brush("ok");
        label.LineHeight = Design.FontSize(12) * 1.45; label.TextWrapping = TextWrapping.Wrap;
        AutomationProperties.SetLiveSetting(label, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Polite);
        return new Border { Child = label, Background = Tint("ok", "bg-input", .08), BorderBrush = Tint("ok", "border-strong", .35),
            BorderThickness = new(1), Padding = new(10,8,10,8), CornerRadius = new(8) };
    }
    private void CloseCopyConfirmation()
    {
        if (removeCopyDialog == null) return;
        removeCopyDialog = null; ModalChanged?.Invoke(null);
        if (removeCopyOpener is { IsLoaded: true } opener) opener.Focus(FocusState.Programmatic);
    }
    private Task ConfirmProjectCopyRemoval()
    {
        if (Busy || model.Selected == null || model.TagsChanged || removeCopyDialog != null) return Task.CompletedTask;
        var skill = model.Selected; var overlay = new Grid(); removeCopyDialog = overlay;
        NativeDialogVisuals.Backdrop(overlay, Design);
        var body = new StackPanel { Padding = new(22), Spacing = 14 };
        var title = Design.Text(T("confirm.title"), 17); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; body.Children.Add(title);
        var message = Hint(Design.Localization.Format("skills.remove_project_copy_confirm", ("skill", S(skill, "name"))), 13);
        message.LineHeight = Design.FontSize(13) * 1.55; body.Children.Add(message);
        var footer = new NativeActionWrap { Gap = 10, RightAligned = true, Margin = new(0,4,0,0) };
        var cancel = Button(T("store.cancel"), () => { CloseCopyConfirmation(); return Task.CompletedTask; });
        var remove = Button(T("skills.remove_project_copy"), async () => { CloseCopyConfirmation(); if (!model.Closed && ReferenceEquals(model.Selected, skill)) await ChangeScope(false); });
        foreach (var button in new[] { cancel, remove })
        {
            Design.ActionButton(button, primary: button == remove); button.Padding = new(16,9,16,9);
            if (button.Content is TextBlock label) Design.BindTypography(label, 13.5);
            footer.Children.Add(button);
        }
        body.Children.Add(footer);
        var scroll = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var card = new Border { Child = scroll, CornerRadius = new(16), Background = Design.Brush("bg-elev"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new(1),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = Microsoft.UI.Xaml.Input.KeyboardNavigationMode.Cycle };
        overlay.Children.Add(card); AutomationProperties.SetName(overlay, T("confirm.title"));
        void Resize() { card.Width = Math.Min(480, Math.Max(0, overlay.ActualWidth - 32)); card.MaxHeight = Math.Max(0, overlay.ActualHeight - 40); }
        overlay.SizeChanged += (_,_) => Resize(); overlay.Loaded += (_,_) => { Resize(); cancel.Focus(FocusState.Programmatic); };
        ModalChanged?.Invoke(overlay); Design.ApplyTypography(body);
        return Task.CompletedTask;
    }
    private async Task SaveTags()
    {
        if (await model.SaveTagsAsync() && tagInput != null && !model.Closed) tagInput.Text = model.TagsText;
        RefreshControls();
    }
    private async Task Remove() { await model.RemoveAsync(); if (!model.Closed) Render(); }
    private void DrawOrigin()
    {
        if (originBody == null) return;
        var body = new StackPanel { Spacing = 10 };
        if (model.InstallSource is { } source)
        {
            var title = Design.Text(T("store.installed_source"), 15); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            body.Children.Add(title); body.Children.Add(NativeSkillSourceView.Create(source, Design));
        }
        if (model.SourceError is { } error)
        {
            body.Children.Add(Hint(L("未能读取安装来源。", "Could not read the installation source.")));
            var retry = Quiet(T("skills.retry"), model.LoadInstallSourceAsync); retry.IsEnabled = !model.SourceLoading;
            ToolTipService.SetToolTip(retry, error); body.Children.Add(retry);
        }
        originBody.Content = body; originBody.Visibility = body.Children.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
    }
    private void DrawFile()
    {
        if (fileBody == null) return;
        filePreview = null;
        var body = new StackPanel { Spacing = 14 }; fileBody.Content = body;
        var toolbar = new NativeActionWrap();
        if (model.IsMarkdown)
        {
            foreach (var (source, label) in new[] { (false, T("skills.preview")), (true, T("skills.source")) })
            {
                var button = Quiet(label, () => { model.SetSourceMode(source); DrawFile(); return Task.CompletedTask; });
                button.Padding = new Thickness(10, 5, 10, 5); button.CornerRadius = new CornerRadius(6); button.MinHeight = 0;
                button.BorderThickness = new Thickness(1); button.BorderBrush = Design.Brush("border");
                button.Background = Design.Brush(model.SourceMode == source ? "bg-sunken" : "bg-app");
                if (button.Content is TextBlock caption) { Design.BindTypography(caption, 12); caption.Foreground = Design.Brush(model.SourceMode == source ? "text" : "text-muted"); }
                toolbar.Children.Add(button);
            }
        }
        body.Children.Add(Row(Hint(model.File ?? "", 12), toolbar));
        if (model.Content == null)
        {
            if (model.Busy)
            {
                var loading = new Grid { ColumnSpacing = 7 }; loading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); loading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
                loading.Children.Add(NativeLoadingVisuals.Spinner(Design, 16)); var label = Hint(T("skills.loading")); Grid.SetColumn(label, 1); loading.Children.Add(label);
                body.Children.Add(new Border { Child = loading, Padding = new Thickness(14), CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-sunken") });
            }
            else body.Children.Add(Hint(model.Error != null ? L("未能读取技能文件。", "Could not read the skill file.") : T("skills.no_files")));
            if (model.Error != null) body.Children.Add(Button(T("skills.retry"), async () => { if (model.File is { } path) await model.ReadAsync(path); else await model.LoadFilesAsync(); if (!model.Closed) Render(); }));
        }
        else
        {
            FrameworkElement content;
            if (model.IsMarkdown && !model.SourceMode) content = new ScrollViewer { Content = TranscriptView.RenderMarkdown(MarkdownBody(model.Content), Design, skillDocument: true), HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            else
            {
                var source = new TextBox { AcceptsReturn = true, Text = model.Content, IsReadOnly = true, TextWrapping = TextWrapping.NoWrap, BorderThickness = new Thickness(0), Background = Design.Brush("bg-sunken"), Padding = new Thickness(0) };
                Design.BindTypography(source, code: true); ScrollViewer.SetHorizontalScrollBarVisibility(source, ScrollBarVisibility.Auto); ScrollViewer.SetHorizontalScrollMode(source, ScrollMode.Enabled);
                AutomationProperties.SetName(source, T("skills.source")); textMenus.Register(source); content = source;
            }
            var preview = new Border { Child = content, CornerRadius = new CornerRadius(12), Padding = new Thickness(24), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), Background = Design.Brush(model.SourceMode || !model.IsMarkdown ? "bg-sunken" : "bg-app") };
            filePreview = preview; preview.Loaded += (_, _) => ResizeDocument(); body.Children.Add(preview);
        }
        Design.ApplyTypography(body);
    }
    private void ResizeDocument()
    {
        if (listScroll is { IsLoaded: true } catalog && BodyScroll.ViewportHeight > 0)
        {
            var listTop = catalog.TransformToVisual(Body).TransformPoint(new Windows.Foundation.Point()).Y;
            catalog.MaxHeight = NativeSettingsLayout.SkillListHeight(BodyScroll.ViewportHeight, listTop, Body.Padding.Bottom);
        }
        var compact = (XamlRoot?.Size.Width ?? ActualWidth) <= 760;
        if (detailTitle != null) Design.BindTypography(detailTitle, compact ? 18 : 22);
        if (filePreview is not { IsLoaded: true } preview || BodyScroll.ViewportHeight <= 0) return;
        var padding = compact ? 14 : 24; preview.Padding = new Thickness(padding);
        var top = preview.TransformToVisual(Body).TransformPoint(new Windows.Foundation.Point()).Y;
        preview.MaxHeight = NativeSettingsLayout.SkillDocumentHeight(BodyScroll.ViewportHeight, top, padding, Body.Padding.Bottom);
    }
    public override void Dispose() { CloseCopyConfirmation(); addMenu?.Hide(); textMenus.Dispose(); model.Changed -= RefreshControls; base.Dispose(); }
}
