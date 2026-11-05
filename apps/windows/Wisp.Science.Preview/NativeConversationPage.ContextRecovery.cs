using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Func<NativeContextRecoveryLaunch, Task>? startContextRecovery;
    private readonly Func<bool>? canShowContextRecovery;
    private Popup? contextRecoveryDialog;
    private XamlRoot? contextRecoveryRoot;
    private Windows.Foundation.TypedEventHandler<XamlRoot, XamlRootChangedEventArgs>? contextRecoveryResize;
    private NativeContextRecovery? contextRecoveryOwner;
    private string? contextRecoveryAppearance;
    private Action? updateContextRecovery;

    private void RefreshContextRecoveryDialog()
    {
        var current = model.ContextRecovery;
        var identity = current is null ? null : current with { Starting = false, Uncertain = false, Error = null };
        var appearance = $"{design.Localization.Locale}/{design.Dark}/{design.Typography}";
        var available = !disposed && IsLoaded && XamlRoot != null && model.CanShowContextRecovery
            && canShowContextRecovery?.Invoke() != false && !openingRunReview && historyDialog == null
            && compactDialog == null && handoffDialog == null && !HasRuntimeModal;
        if (!available || contextRecoveryOwner != identity || contextRecoveryAppearance != appearance) CloseContextRecoveryDialog();
        if (!available || current == null) return;
        if (contextRecoveryDialog != null) { updateContextRecovery?.Invoke(); return; }
        // A new provider failure replaces transient composer menus, as WebView
        // does. Existing deliberate dialogs remain above this pending chooser.
        selectionFlyout?.Hide(); optionSubmenu.Hide(); composerOptions.Hide(); modelMenu.Hide(); effortMenu.Hide(); queueMenu?.Hide();
        CloseIdleCompactionDialog();
        if (VisualTreeHelper.GetOpenPopupsForXamlRoot(XamlRoot).Any(popup => popup.IsOpen)) return;
        contextRecoveryOwner = identity; contextRecoveryAppearance = appearance;
        var overlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
        NativeDialogVisuals.Backdrop(overlay, design);
        var content = new StackPanel { Padding = new(22), Spacing = 16 };
        var title = design.Text(design.T("context_recovery.title"), 18); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        content.Children.Add(title);
        var body = design.Text(design.T("context_recovery.body"), 13); body.Foreground = design.Brush("text-muted");
        design.BindLineHeight(body, 13, 1.55); content.Children.Add(body);
        var choices = new StackPanel { Spacing = 9 }; content.Children.Add(choices);
        Button Choice(string key, string id, bool recommended = false)
        {
            var labels = new StackPanel { Spacing = 4 };
            var name = design.Text(design.T($"context_recovery.{key}"), 13.5); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            var hint = design.Text(design.T($"context_recovery.{key}_hint"), 12); hint.Foreground = design.Brush("text-muted");
            design.BindLineHeight(hint, 12, 1.45); labels.Children.Add(name); labels.Children.Add(hint);
            var button = new Button { Content = labels, HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch, Padding = new(13, 12, 13, 12), CornerRadius = new(8), BorderThickness = new(1),
                Background = recommended ? Blend("clay", "bg-elev", .07) : design.Brush("bg-elev"),
                BorderBrush = recommended ? Blend("clay", "border-strong", .6) : design.Brush("border-strong") };
            button.Resources["ButtonBackgroundPointerOver"] = design.Brush("surface-hover");
            button.Resources["ButtonBorderBrushPointerOver"] = design.Brush("text-faint");
            button.Resources["ButtonBackgroundDisabled"] = button.Background;
            button.Resources["ButtonBorderBrushDisabled"] = button.BorderBrush;
            button.Resources["ButtonForegroundDisabled"] = design.Brush("text");
            AutomationProperties.SetName(button, name.Text); AutomationProperties.SetHelpText(button, hint.Text);
            AutomationProperties.SetAutomationId(button, "context-recovery-" + id); choices.Children.Add(button); return button;
        }
        var compact = Choice("compact", "compact", true);
        var fresh = Choice("new_session", "new-session");
        var pause = Choice("pause", "pause");
        compact.Click += (_, _) => model.CompactAndContinue();
        fresh.Click += async (_, _) => { if (startContextRecovery != null) await model.StartContextRecoverySessionAsync(design.T("context_recovery.new_prompt"), startContextRecovery); };
        pause.Click += (_, _) => model.DismissContextRecovery();
        var progress = design.Text(design.T("context_recovery.starting"), 12.5); progress.Foreground = design.Brush("text-muted"); content.Children.Add(progress);
        AutomationProperties.SetLiveSetting(progress, AutomationLiveSetting.Polite);
        var error = design.Text("", 12.5); error.Foreground = design.Brush("err"); design.BindLineHeight(error, 12.5, 1.5);
        var errorBox = new Border { Child = error, Padding = new(11, 9, 11, 9), CornerRadius = new(6), BorderThickness = new(1),
            Background = Blend("err", "bg-elev", .09), BorderBrush = Blend("err", "bg-elev", .28) };
        AutomationProperties.SetLiveSetting(error, AutomationLiveSetting.Assertive); content.Children.Add(errorBox);
        var scroll = new ScrollViewer { Content = content, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var card = new Border { Child = scroll, Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"),
            BorderThickness = new(1), CornerRadius = new(16), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
            TabFocusNavigation = KeyboardNavigationMode.Cycle, Shadow = new ThemeShadow(), Translation = new(0, 0, 24) };
        AutomationProperties.SetAutomationId(card, "context-recovery-modal"); AutomationProperties.SetName(card, title.Text); overlay.Children.Add(card);
        updateContextRecovery = () =>
        {
            if (model.ContextRecovery is not { } state) return;
            compact.IsEnabled = model.CanRecoverContext;
            fresh.IsEnabled = model.CanRecoverContext && startContextRecovery != null && !state.Uncertain;
            pause.IsEnabled = !state.Starting;
            foreach (var option in new[] { compact, fresh, pause }) option.Opacity = option.IsEnabled ? 1 : .55;
            progress.Visibility = state.Starting ? Visibility.Visible : Visibility.Collapsed;
            error.Text = state.Error ?? "";
            if (state.Uncertain) error.Text += "\n" + design.L("新建结果未确认；不会自动重试。请返回会话列表核对。", "Creation was not confirmed and will not be retried automatically. Check the conversation list before starting again.");
            errorBox.Visibility = error.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        };
        contextRecoveryRoot = XamlRoot;
        void Fit() { if (contextRecoveryRoot == null) return; overlay.Width = contextRecoveryRoot.Size.Width; overlay.Height = contextRecoveryRoot.Size.Height;
            card.Width = Math.Max(0, Math.Min(560, overlay.Width - 32)); card.MaxHeight = Math.Max(0, overlay.Height - 32); }
        contextRecoveryResize = (_, _) => Fit(); contextRecoveryRoot!.Changed += contextRecoveryResize;
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, e) => e.Handled = HandleContextRecoveryEscape(); overlay.KeyboardAccelerators.Add(escape);
        overlay.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        contextRecoveryDialog = new Popup { XamlRoot = XamlRoot, Child = overlay, IsLightDismissEnabled = false };
        Fit(); updateContextRecovery(); contextRecoveryDialog.IsOpen = true; compact.Focus(FocusState.Programmatic);

        SolidColorBrush Blend(string foreground, string background, double ratio)
        {
            var a = design.Brush(foreground).Color; var b = design.Brush(background).Color;
            byte Mix(byte x, byte y) => (byte)Math.Round(x * ratio + y * (1 - ratio));
            return new(Windows.UI.Color.FromArgb(255, Mix(a.R, b.R), Mix(a.G, b.G), Mix(a.B, b.B)));
        }
    }
    public bool HandleContextRecoveryEscape()
    {
        if (contextRecoveryDialog == null) return false;
        model.DismissContextRecovery(); return true;
    }
    private void CloseContextRecoveryDialog()
    {
        if (contextRecoveryRoot != null && contextRecoveryResize != null) contextRecoveryRoot.Changed -= contextRecoveryResize;
        contextRecoveryRoot = null; contextRecoveryResize = null; contextRecoveryOwner = null; contextRecoveryAppearance = null; updateContextRecovery = null;
        if (contextRecoveryDialog != null) { contextRecoveryDialog.IsOpen = false; contextRecoveryDialog.Child = null; contextRecoveryDialog = null; }
    }
}
