using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using static Wisp.ProjectBrowser.NativeSkillsModel;

namespace Wisp.Science.Preview;

internal sealed class NativeSkillStorePage : NativeActionPage
{
    private readonly NativeSkillStoreModel model;
    private readonly Action close;
    private readonly SettingsTextMenus menus = new();
    private readonly List<Control> previewControls = [];
    private ComboBox? tagChoice;
    private ContentControl? catalogHost, candidatesHost;
    private TextBlock? count, dialogError;
    private Button? discover;
    private Grid? confirmation;
    private readonly List<Control> confirmationControls = [];
    public event Action<FrameworkElement?>? ModalChanged;
    public bool Busy => model.Busy;
    public NativeSkillStorePage(INativeSettingsClient client, string? project, bool github, WispDesign design, Action close)
        : this(new NativeSkillStoreModel(client, project, github, design.Localization), design, close) { _ = Load(); }
    private NativeSkillStorePage(NativeSkillStoreModel model, WispDesign design, Action close) : base(design, design.T("settings.nav.skills"), model, close)
    {
        this.model = model; this.close = close;
        UseSettingsLayout(); Body.Padding = new Thickness(24, 18, 24, 22); Form.MaxWidth = double.PositiveInfinity; Form.Spacing = 14; Footer.Visibility = Visibility.Collapsed;
        model.Changed += Refresh; Render();
    }
    private TextBlock Text(string value, double size = 14, bool bold = false)
    {
        var text = Design.Text(value, size); if (bold) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        return text;
    }
    private TextBlock Hint(string value) { var text = Text(value, 12); text.Foreground = Design.Brush("text-muted"); return text; }
    private string T(string key) => Design.T(key);
    private void StoreButton(Button button)
    {
        button.Padding = new Thickness(12, 8, 12, 8); button.CornerRadius = new CornerRadius(7); button.MinHeight = 0;
        button.Background = Design.Brush("bg-input"); button.BorderBrush = Design.Brush("border-strong");
        if (button.Content is TextBlock label) Design.BindTypography(label, 13);
    }
    private Button ActionButton(string label, Func<Task> action, bool previewLock = true)
    {
        var button = Button(label, action); StoreButton(button); if (previewLock) previewControls.Add(button); return button;
    }
    private Button ActionButton(string label, Action action, bool previewLock = true) => ActionButton(label, () => { action(); return Task.CompletedTask; }, previewLock);
    private TextBox Input(string label, string text, Action<string> changed, string placeholder = "")
    {
        var input = new TextBox { Text = text, PlaceholderText = placeholder, CornerRadius = new CornerRadius(10), Padding = new Thickness(11, 9, 11, 9), HorizontalAlignment = HorizontalAlignment.Stretch,
            Background = Design.Brush("bg-input"), Foreground = Design.Brush("text"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        Design.BindTypography(input, size: 14);
        AutomationProperties.SetName(input, label.Length > 0 ? label : placeholder); menus.Register(input);
        input.TextChanged += (_, _) => changed(input.Text); previewControls.Add(input); return input;
    }
    private FrameworkElement Field(string label, TextBox input)
    {
        var field = new StackPanel { Spacing = 6 }; field.Children.Add(Hint(label)); field.Children.Add(NativeDialogVisuals.Input(input, Design)); return field;
    }
    private async Task Load(bool refresh = false) { await model.LoadCatalogAsync(refresh); if (!model.Closed) Render(); }
    private void Refresh()
    {
        SetContentEnabled(!model.Busy && !model.Confirming);
        foreach (var control in previewControls) control.IsEnabled = !model.Busy && !model.Previewing && !model.Confirming;
        if (discover != null) discover.IsEnabled = !model.Busy && !model.Previewing && !model.Confirming && model.SourceUrl.Trim().Length > 0;
        foreach (var control in confirmationControls) control.IsEnabled = !model.Busy;
        if (dialogError != null) { dialogError.Text = model.Error ?? ""; dialogError.Visibility = model.Error == null ? Visibility.Collapsed : Visibility.Visible; }
    }
    public void RequestLeave(Action leave)
    {
        if (model.Busy || model.Confirming) return;
        if (model.Previewing) model.CancelPreview(); leave();
    }
    public override void HandleEscape()
    {
        if (menus.CloseTop()) return;
        if (tagChoice?.IsDropDownOpen == true) { tagChoice.IsDropDownOpen = false; return; }
        Back();
    }
    private void Back()
    {
        if (model.Busy) return;
        var modal = model.Confirming;
        if (!model.Back()) { close(); return; }
        if (modal) { CloseConfirmation(); Refresh(); } else Render();
    }
    private void Render()
    {
        if (model.Closed) return;
        menus.CloseTop(); Form.Children.Clear(); Results.Children.Clear(); HeaderActions.Children.Clear(); previewControls.Clear(); tagChoice = null; discover = null;
        SettingsHeading(T("settings.nav.skills"));
        var top = new Grid { ColumnSpacing = 12 };
        top.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); top.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); top.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var back = ActionButton(T("store.back"), Back, false); var backContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 }; backContent.Children.Add(Design.Icon("chevron-left", 18)); backContent.Children.Add(Text(T("store.back"), 12.5)); back.Content = backContent; top.Children.Add(back);
        var title = Text(T("store.title"), 17, true); title.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(title, 1); top.Children.Add(title);
        var guide = Link(T("store.guide"), "https://github.com/xuzhougeng/wisp-science/blob/main/docs/skill-authoring.md", toolbar: true); Grid.SetColumn(guide, 2); top.Children.Add(guide);
        Form.Children.Add(top); Form.Children.Add(Hint(T("store.scope")));
        if (model.Selected is { } selected) Preview(selected); else Browse();
        if (!string.IsNullOrEmpty(model.Success)) Form.Children.Add(Text(model.Success));
        if (!string.IsNullOrEmpty(model.PreviewError)) Form.Children.Add(Text(model.PreviewError));
        Design.ApplyTypography(Form); Refresh();
    }
    private FrameworkElement Link(string label, string url, bool toolbar = false)
    {
        if (!Uri.TryCreate(url, UriKind.Absolute, out var uri) || uri.Scheme is not ("http" or "https")) return Text(label);
        var copy = Text(label, toolbar ? 12.5 : 13); copy.Foreground = Design.Brush(toolbar ? "text-muted" : "clay");
        if (!toolbar) copy.TextDecorations = Windows.UI.Text.TextDecorations.Underline;
        var link = new HyperlinkButton { Content = copy, NavigateUri = uri, Padding = toolbar ? new Thickness(12, 7, 12, 7) : new Thickness(0),
            BorderThickness = new Thickness(toolbar ? 1 : 0), BorderBrush = Design.Brush("border-strong"), CornerRadius = new CornerRadius(10),
            Background = toolbar ? Design.Brush("bg-input") : null, HorizontalContentAlignment = HorizontalAlignment.Left };
        AutomationProperties.SetName(link, label); ToolTipService.SetToolTip(link, url); return link;
    }
    private FrameworkElement Badge(string text) => new Border { Child = Hint(text), CornerRadius = new CornerRadius(9), Padding = new Thickness(6, 2, 6, 2), Background = Design.Brush("bg-sunken"), HorizontalAlignment = HorizontalAlignment.Left };
    private TextBlock Code(string value)
    {
        var text = Text(value, 13); text.FontFamily = Design.Font(true); return text;
    }
    private FrameworkElement Status(string value, bool ok)
    {
        var color = Design.Brush(ok ? "ok" : "err"); var copy = Text(value, 12); copy.Foreground = color;
        SolidColorBrush Blend(string background, double amount)
        {
            var first = color.Color; var second = Design.Brush(background).Color;
            byte Channel(byte a, byte b) => (byte)Math.Round(a * amount + b * (1 - amount));
            return new(Windows.UI.Color.FromArgb(255, Channel(first.R, second.R), Channel(first.G, second.G), Channel(first.B, second.B)));
        }
        return new Border { Child = copy, Padding = new Thickness(10, 8, 10, 8), CornerRadius = new CornerRadius(10),
            BorderThickness = new Thickness(1), BorderBrush = Blend("border-strong", .35), Background = Blend("bg-input", .08) };
    }
    private FrameworkElement Tiles(IEnumerable<FrameworkElement> items, double minWidth, double gap = 14)
    {
        var grid = new Grid { ColumnSpacing = gap, RowSpacing = gap }; foreach (var item in items) grid.Children.Add(item);
        var oldColumns = 0;
        void Resize(double width)
        {
            var columns = Math.Max(1, Math.Min(grid.Children.Count, (int)((width + gap) / (minWidth + gap)))); if (columns == oldColumns) return; oldColumns = columns;
            grid.ColumnDefinitions.Clear(); grid.RowDefinitions.Clear();
            for (var i = 0; i < columns; i++) grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            for (var i = 0; i < (grid.Children.Count + columns - 1) / columns; i++) grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
            for (var i = 0; i < grid.Children.Count; i++) { Grid.SetColumn((FrameworkElement)grid.Children[i], i % columns); Grid.SetRow((FrameworkElement)grid.Children[i], i / columns); }
        }
        Resize(600); grid.SizeChanged += (_, e) => Resize(e.NewSize.Width); return grid;
    }
    private void Browse()
    {
        var tabs = new NativeActionWrap { Gap = 6 };
        var community = ActionButton(T("store.browse"), () => { model.SetMode(false); Render(); }); var github = ActionButton(T("store.github"), () => { model.SetMode(true); Render(); });
        foreach (var tab in new[] { community, github })
        {
            tab.Padding = new Thickness(10, 7, 10, 7); tab.CornerRadius = new CornerRadius(6); tab.BorderBrush = Design.Brush("border");
            if (tab.Content is TextBlock label) { Design.BindTypography(label, 12.5); label.Foreground = Design.Brush("text-muted"); }
        }
        var active = model.Github ? github : community; active.Background = Design.Brush("clay-soft");
        if (active.Content is TextBlock activeLabel) activeLabel.Foreground = Design.Brush("clay-strong"); tabs.Children.Add(community); tabs.Children.Add(github);
        Form.Children.Add(new Border { Child = tabs, Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), Padding = new Thickness(10, 8, 10, 8), CornerRadius = new CornerRadius(10) });
        if (!model.Github)
        {
            var marketplaces = Tiles(NativeSkillStoreModel.Marketplaces.Select((market, index) =>
            {
                var content = new StackPanel { Spacing = 7 }; var heading = new Grid(); heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
                heading.Children.Add(Text(market.Name, 13, true)); var icon = Design.Icon("chevron-right", 16); Grid.SetColumn(icon, 1); heading.Children.Add(icon); content.Children.Add(heading); content.Children.Add(Hint(T(market.DescriptionKey)));
                var button = ActionButton(market.Name, () => PreviewMarketplace(index)); button.Content = content; button.Padding = new Thickness(14, 12, 14, 12); button.HorizontalAlignment = HorizontalAlignment.Stretch; button.VerticalAlignment = VerticalAlignment.Stretch; button.HorizontalContentAlignment = HorizontalAlignment.Stretch;
                button.CornerRadius = new CornerRadius(10); button.BorderBrush = Design.Brush(model.Marketplace == index ? "clay" : "border");
                if (model.Marketplace == index) button.Background = Design.Brush("clay-soft"); return (FrameworkElement)button;
            }), 230, 10); marketplaces.Margin = new Thickness(0, 14, 0, 14); AutomationProperties.SetName(marketplaces, T("store.marketplaces")); Form.Children.Add(marketplaces);
        }
        if (model.Marketplace is { } index)
        {
            var heading = new NativeActionWrap(); heading.Children.Add(Text(NativeSkillStoreModel.Marketplaces[index].Name, 16, true)); heading.Children.Add(ActionButton(T("store.refresh_source"), () => PreviewMarketplace(index))); Form.Children.Add(heading);
            Form.Children.Add(Hint(T("store.marketplace_notice")));
        }
        else if (!model.Github) Community();
        else
        {
            var fields = new StackPanel { Spacing = 12, Margin = new Thickness(0, 16, 0, 16) };
            fields.Children.Add(Field(T("store.url"), Input(T("store.url"), model.SourceUrl, value => { model.EditSource(value, model.ExactRef); DrawCandidates(); }, "https://github.com/owner/repository/tree/ref/skills/example")));
            fields.Children.Add(Field(T("store.exact_ref"), Input(T("store.exact_ref"), model.ExactRef, value => { model.EditSource(model.SourceUrl, value); DrawCandidates(); })));
            fields.Children.Add(Hint(T("store.ref_help")));
            discover = ActionButton(T("store.discover"), PreviewUser); discover.HorizontalAlignment = HorizontalAlignment.Left; fields.Children.Add(discover); Form.Children.Add(fields);
        }
        if (model.Previewing)
        {
            var loading = new Grid { ColumnSpacing = 12 };
            loading.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); loading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); loading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            loading.Children.Add(NativeLoadingVisuals.Spinner(Design, 24));
            var copy = new StackPanel { Spacing = 3, VerticalAlignment = VerticalAlignment.Center };
            copy.Children.Add(Text(T("store.loading_title"), 13, true)); var hint = Hint(T("store.loading")); hint.LineHeight = Design.FontSize(12) * 1.5; copy.Children.Add(hint); Grid.SetColumn(copy, 1); loading.Children.Add(copy);
            var cancel = ActionButton(T("store.cancel"), () => { model.CancelPreview(); Render(); }, false); cancel.Padding = new Thickness(12, 7, 12, 7); cancel.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(cancel, 2); loading.Children.Add(cancel);
            SolidColorBrush Tint(string background, double amount)
            {
                var accent = Design.Brush("clay").Color; var surface = Design.Brush(background).Color;
                byte Channel(byte a, byte b) => (byte)Math.Round(a * amount + b * (1 - amount));
                return new(Windows.UI.Color.FromArgb(255, Channel(accent.R, surface.R), Channel(accent.G, surface.G), Channel(accent.B, surface.B)));
            }
            Form.Children.Add(new Border { Child = loading, Margin = new Thickness(0, 16, 0, 16), Padding = new Thickness(16, 14, 16, 14), CornerRadius = new CornerRadius(10), BorderThickness = new Thickness(1), BorderBrush = Tint("border", .22), Background = Tint("bg-elev", .05) });
        }
        candidatesHost = new ContentControl { HorizontalContentAlignment = HorizontalAlignment.Stretch }; Form.Children.Add(candidatesHost); DrawCandidates();
    }
    private void Community()
    {
        Form.Children.Add(Hint(T("store.community_notice")));
        var search = Input("", model.Query, value => { model.Query = value; DrawCatalog(); }, T("store.search"));
        tagChoice = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch, MinWidth = 130, CornerRadius = new CornerRadius(10), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong") }; AutomationProperties.SetName(tagChoice, T("store.tags"));
        tagChoice.Items.Add(new ComboBoxItem { Content = T("store.all_tags"), Tag = "" }); foreach (var tag in model.AllTags) tagChoice.Items.Add(new ComboBoxItem { Content = tag, Tag = tag });
        tagChoice.SelectedItem = tagChoice.Items.Cast<ComboBoxItem>().FirstOrDefault(t => (string)t.Tag == model.Tag) ?? tagChoice.Items[0];
        tagChoice.SelectionChanged += (_, _) => { if (tagChoice.SelectedItem is ComboBoxItem item) { model.Tag = (string)item.Tag; DrawCatalog(); } }; previewControls.Add(tagChoice);
        var toolbar = new Grid { ColumnSpacing = 10, RowSpacing = 10 }; toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto }); toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto }); toolbar.Children.Add(search); Grid.SetColumn(tagChoice, 1); toolbar.Children.Add(tagChoice);
        var refresh = ActionButton(T("store.refresh"), () => Load(true)); Grid.SetColumn(refresh, 2); toolbar.Children.Add(refresh);
        toolbar.SizeChanged += (_, e) => { var narrow = e.NewSize.Width < 650; Grid.SetColumnSpan(search, narrow ? 3 : 1); Grid.SetRow(tagChoice, narrow ? 1 : 0); Grid.SetColumn(tagChoice, narrow ? 0 : 1); Grid.SetColumnSpan(tagChoice, narrow ? 2 : 1); Grid.SetRow(refresh, narrow ? 1 : 0); };
        Form.Children.Add(toolbar); if (!string.IsNullOrEmpty(model.CatalogNotice)) Form.Children.Add(Hint(model.CatalogNotice));
        count = Hint(""); Form.Children.Add(count); catalogHost = new ContentControl { HorizontalContentAlignment = HorizontalAlignment.Stretch }; Form.Children.Add(catalogHost); DrawCatalog();
    }
    private void DrawCatalog()
    {
        if (catalogHost == null || count == null) return;
        previewControls.RemoveAll(control => control.Tag as string == "catalog-action");
        var visible = model.Visible; count.Text = $"{visible.Length} / {model.Catalog.Count} " + T("store.directory_entries");
        if (visible.Length == 0) { catalogHost.Content = Hint(T("store.empty")); return; }
        catalogHost.Content = Tiles(visible.Select(entry =>
        {
            var body = new StackPanel { Spacing = 10 }; body.Children.Add(Text(S(entry, "name"), 15, true)); body.Children.Add(Text(S(entry, "description"))); body.Children.Add(Hint(S(entry, "author") + " · " + S(entry, "license"))); body.Children.Add(Hint(S(entry, "repository") + " @ " + S(entry, "git_ref")));
            var tags = new NativeActionWrap { Gap = 5 }; foreach (var tag in Strings(entry["tags"])) tags.Children.Add(Badge(tag)); body.Children.Add(tags); body.Children.Add(Badge(T("store.community")));
            body.Children.Add(Text(T(model.InstalledNames.Contains(S(entry, "name")) ? "store.name_present" : "store.not_installed")));
            var preview = ActionButton(T("store.preview"), () => PreviewEntry(entry)); preview.Tag = "catalog-action"; preview.HorizontalAlignment = HorizontalAlignment.Left; body.Children.Add(preview);
            var card = Design.Card(body); card.Padding = new Thickness(16); card.CornerRadius = new CornerRadius(10); card.Background = Design.Brush("bg-app"); return (FrameworkElement)card;
        }), 290); Design.ApplyTypography(catalogHost); Refresh();
    }
    private Task PreviewEntry(JsonObject entry) => ObservePreview(model.PreviewEntryAsync(entry));
    private Task PreviewMarketplace(int index) => ObservePreview(model.PreviewMarketplaceAsync(index));
    private Task PreviewUser() => ObservePreview(model.PreviewAsync());
    private async Task ObservePreview(Task<bool> task)
    {
        var version = model.PreviewVersion; Render(); await task;
        // A cancelled read must not rebuild controls after the user resumes editing.
        if (!model.Closed && version == model.PreviewVersion) Render();
    }
    private void DrawCandidates()
    {
        if (candidatesHost == null) return;
        previewControls.RemoveAll(control => control.Tag as string is "candidate-action" or "candidate-search");
        var body = new StackPanel { Spacing = 10 }; candidatesHost.Content = body;
        if (model.Candidates.Count > 0)
        {
            var search = Input("", model.CandidateQuery, value => { model.CandidateQuery = value; DrawCandidateRows(body); }, T("store.search_packages")); search.Tag = "candidate-search"; body.Children.Add(search); body.Children.Add(new StackPanel()); DrawCandidateRows(body);
        }
        else if (model.Previewed) body.Children.Add(Hint(Design.L("未找到可安装的技能包。", "No installable packages found.")));
        Design.ApplyTypography(body); Refresh();
    }
    private void DrawCandidateRows(StackPanel body)
    {
        previewControls.RemoveAll(control => control.Tag as string == "candidate-action");
        var rows = (StackPanel)body.Children[1]; rows.Spacing = 8; rows.Children.Clear();
        foreach (var candidate in model.VisibleCandidates)
        {
            var content = new StackPanel { Spacing = 8 }; var heading = new NativeActionWrap(); heading.Children.Add(Text(S(candidate, "name"), 14, true)); heading.Children.Add(Hint(S(candidate["source"], "package_path"))); content.Children.Add(heading);
            content.Children.Add(Hint(S(candidate, "description"))); content.Children.Add(Text(NativeSkillStoreModel.CandidateStatus(candidate, Design.Localization), 12));
            var line = new Grid { ColumnSpacing = 12 }; line.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); line.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); line.Children.Add(content);
            var chevron = Design.Icon("chevron-right", 16); Grid.SetColumn(chevron, 1); line.Children.Add(chevron);
            var button = ActionButton(S(candidate, "name"), () => { if (model.Select(candidate)) { Render(); BodyScroll.ChangeView(null, 0, null, true); } }); button.Tag = "candidate-action"; button.Content = line; button.HorizontalAlignment = HorizontalAlignment.Stretch; button.HorizontalContentAlignment = HorizontalAlignment.Stretch; button.Padding = new Thickness(14); rows.Children.Add(button);
        }
        if (rows.Children.Count == 0) rows.Children.Add(Hint(Design.L("没有匹配的技能包。", "No matching packages."))); Design.ApplyTypography(rows); Refresh();
    }
    private FrameworkElement Sources(JsonObject source) => NativeSkillSourceView.Create(source, Design);
    private FrameworkElement Definitions(IEnumerable<(string Label, FrameworkElement Value)> entries)
    {
        var grid = new Grid { ColumnSpacing = 12, RowSpacing = 7 }; grid.ColumnDefinitions.Add(new() { Width = new GridLength(130) }); grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        foreach (var entry in entries)
        {
            var row = grid.RowDefinitions.Count; grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); var label = Text(entry.Label, 13, true); Grid.SetRow(label, row); grid.Children.Add(label); Grid.SetColumn(entry.Value, 1); Grid.SetRow(entry.Value, row); grid.Children.Add(entry.Value);
        }
        grid.SizeChanged += (_, e) => grid.ColumnDefinitions[0].Width = new GridLength(e.NewSize.Width < 420 ? 80 : 130); return grid;
    }
    private void Preview(JsonObject candidate)
    {
        Form.Children.Add(Text(S(candidate, "name"), 17, true)); Form.Children.Add(Text(S(candidate, "description")));
        if (candidate["source"] is JsonObject source) Form.Children.Add(Sources(source));
        Form.Children.Add(Badge(model.Marketplace is { } market ? NativeSkillStoreModel.Marketplaces[market].Name : T(model.SelectedEntry != null ? "store.community" : "store.user_source")));
        if (model.SelectedEntry is { } entry)
        {
            if (S(entry, "name") != S(candidate, "name") || S(entry, "description") != S(candidate, "description")) Form.Children.Add(Text(T("store.metadata_drift")));
            Form.Children.Add(Definitions(new[] { ("responsibilities", "store.responsibilities"), ("when_to_use", "store.trigger"), ("inputs", "store.inputs"), ("outputs", "store.outputs"), ("out_of_scope", "store.excludes"), ("required_dependencies", "store.required"), ("optional_dependencies", "store.optional"), ("operation_boundary", "store.boundaries"), ("supported_wisp", "store.declared"), ("verified_wisp", "store.verified"), ("known_limits", "store.limits") }
                .Select(field => (T(field.Item2), (FrameworkElement)Text(NativeSkillStoreModel.MetadataValue(entry, field.Item1, Design.Localization), 13)))
                .Append((T("store.author"), Text(S(entry, "author") + " · " + S(entry, "license"), 13)))));
            Form.Children.Add(Link(T("store.feedback"), S(entry, "feedback_url"), toolbar: true));
        }
        if (Strings(candidate["format_errors"]).Length == 0) Form.Children.Add(Status(T("store.format_pass"), true));
        foreach (var (key, title) in new[] { ("format_errors", "store.format_error"), ("resource_errors", "store.resource_error"), ("warnings", "") })
            foreach (var value in Strings(candidate[key])) Form.Children.Add(title.Length == 0 ? Hint(value) : Status(T(title) + ": " + value, false));
        if (candidate["conflict"] != null) Form.Children.Add(Status(T("store.conflict") + ": " + S(candidate, "conflict"), false));
        if (candidate["installed_source"] is JsonObject installed)
        {
            if (candidate["conflict"] == null) Form.Children.Add(Text(T("store.already_installed"))); Form.Children.Add(Text(T("store.installed_source"), 15, true)); Form.Children.Add(Sources(installed));
        }
        Form.Children.Add(Hint(T("store.dependencies_pending"))); Form.Children.Add(Hint(T("store.unverified")));
        Form.Children.Add(Text("SKILL.md", 14, true));
        var markdown = new TextBox { AcceptsReturn = true, Text = S(candidate, "markdown"), IsReadOnly = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 420, CornerRadius = new CornerRadius(8), Padding = new Thickness(14) }; AutomationProperties.SetName(markdown, "SKILL.md"); Design.BindTypography(markdown, code: true); menus.Register(markdown); Form.Children.Add(markdown);
        var review = Button(T("store.review_install"), () => { if (model.BeginConfirmation()) ShowConfirmation(); return Task.CompletedTask; }); StoreButton(review); review.IsEnabled = NativeSkillStoreModel.Installable(candidate); review.HorizontalAlignment = HorizontalAlignment.Left; Form.Children.Add(review);
    }
    private void ShowConfirmation()
    {
        if (model.ConfirmationSource is not { } source || model.Selected == null) return;
        var overlay = new Grid(); NativeDialogVisuals.Backdrop(overlay, Design);
        var body = new StackPanel { Spacing = 14, Padding = new Thickness(22) };
        // The WebView confirmation inherits the browser's h3/h4/dl/p margins.
        var title = Text(T("store.confirm_title"), 17, true); title.Margin = new Thickness(0, 16, 0, 16); body.Children.Add(title);
        var name = Text(S(model.Selected, "name"), 14, true); name.Margin = new Thickness(0, 18, 0, 18); body.Children.Add(name);
        var sources = Sources(source); sources.Margin = new Thickness(0, 13, 0, 13); body.Children.Add(sources);
        foreach (var paragraph in new[] { Code("~/.wisp/skills/" + S(model.Selected, "name")), Text(T("store.scope")), Text(T("store.no_execution")) })
        { paragraph.Margin = new Thickness(0, 14, 0, 14); body.Children.Add(paragraph); }
        dialogError = Text(""); body.Children.Add(dialogError); var actions = new NativeActionWrap { Gap = 0 };
        var cancel = Button(T("store.cancel"), () => { Back(); return Task.CompletedTask; }); var install = Button(T("store.confirm_install"), async () => { if (await model.InstallAsync()) { CloseConfirmation(); Render(); } else Refresh(); }); StoreButton(cancel); StoreButton(install);
        confirmationControls.Clear(); confirmationControls.Add(cancel); confirmationControls.Add(install); actions.Children.Add(cancel); actions.Children.Add(install); body.Children.Add(actions);
        var card = new Border { Background = Design.Brush("bg-elev"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(16), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle,
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24),
            Child = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch } };
        overlay.Children.Add(card); void Resize() { card.Width = Math.Min(480, Math.Max(0, overlay.ActualWidth - 40)); card.MaxHeight = Math.Max(0, overlay.ActualHeight - 60); body.Width = Math.Max(0, card.Width - 2); }
        overlay.SizeChanged += (_, _) => Resize(); overlay.Loaded += (_, _) => { Resize(); cancel.Focus(FocusState.Programmatic); };
        AutomationProperties.SetName(overlay, T("store.confirm_title")); confirmation = overlay; ModalChanged?.Invoke(overlay); Design.ApplyTypography(body); Refresh();
    }
    private void CloseConfirmation() { confirmation = null; dialogError = null; confirmationControls.Clear(); ModalChanged?.Invoke(null); }
    public override void Dispose() { menus.Dispose(); CloseConfirmation(); model.Changed -= Refresh; base.Dispose(); }
}
