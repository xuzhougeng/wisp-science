using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Func<bool>? modelSwitchWarningDisabled;
    private readonly Action? disableModelSwitchWarning;
    private Popup? modelSwitchDialog;
    private XamlRoot? modelSwitchRoot;
    private Windows.Foundation.TypedEventHandler<XamlRoot, XamlRootChangedEventArgs>? modelSwitchResize;

    private async Task RequestHttpModelSwitchAsync(ComposerModelMenuChoice choice, (string? Project, string? Session, bool History) owner)
    {
        if (modelSwitchDialog != null || XamlRoot == null) return;
        if (model.VisibleItems.Length == 0 || modelSwitchWarningDisabled?.Invoke() == true)
        {
            await CommitHttpModelSwitchAsync(choice, owner, false);
            return;
        }
        var overlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
        NativeDialogVisuals.Backdrop(overlay, design);
        var body = new Grid { Padding = new(22), RowSpacing = 14 };
        body.RowDefinitions.Add(new() { Height = GridLength.Auto });
        body.RowDefinitions.Add(new() { Height = new(1, GridUnitType.Star) });
        body.RowDefinitions.Add(new() { Height = GridLength.Auto }); body.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var heading = design.Text(design.T("models.switch_confirm_title"), 17);
        heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; body.Children.Add(heading);
        var hint = design.Text(design.Localization.Format(choice.SupportsVision ? "models.switch_confirm_hint" : "models.switch_confirm_image_hint", ("model", choice.Label)), 13);
        hint.Foreground = design.Brush("text-muted"); hint.LineHeight = design.FontSize(13) * 1.55;
        var scrollHint = new ScrollViewer { Content = hint, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Grid.SetRow(scrollHint, 1); body.Children.Add(scrollHint);
        var rememberLabel = design.Text(design.T("models.switch_dont_ask"), 13); rememberLabel.Margin = new(8, 0, 0, 0);
        var remember = new CheckBox { Content = rememberLabel, MinHeight = 0,
            Padding = new(0), Foreground = design.Brush("text-muted") };
        remember.Resources["CheckBoxCheckBackgroundFillColorChecked"] = design.Brush("clay-strong");
        AutomationProperties.SetName(remember, design.T("models.switch_dont_ask"));
        Grid.SetRow(remember, 2); body.Children.Add(remember);
        var footer = new NativeActionWrap { Gap = 10, RowGap = 8, RightAligned = true, Margin = new(0, 4, 0, 0) };
        var cancel = new Button { Content = design.T("models.switch_no"), MinWidth = 72 };
        var confirm = new Button { Content = design.T(choice.SupportsVision ? "models.switch_yes" : "models.switch_ignore_images"), MinWidth = 72 };
        design.ActionButton(cancel); design.ActionButton(confirm, true);
        design.BindTypography(cancel, size: 13.5); design.BindTypography(confirm, size: 13.5);
        cancel.Click += (_, _) => CloseModelSwitchConfirmation();
        confirm.Click += async (_, _) =>
        {
            if (modelSwitchDialog == null) return;
            var skipFuture = remember.IsChecked == true;
            CloseModelSwitchConfirmation();
            await CommitHttpModelSwitchAsync(choice, owner, skipFuture);
        };
        footer.Children.Add(cancel); footer.Children.Add(confirm); Grid.SetRow(footer, 3); body.Children.Add(footer);
        var card = new Border { Child = body, CornerRadius = new(16), Background = design.Brush("bg-elev"),
            BorderBrush = design.Brush("border-strong"), BorderThickness = new(1),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
            TabFocusNavigation = KeyboardNavigationMode.Cycle, Shadow = new ThemeShadow(), Translation = new(0, 0, 24) };
        AutomationProperties.SetAutomationId(card, "model-switch-confirm");
        AutomationProperties.SetName(card, heading.Text); overlay.Children.Add(card);
        modelSwitchRoot = XamlRoot;
        void Fit()
        {
            if (modelSwitchRoot == null) return;
            overlay.Width = modelSwitchRoot.Size.Width; overlay.Height = modelSwitchRoot.Size.Height;
            card.Width = Math.Max(0, Math.Min(480, overlay.Width - 40)); card.MaxHeight = Math.Max(0, overlay.Height - 40);
        }
        modelSwitchResize = (_, _) => Fit(); modelSwitchRoot.Changed += modelSwitchResize;
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, e) => e.Handled = HandleModelSwitchEscape();
        overlay.KeyboardAccelerators.Add(escape); overlay.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        modelSwitchDialog = new Popup { XamlRoot = XamlRoot, Child = overlay, IsLightDismissEnabled = false };
        Fit(); modelSwitchDialog.IsOpen = true; cancel.Focus(FocusState.Programmatic);
    }

    private async Task CommitHttpModelSwitchAsync(ComposerModelMenuChoice choice, (string? Project, string? Session, bool History) owner, bool skipFuture)
    {
        if (disposed || owner != (model.CurrentProjectId, model.CurrentSessionId, model.ShowingHistory) || !CanPickModel || model.AcpModelLocked) return;
        if (model.HasProjectDraft)
        {
            await model.SelectDraftModelAsync(choice.Id, lifetime.Token);
            return;
        }
        if (!await EnsureSessionForActionAsync() || disposed || owner.Project != model.CurrentProjectId
            || owner.Session != null && owner.Session != model.CurrentSessionId || !CanPickModel || model.AcpModelLocked) return;
        if (await model.SelectModelAsync(choice.Id, lifetime.Token) && !disposed && skipFuture)
            disableModelSwitchWarning?.Invoke();
    }

    private bool HandleModelSwitchEscape()
    {
        if (modelSwitchDialog == null) return false;
        CloseModelSwitchConfirmation(); return true;
    }
    private void CloseModelSwitchConfirmation(bool restoreFocus = true)
    {
        if (modelSwitchDialog == null) return;
        if (modelSwitchRoot != null && modelSwitchResize != null) modelSwitchRoot.Changed -= modelSwitchResize;
        modelSwitchRoot = null; modelSwitchResize = null;
        modelSwitchDialog.IsOpen = false; modelSwitchDialog.Child = null; modelSwitchDialog = null;
        if (restoreFocus && !disposed && models.IsLoaded) models.Focus(FocusState.Programmatic);
    }
}
