using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// Windows uses the same release-page update flow as the Windows WebView.
internal sealed class NativeUpdateCheckPage : UserControl, IWorkspaceSheet
{
    private readonly NativeUpdateCheckModel model;
    private readonly WispDesign design;
    private readonly Action close;
    private readonly Action<string> openRelease;
    private readonly Grid overlay = new();
    private Border? card;
    private ScrollViewer? notes;
    private Button? safeAction;
    private FrameworkElement? titleBlock, hintBlock, actionRow;
    private bool disposed, started;
    public bool IsModal => true;
    public NativeUpdateCheckPage(NativeUpdateCheckModel model, WispDesign design, Action close, Action<string> openRelease)
    {
        this.model = model; this.design = design; this.close = close; this.openRelease = openRelease;
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        design.BindTypography(this); Content = overlay; NativeDialogVisuals.Backdrop(overlay, design);
        SizeChanged += (_, _) => Resize();
        Loaded += async (_, _) => { if (!started) { started = true; await model.LoadAsync(); } };
        model.Changed += Render; Render();
    }
    private Button Button(string key, Action action, bool primary = false, bool muted = false)
    {
        var label = design.Text(design.T(key), 13.5);
        label.Foreground = design.Brush(primary ? "on-clay" : muted ? "text-faint" : "text");
        if (primary) label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var button = new Button { Content = label, Padding = new Thickness(16, 9, 16, 9), CornerRadius = new CornerRadius(10),
            Background = design.Brush(primary ? "clay" : "bg-input"), BorderBrush = design.Brush(primary ? "clay" : "border-strong"),
            BorderThickness = new Thickness(1), IsEnabled = !model.Busy };
        button.Resources["ButtonBackgroundPointerOver"] = design.Brush(primary ? "clay-strong" : "surface-hover");
        button.Resources["ButtonBackgroundPressed"] = button.Resources["ButtonBackgroundPointerOver"];
        button.Resources["ButtonBorderBrushPointerOver"] = button.BorderBrush;
        button.Resources["ButtonBorderBrushPressed"] = button.BorderBrush;
        AutomationProperties.SetName(button, design.T(key)); AutomationProperties.SetAutomationId(button, key);
        button.Click += (_, _) => action(); return button;
    }
    private void Render()
    {
        if (disposed) return;
        if (card != null) overlay.Children.Remove(card);
        notes = null; safeAction = null;
        var body = new StackPanel { Padding = new Thickness(22), Spacing = 14, TabFocusNavigation = KeyboardNavigationMode.Cycle };
        var title = design.Text(design.T(model.TitleKey), 17); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        titleBlock = title;
        body.Children.Add(title);
        var hint = design.Text(model.Error ?? design.Localization.Format(model.BodyKey, ("version", model.Version)), 13);
        hint.Foreground = design.Brush("text-muted"); hint.LineHeight = design.FontSize(13 * 1.55);
        hint.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        hintBlock = hint;
        if (model.Error != null) AutomationProperties.SetLiveSetting(hint, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive);
        body.Children.Add(hint);
        if (model.Error == null && model.Available && !string.IsNullOrWhiteSpace(model.Notes))
        {
            var markdown = TranscriptView.RenderUpdateNotes(model.Notes, design);
            notes = new ScrollViewer { Content = markdown,
                HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled,
                HorizontalContentAlignment = HorizontalAlignment.Stretch };
            var frame = new Border { Child = notes, Padding = new Thickness(12, 10, 12, 10), Margin = new Thickness(0, 4, 0, 2),
                BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(6), Background = design.Brush("bg-elev") };
            body.Children.Add(frame);
        }
        var actions = new NativeActionWrap { RightAligned = true, FirstItemLeading = model.Available && model.Error == null,
            Gap = 10, Margin = new Thickness(0, 4, 0, 0) };
        actionRow = actions;
        if (model.Error != null || model.Result != null)
        {
            if (model.Error == null && model.Available)
            {
                actions.Children.Add(Button("update_modal.never", async () => { if (await model.DisableRemindersAsync() && !disposed) close(); }, muted: true));
                safeAction = Button("update_modal.later", HandleEscape); actions.Children.Add(safeAction);
            }
            else { safeAction = Button("update_modal.ok", HandleEscape, primary: model.Error == null); actions.Children.Add(safeAction); }
            if (model.Available || model.Error != null)
                actions.Children.Add(Button("update_modal.open_releases", () =>
                {
                    try { openRelease(model.ReleaseUrl); close(); }
                    catch (Exception error) { model.Fail(error.Message); }
                }, primary: true));
            body.Children.Add(actions);
        }
        // The changelog scrolls independently. Very short viewports can also
        // scroll the card to keep the actions reachable.
        card = design.Card(new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            HorizontalScrollMode = ScrollMode.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch });
        card.Padding = new Thickness(0); card.CornerRadius = new CornerRadius(16); card.BorderBrush = design.Brush("border-strong");
        card.HorizontalAlignment = HorizontalAlignment.Center; card.VerticalAlignment = VerticalAlignment.Center;
        card.Shadow = new ThemeShadow(); card.Translation = new System.Numerics.Vector3(0, 0, 24);
        AutomationProperties.SetName(card, title.Text); AutomationProperties.SetAutomationId(card, "update-check-modal");
        var mounted = card;
        card.Loaded += (_, _) => DispatcherQueue.TryEnqueue(() =>
        { if (!disposed && ReferenceEquals(card, mounted)) safeAction?.Focus(FocusState.Programmatic); });
        body.SizeChanged += (_, _) => { if (ReferenceEquals(card, mounted)) Resize(); };
        overlay.Children.Add(card); Resize();
    }
    private void Resize()
    {
        if (card == null) return;
        var layout = ProjectDialogLayout.ForViewport(ActualWidth, ActualHeight, 480);
        card.Width = layout.Width; card.MaxHeight = layout.MaxHeight;
        if (notes != null && titleBlock != null && hintBlock != null && actionRow != null)
        {
            var constraint = new Windows.Foundation.Size(Math.Max(0, layout.Width - 46), double.PositiveInfinity);
            titleBlock.Measure(constraint); hintBlock.Measure(constraint); actionRow.Measure(constraint);
            // Include card padding, three content gaps and the notes frame.
            // Wrapped action rows and larger fonts must reduce the changelog,
            // never push the primary action below the visible card.
            var occupied = 44 + 42 + 28 + titleBlock.DesiredSize.Height + hintBlock.DesiredSize.Height + actionRow.DesiredSize.Height;
            notes.MaxHeight = Math.Max(1, Math.Min(ActualHeight * .46, layout.MaxHeight - occupied));
        }
    }
    public void HandleEscape() { if (!disposed && !(model.Busy && model.DismissAttempted)) close(); }
    public void Dispose() { if (disposed) return; disposed = true; model.Changed -= Render; model.Dispose(); }
}
