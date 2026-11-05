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
    private Popup? idleCompactDialog;
    private XamlRoot? idleCompactRoot;
    private Windows.Foundation.TypedEventHandler<XamlRoot, XamlRootChangedEventArgs>? idleCompactResize;
    private NativeIdleCompactionPrompt? idleCompactOwner;
    private string? idleCompactAppearance;

    private void RefreshIdleCompactionDialog()
    {
        var prompt = model.IdleCompactionPrompt;
        var appearance = $"{design.Localization.Locale}/{design.Dark}/{design.Typography}";
        var available = !disposed && IsLoaded && XamlRoot != null && model.CanShowIdleCompaction
            && canShowRunReview?.Invoke() != false && !openingRunReview && historyDialog == null
            && compactDialog == null && handoffDialog == null && contextRecoveryDialog == null && model.ContextRecovery == null && !HasRuntimeModal && queueMenu == null
            && !composerOptionsOpen && !modelMenuOpen && !contextUsageOpen && selectionFlyout == null;
        if (!available || idleCompactOwner != prompt || idleCompactAppearance != appearance) CloseIdleCompactionDialog();
        if (!available || prompt == null || idleCompactDialog != null) return;
        // Do not place automatic advice over a component-local menu/dialog.
        // The normal conversation refresh can show it when that surface closes.
        if (VisualTreeHelper.GetOpenPopupsForXamlRoot(XamlRoot).Any(popup => popup.IsOpen)) return;
        idleCompactOwner = prompt; idleCompactAppearance = appearance;
        var overlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
        NativeDialogVisuals.Backdrop(overlay, design);
        var content = new StackPanel { Spacing = 16, Padding = new(22) };
        var title = design.Text(design.T("compact.idle_title"), 18); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        content.Children.Add(title);
        var body = design.Text(design.Localization.Format("compact.idle_body", ("hours", prompt.Hours.ToString())), 12);
        body.Foreground = design.Brush("text-muted"); design.BindLineHeight(body, 12, 1.45); content.Children.Add(body);
        var actions = new NativeActionWrap { Gap = 10, RowGap = 8 };
        var dismiss = new Button { Content = design.T("compact.idle_dismiss") }; design.ActionButton(dismiss);
        var accept = new Button { Content = design.T("compact.idle_accept") }; design.ActionButton(accept, true);
        AutomationProperties.SetAutomationId(dismiss, "compact-idle-dismiss"); AutomationProperties.SetAutomationId(accept, "compact-idle-accept");
        dismiss.Click += (_, _) => model.DismissIdleCompaction();
        accept.Click += (_, _) => { if (model.IdleCompactionPrompt == prompt) model.AcceptIdleCompaction(); };
        actions.Children.Add(dismiss); actions.Children.Add(accept); content.Children.Add(actions);
        var scroll = new ScrollViewer { Content = content, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var card = new Border { Child = scroll, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"),
            BorderThickness = new(1), CornerRadius = new(16), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
            TabFocusNavigation = KeyboardNavigationMode.Cycle, Shadow = new ThemeShadow(), Translation = new(0, 0, 24) };
        AutomationProperties.SetAutomationId(card, "compact-idle-prompt"); AutomationProperties.SetName(card, title.Text); overlay.Children.Add(card);
        idleCompactRoot = XamlRoot;
        void Fit() { if (idleCompactRoot == null) return; overlay.Width = idleCompactRoot.Size.Width; overlay.Height = idleCompactRoot.Size.Height;
            card.Width = Math.Max(0, Math.Min(560, overlay.Width - 32)); card.MaxHeight = Math.Max(0, overlay.Height - 32); }
        idleCompactResize = (_, _) => Fit(); idleCompactRoot!.Changed += idleCompactResize;
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, e) => e.Handled = HandleIdleCompactionEscape(); overlay.KeyboardAccelerators.Add(escape);
        overlay.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        idleCompactDialog = new Popup { XamlRoot = XamlRoot, Child = overlay, IsLightDismissEnabled = false };
        Fit(); idleCompactDialog.IsOpen = true; dismiss.Focus(FocusState.Programmatic);
    }
    private bool HandleIdleCompactionEscape()
    {
        if (idleCompactDialog == null) return false;
        model.DismissIdleCompaction(); return true;
    }
    private void CloseIdleCompactionDialog()
    {
        if (idleCompactRoot != null && idleCompactResize != null) idleCompactRoot.Changed -= idleCompactResize;
        idleCompactRoot = null; idleCompactResize = null; idleCompactOwner = null; idleCompactAppearance = null;
        if (idleCompactDialog != null) { idleCompactDialog.IsOpen = false; idleCompactDialog.Child = null; idleCompactDialog = null; }
    }
}
