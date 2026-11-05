using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativePublicationPage
{
    private readonly StackPanel runBinding = new();
    private NativePublicationWorkspace? bindingWorkspace;
    private bool bringBindingIntoView = true;
    private string purpose = "", selection = "selected", visibilityValue = "private";
    private string? item, claim;
    private Button? bind;

    private string T(string key) => Design.T("publication." + key);
    private string ValueLabel(string group, string? value)
    {
        if (string.IsNullOrEmpty(value)) return "";
        var key = "publication." + group + "." + value;
        var label = Design.T(key);
        return label == key ? value : label;
    }
    private TextBlock Serif(string key, double size)
    {
        var text = Design.Text(T(key), size);
        text.FontFamily = new("ms-appx:///Assets/Fonts/wisp-serif.ttf#Wisp Serif, Noto Serif SC, SimSun, Georgia, Times New Roman");
        return text;
    }

    private void SetupPublicationHeader(Action close)
    {
        var header = new Grid { ColumnSpacing = 12, Margin = new(0, 0, 0, 32) };
        header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var copy = new StackPanel { Spacing = 7 }; copy.Children.Add(Serif("title", 28)); copy.Children.Add(Mute(T("subtitle"))); header.Children.Add(copy);
        var controls = new NativeActionWrap();
        var refresh = Design.ToolButton(T("refresh"), "sync"); refresh.Click += async (_, _) => await ReadPublication(); controls.Children.Add(refresh);
        var back = Design.ToolButton(T("close"), "arrow-left"); back.Click += (_, _) => close(); controls.Children.Add(back);
        controls.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(controls, 1); header.Children.Add(controls);
        Form.MaxWidth = Body.MaxWidth = double.PositiveInfinity;
        void Resize() {
            var width = ActualWidth;
            var gutter = width <= 740 ? 16 : Math.Clamp(width * .03, 24, 64);
            Body.Width = Math.Max(1, width); Body.Padding = new(gutter, width <= 740 ? 20 : 30, gutter, 40);
        }
        SizeChanged += (_, _) => Resize(); Loaded += (_, _) => Resize();
        UseScrollingHeader(header);
    }

    private void SetupCreation()
    {
        var intro = new StackPanel { Spacing = 14, MaxWidth = 540, HorizontalAlignment = HorizontalAlignment.Left };
        intro.Children.Add(Serif("create", 19));
        var hint = Mute(T("create_hint")); hint.LineHeight = Design.FontSize(21); intro.Children.Add(hint);
        var steps = new NativeActionWrap { Gap = 12 };
        foreach (var (key, index) in new[] { ("choose_source", 1), ("explain_use", 2), ("tab_readiness", 3) })
            steps.Children.Add(Mute(index + ". " + T(key)));
        intro.Children.Add(steps); createLayout.Children.Add(intro);
        var card = Design.Card(creation); card.MaxWidth = 760; card.CornerRadius = new(10);
        StylePrimary(createPublication);
        creation.Spacing = 14; createLayout.Children.Add(card);
        createLayout.RowDefinitions.Add(new() { Height = GridLength.Auto }); createLayout.RowDefinitions.Add(new() { Height = GridLength.Auto });
        createLayout.ColumnDefinitions.Add(new()); createLayout.ColumnDefinitions.Add(new());
        void Arrange() {
            var wide = ActualWidth >= 1100;
            createLayout.ColumnDefinitions[0].Width = wide ? new(320) : new(1, GridUnitType.Star);
            createLayout.ColumnDefinitions[1].Width = wide ? new(1, GridUnitType.Star) : new(0);
            createLayout.ColumnSpacing = wide ? 48 : 0;
            createLayout.RowSpacing = wide ? 0 : 44;
            Grid.SetColumn(card, wide ? 1 : 0); Grid.SetRow(card, wide ? 0 : 1);
            card.Width = Math.Max(1, Math.Min(760, createLayout.ActualWidth - (wide ? 368 : 0)));
            card.HorizontalAlignment = HorizontalAlignment.Left; card.Padding = new(ActualWidth <= 740 ? 16 : 24);
        }
        createLayout.Loaded += (_, _) => Arrange(); createLayout.SizeChanged += (_, _) => Arrange();
    }
    private void StylePrimary(Button button)
    {
        if (button.Content is TextBlock caption) caption.Foreground = button.Foreground;
        button.Resources["ButtonBackgroundDisabled"] = button.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay");
        button.Resources["ButtonForegroundDisabled"] = button.Foreground;
    }
    private void UpdateCreate()
    {
        if (createPublication == null) return;
        createPublication.IsEnabled = model.CanCreate; createPublication.Opacity = model.CanCreate ? 1 : .5;
    }

    private async Task ReadPublication()
    {
        if (!await model.LoadAsync() || model.PendingRun == null || model.Workspace?.Revision?.State == "draft") return;
        if (model.Workspace?.Revisions?.FirstOrDefault(r => r.State == "draft") is { } draft)
            await model.SelectWorkspaceAsync(model.PublicationId, draft.Id);
    }

    private void RenderRunBinding()
    {
        var source = model.PendingRun;
        runBinding.Visibility = source == null ? Visibility.Collapsed : Visibility.Visible;
        if (source == null) { bind = null; runBinding.Children.Clear(); bindingWorkspace = null; return; }
        tab = "sources";
        var ws = model.Workspace;
        if (ws == null || ReferenceEquals(ws, bindingWorkspace)) { UpdateBind(); return; }
        if (ws.Revision?.Id != bindingWorkspace?.Revision?.Id) item = claim = null;
        bindingWorkspace = ws; bind = null; runBinding.Children.Clear();
        var card = new StackPanel { Spacing = 15 }; runBinding.Children.Add(Design.Card(card));
        var head = new Grid(); head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var labels = new StackPanel { Spacing = 3 }; labels.Children.Add(Design.Text(T("bind_title"), 18)); labels.Children.Add(Mute(source.Title)); head.Children.Add(labels);
        var x = Design.ToolButton(T("close"), "close"); x.Click += (_, _) => CancelBinding();
        Grid.SetColumn(x, 1); head.Children.Add(x); card.Children.Add(head);
        if (ws.Revisions?.Any(r => r.State == "draft") != true)
            card.Children.Add(Mute(T("bind_requires_draft")));
        else
        {
            var fields = new Grid { ColumnSpacing = 14, RowSpacing = 12 };
            fields.ColumnDefinitions.Add(new()); fields.ColumnDefinitions.Add(new());
            for (int n = 0; n < 6; n++) fields.RowDefinitions.Add(new() { Height = GridLength.Auto });
            var groups = Enumerable.Range(0, 6).Select(_ => new StackPanel()).ToArray();
            foreach (var group in groups) fields.Children.Add(group);
            void Arrange() {
                var narrow = fields.ActualWidth < 490;
                var rows = narrow ? new[] { 0, 1, 2, 3, 4, 5 } : [0, 0, 1, 2, 2, 3];
                for (var i = 0; i < groups.Length; i++) {
                    Grid.SetRow(groups[i], rows[i]); Grid.SetColumn(groups[i], !narrow && i is 1 or 4 ? 1 : 0);
                    Grid.SetColumnSpan(groups[i], narrow || i == 2 ? 2 : 1);
                }
            }
            fields.SizeChanged += (_, _) => Arrange(); Arrange(); card.Children.Add(fields);
            var revision = ws.Revision?.Id;
            var revisionChoice = Choice(groups[0], T("revision"), ws.Revisions!.Select(r => (r.Id, r.Label + " · " + r.State)), revision, async id => {
                if (id == model.RevisionId || model.Busy) return;
                if (!await model.SelectWorkspaceAsync(model.PublicationId, id) && !model.Closed) { bindingWorkspace = null; Render(); }
            });
            foreach (var option in revisionChoice.Items.Cast<ComboBoxItem>()) option.IsEnabled = ws.Revisions!.Any(r => r.Id == (string)option.Tag && r.State == "draft");
            Choice(groups[1], T("item_target"), new[] { ("", T("unassigned")) }.Concat(ws.Items.Select(i => (i.Id, i.Title))), item, id => item = id == "" ? null : id);
            Input(groups[2], T("purpose"), purpose, value => { purpose = value; UpdateBind(); }, true);
            Choice(groups[3], T("supported_claim"), new[] { ("", T("no_claim")) }.Concat(ws.Items.Where(i => i.Kind == "claim").Select(i => (i.Id, i.Title))), claim, id => claim = id == "" ? null : id);
            Choice(groups[4], T("selection"), new[] { "candidate", "selected", "rejected" }.Select(v => (v, T("selection." + v))), selection, id => selection = id);
            Choice(groups[5], T("visibility"), new[] { "public", "restricted", "private" }.Select(v => (v, T("visibility." + v))), visibilityValue, id => visibilityValue = id);
            var footer = new NativeActionWrap { HorizontalAlignment = HorizontalAlignment.Right };
            footer.Children.Add(Button(T("cancel"), () => { CancelBinding(); return Task.CompletedTask; }));
            bind = Button(T("bind_action"), async () => {
                if (revision != null && await model.BindPendingRunAsync(revision, item, purpose, claim, selection, visibilityValue)) { tab = "evidence"; ShowTab(); }
            }, true); StylePrimary(bind); footer.Children.Add(bind); card.Children.Add(footer); UpdateBind();
        }
        if (bringBindingIntoView) {
            bringBindingIntoView = false;
            void Reveal() { if (!model.Closed && model.PendingRun != null) runBinding.StartBringIntoView(new() { AnimationDesired = false, VerticalAlignmentRatio = 0 }); }
            if (runBinding.IsLoaded) DispatcherQueue.TryEnqueue(Reveal);
            else runBinding.Loaded += (_, _) => Reveal();
        }
    }
    private void UpdateBind()
    {
        if (bind == null) return;
        bind.IsEnabled = model.Editable && !string.IsNullOrWhiteSpace(purpose);
        bind.Opacity = bind.IsEnabled ? 1 : .5;
    }
    private void CancelBinding()
    {
        model.DismissRunBinding();
        if (tab == "sources" && model.Workspace?.Publication != null) _ = LoadSources();
    }
}
