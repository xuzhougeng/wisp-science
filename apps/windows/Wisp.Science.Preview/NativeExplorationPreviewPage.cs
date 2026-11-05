using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeExplorationPreviewPage : WorkspaceSheet
{
    private readonly WorkspaceExplorationPreview model;
    private readonly Action<NativeExplorationActionResult> completed;
    private readonly StackPanel top = new() { Spacing = 12 }, rows = new() { Spacing = 4 };
    private readonly NativeActionWrap tabs = new() { Gap = 3, RowGap = 3 };
    private readonly Dictionary<ExplorationReviewTab, Button> tabButtons = [];
    private readonly ScrollViewer diffScroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, MinHeight = 120 };
    private readonly Border diff;
    private readonly TextBlock error, loading;
    private readonly Button discard, promote, refresh;
    private Button? openFolders, finishManual;
    private readonly ContentControl surface;
    private NativeExplorationPreviewData? rendered;
    private ExplorationReviewTab tab;
    private NativeExplorationConfirmPage? confirmation;
    private Control? confirmationFocus;
    private bool disposed;
    public NativeExplorationPreviewPage(WorkspaceExplorationPreview model, WispDesign design, Action<NativeExplorationActionResult> completed, Action close)
        : base(design, design.T("exploration.diff_title"), close, modal: true, modalWidth: 920)
    {
        this.model = model; this.completed = completed; SetModalHeight(720);
        var overlay = (Grid)Content; var card = overlay.Children.Last(); overlay.Children.Remove(card);
        surface = new ContentControl { Content = card, HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Stretch };
        overlay.Children.Add(surface);
        Body.Spacing = 10;
        error = Text("", 12, "err"); Notices.Children.Add(new ScrollViewer { Content = error, MaxHeight = 130, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled });
        loading = Text(design.T("loading"), 12); loading.Margin = new(0, 40, 0, 40); loading.HorizontalAlignment = HorizontalAlignment.Center; Body.Children.Add(loading);
        refresh = Action("session.artifacts.refresh", async () => await model.LoadAsync()); HeaderActions.Children.Add(refresh);
        Body.Children.Add(top);
        foreach (var value in Enum.GetValues<ExplorationReviewTab>())
        {
            var button = Action(WorkspaceExplorationPreview.TabKey(value), () => { tab = value; RenderRows(); });
            design.QuietButton(button); button.Padding = new(9, 6, 9, 6); button.CornerRadius = new(5); button.BorderThickness = new(1);
            AutomationProperties.SetAutomationId(button, "exploration-tab-" + value); tabButtons[value] = button; tabs.Children.Add(button);
        }
        Body.Children.Add(tabs);
        diffScroll.Content = rows; diff = new Border { Child = diffScroll, Padding = new(8), Background = design.Brush("bg-sunken"), BorderBrush = design.Brush("border"), BorderThickness = new(1), CornerRadius = new(6) };
        Body.Children.Add(diff);
        discard = Action("exploration.discard", () => Confirm("discard", discard!), danger: true);
        promote = Action("exploration.promote", () => Confirm("promote", promote!), primary: true);
        var actions = new Grid { ColumnSpacing = 10 }; actions.ColumnDefinitions.Add(new()); actions.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        discard.HorizontalAlignment = HorizontalAlignment.Left; actions.Children.Add(discard); Grid.SetColumn(promote, 1); actions.Children.Add(promote);
        Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        BodyScroll.SizeChanged += (_, _) => Layout(); top.SizeChanged += (_, _) => Layout(); tabs.SizeChanged += (_, _) => Layout();
        model.Changed += Refresh; Refresh();
    }
    private TextBlock Text(string value, double size = 12, string color = "text-muted", bool code = false)
    {
        var label = Design.Text(value, size); if (code) { label.FontFamily = Design.Font(true); label.FontSize = Design.FontSize(size, true); }
        label.Foreground = Design.Brush(color); Design.BindLineHeight(label, size, 1.45); return label;
    }
    private Button Action(string key, Action action, bool primary = false, bool danger = false)
    {
        var label = Text(Design.T(key), 13, primary ? "on-clay" : danger ? "err" : "text");
        var button = new Button { Content = label }; Design.ActionButton(button, primary);
        if (primary)
        {
            button.Resources["ButtonBackgroundDisabled"] = button.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay");
            button.Resources["ButtonForegroundDisabled"] = Design.Brush("on-clay");
        }
        button.Click += (_, _) => action(); return button;
    }
    private void Layout()
    {
        if (BodyScroll.ActualHeight <= 0) return;
        diff.Height = Math.Max(140, BodyScroll.ActualHeight - top.ActualHeight - tabs.ActualHeight - 32);
    }
    private void Refresh()
    {
        if (disposed) return;
        error.Text = (model.MutationUnconfirmed ? Design.L("结果未能确认；不会自动重试。关闭后请检查主线和探索列表。", "The result could not be confirmed and will not be retried. Close and check mainline and the exploration list.") + "\n" : "") + model.Error;
        Notices.Visibility = model.ShowFailure ? Visibility.Visible : Visibility.Collapsed;
        loading.Visibility = model.Busy && model.Preview == null ? Visibility.Visible : Visibility.Collapsed;
        top.Visibility = tabs.Visibility = diff.Visibility = model.Preview == null ? Visibility.Collapsed : Visibility.Visible;
        if (model.Preview?.Preview is { } preview && !ReferenceEquals(rendered, preview))
        {
            rendered = preview; SetModalHeading(preview.Exploration.Name, Design.T("exploration.status_" + preview.Exploration.Status)); top.Children.Clear(); openFolders = finishManual = null;
            if (!preview.Eligibility.Eligible)
            {
                var copy = new StackPanel { Spacing = 3 }; var title = Text(Design.T("exploration.cannot_promote"), 12, "warn"); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; copy.Children.Add(title);
                foreach (var reason in preview.Eligibility.Reasons) copy.Children.Add(Text(WorkspaceExplorationPreview.BlockerText(reason, Design.Localization)));
                top.Children.Add(Card(copy, "warn"));
            }
            if (preview.Eligibility.ManualResolutionAvailable)
            {
                var copy = new StackPanel { Spacing = 5 }; var title = Text(Design.T("exploration.manual_title"), 12, "text"); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; copy.Children.Add(title);
                copy.Children.Add(Text(Design.T("exploration.manual_body"))); copy.Children.Add(Text(Design.T("exploration.manual_warning"), 12, "warn"));
                var actions = new NativeActionWrap { Gap = 8, RowGap = 6, RightAligned = true };
                openFolders = Action("exploration.manual_open_folders", async () => await model.OpenFoldersAsync()); actions.Children.Add(openFolders);
                finishManual = Action("exploration.manual_finish", () => Confirm("manual_finish", finishManual!), primary: true); actions.Children.Add(finishManual);
                copy.Children.Add(actions); top.Children.Add(Card(copy));
            }
            RenderRows();
        }
        SetDismissEnabled(model.CanDismiss); refresh.IsEnabled = model.CanRead;
        refresh.Visibility = model.Error != null ? Visibility.Visible : Visibility.Collapsed;
        discard.Visibility = model.Preview?.Preview.Exploration.Status is "active" or "failed" ? Visibility.Visible : Visibility.Collapsed;
        discard.IsEnabled = model.CanDiscard; promote.IsEnabled = model.CanPromote; promote.Opacity = model.CanPromote ? 1 : .5;
        if (openFolders != null) openFolders.IsEnabled = model.CanResolveManually;
        if (finishManual != null) finishManual.IsEnabled = model.CanResolveManually;
        foreach (var button in tabButtons.Values) button.IsEnabled = !model.Mutating;
        Layout();
    }
    private Border Card(UIElement child, string border = "border") => new() { Child = child, Padding = new(11, 9, 11, 9), CornerRadius = new(5), BorderThickness = new(1), BorderBrush = Design.Brush(border), Background = Design.Brush("bg-elev") };
    private void RenderRows()
    {
        if (rendered == null) return;
        foreach (var (value, button) in tabButtons)
        {
            ((TextBlock)button.Content).Text = Design.T(WorkspaceExplorationPreview.TabKey(value)) + " " + WorkspaceExplorationPreview.Count(rendered.Diff, value);
            button.Background = Design.Brush(value == tab ? "clay-soft" : "bg-elev"); button.BorderBrush = value == tab ? Design.Brush("clay") : new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            button.Resources["ButtonBackgroundPointerOver"] = Design.Brush(value == tab ? "clay-soft" : "surface-hover");
            button.Resources["ButtonBorderBrushPointerOver"] = button.BorderBrush;
            AutomationProperties.SetItemStatus(button, value == tab ? "selected" : "");
        }
        rows.Children.Clear();
        foreach (var section in WorkspaceExplorationPreview.Sections(rendered, tab))
        {
            if (section.HeadingKey != null)
            {
                var title = Text(Design.T(section.HeadingKey), 10, "text-faint"); title.CharacterSpacing = 40; title.Margin = new(4, rows.Children.Count == 0 ? 2 : 12, 4, 5); rows.Children.Add(title);
            }
            if (section.Rows.Length == 0) { var empty = Text(Design.T("exploration.diff_empty")); empty.Margin = new(8, 45, 8, 45); empty.HorizontalAlignment = HorizontalAlignment.Center; rows.Children.Add(empty); }
            foreach (var row in section.Rows)
            {
                var copy = new StackPanel { Spacing = 2 }; var title = Text(row.Title, 12, row.Warning ? "warn" : "text", code: row.Code && row.DeltaKind != null);
                title.IsTextSelectionEnabled = true; if (row.DeltaKind == null) title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; copy.Children.Add(title);
                if (row.Detail.Length > 0) { var detail = Text(row.Detail, 11, "text-faint", row.Code); detail.IsTextSelectionEnabled = true; copy.Children.Add(detail); }
                UIElement content = copy;
                if (row.DeltaKind is { } kind)
                {
                    var grid = new Grid { ColumnSpacing = 8 }; grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); grid.ColumnDefinitions.Add(new());
                    var label = Text(kind.ToUpperInvariant(), 9.5, kind switch { "added" => "ok", "modified" => "warn", "deleted" => "err", _ => "text-faint" }, code: true); label.MinWidth = 52; grid.Children.Add(label); Grid.SetColumn(copy, 1); grid.Children.Add(copy); content = grid;
                }
                rows.Children.Add(new Border { Child = content, Padding = new(9, 7, 9, 7), CornerRadius = new(4), Background = Design.Brush("bg-elev") });
            }
        }
        diffScroll.ChangeView(null, 0, null, true); Layout();
    }
    private void Confirm(string action, Control source)
    {
        if (confirmation != null || !model.CanAct) return;
        confirmationFocus = source; surface.IsEnabled = false;
        confirmation = new NativeExplorationConfirmPage(Design, action, async () =>
        {
            CloseConfirmation();
            if (await model.CompleteAsync(action) && !disposed && model.Result is { } result) completed(result);
        }, CloseConfirmation);
        confirmation.Translation = new System.Numerics.Vector3(0, 0, 48); ((Grid)Content).Children.Add(confirmation);
    }
    private void CloseConfirmation()
    {
        if (confirmation == null) return;
        ((Grid)Content).Children.Remove(confirmation); confirmation.Dispose(); confirmation = null; surface.IsEnabled = true;
        confirmationFocus?.Focus(FocusState.Programmatic); confirmationFocus = null;
    }
    public override void HandleEscape() { if (confirmation != null) CloseConfirmation(); else if (model.CanDismiss) base.HandleEscape(); }
    public override void Dispose() { disposed = true; CloseConfirmation(); model.Changed -= Refresh; model.Dispose(); base.Dispose(); }
}

internal sealed class NativeExplorationConfirmPage : WorkspaceSheet
{
    public NativeExplorationConfirmPage(WispDesign design, string action, Action confirm, Action close)
        : base(design, design.T(action == "promote" ? "exploration.promote_confirm_title" : action == "manual_finish" ? "exploration.manual_confirm_title" : "exploration.discard_confirm_title"), close, modal: true, modalWidth: 540)
    {
        HideDismissButton();
        var hint = design.Text(design.T(action == "promote" ? "exploration.promote_confirm_body" : action == "manual_finish" ? "exploration.manual_confirm_body" : "exploration.discard_confirm_body"), 13);
        design.BindLineHeight(hint, 13, 1.55); hint.Foreground = design.Brush("text-muted"); Body.Children.Add(hint);
        var actions = new NativeActionWrap { Gap = 10, RowGap = 8, RightAligned = true };
        var cancel = new Button { Content = design.T("settings.cancel") }; design.BindTypography(cancel, size: 13); design.ActionButton(cancel); cancel.Click += (_, _) => close(); actions.Children.Add(cancel);
        var submit = new Button { Content = design.T(action == "promote" ? "exploration.promote" : action == "manual_finish" ? "exploration.manual_finish" : "exploration.discard") };
        design.BindTypography(submit, size: 13); design.ActionButton(submit, true);
        if (action != "promote") { submit.Background = submit.BorderBrush = design.Brush("err"); }
        submit.Click += (_, _) => confirm(); actions.Children.Add(submit); Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        Loaded += (_, _) => cancel.Focus(FocusState.Programmatic);
    }
}
