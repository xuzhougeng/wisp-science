using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Func<NativeHandoffLaunch, Task>? startHandoff;
    private Popup? handoffDialog;
    private XamlRoot? handoffRoot;
    private Windows.Foundation.TypedEventHandler<XamlRoot, XamlRootChangedEventArgs>? handoffResize;
    private Action? updateHandoffDialog;

    private void RefreshHandoffDialog()
    {
        var state = model.Handoff;
        if (!state.Visible || model.ShowingHistory) { CloseHandoffDialog(); return; }
        if (!IsLoaded || XamlRoot == null) return;
        if (handoffDialog != null) { updateHandoffDialog?.Invoke(); return; }
        var overlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
        NativeDialogVisuals.Backdrop(overlay, design);
        var body = new StackPanel { Spacing = 14 };
        var head = new Grid { ColumnSpacing = 12 };
        head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = design.Text(design.T("handoff.title"), 18); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        var titles = new StackPanel { Spacing = 6 }; titles.Children.Add(title);
        var subtitle = design.Text(design.T("handoff.subtitle"), 13); subtitle.Foreground = design.Brush("text-muted"); titles.Children.Add(subtitle);
        head.Children.Add(titles);
        var close = new Button { Content = design.Icon("close", 18), VerticalAlignment = VerticalAlignment.Top };
        design.QuietButton(close); AutomationProperties.SetName(close, design.T("handoff.close"));
        close.Click += (_, _) => state.Close(); Grid.SetColumn(close, 1); head.Children.Add(close); body.Children.Add(head);
        TextBox Field(string label, string placeholder, double height)
        {
            var field = new StackPanel { Spacing = 6 };
            var labelText = design.Text(design.T(label), 12); labelText.Foreground = design.Brush("text-muted"); field.Children.Add(labelText);
            var input = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = height,
                PlaceholderText = design.T(placeholder), HorizontalAlignment = HorizontalAlignment.Stretch,
                Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1), CornerRadius = new(8) };
            design.BindTypography(input, size: 13); AutomationProperties.SetName(input, design.T(label)); field.Children.Add(input); body.Children.Add(field); return input;
        }
        var instruction = Field("handoff.instruction_label", "handoff.instruction_placeholder", 56);
        var progress = new Grid { ColumnSpacing = 8 };
        progress.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); progress.ColumnDefinitions.Add(new());
        progress.Children.Add(new ProgressRing { IsActive = true, Width = 18, Height = 18 });
        var generating = design.Text(design.T("handoff.generating"), 13); Grid.SetColumn(generating, 1); progress.Children.Add(generating); body.Children.Add(progress);
        AutomationProperties.SetLiveSetting(generating, AutomationLiveSetting.Polite);
        var error = design.Text("", 13); error.Foreground = design.Brush("err"); body.Children.Add(error);
        AutomationProperties.SetLiveSetting(error, AutomationLiveSetting.Assertive);
        var draft = Field("handoff.draft_label", "handoff.draft_placeholder", 240);
        design.BindTypography(draft, code: true, size: 12.5);
        var updating = false;
        instruction.TextChanged += (_, _) => { if (!updating) state.SetInstruction(instruction.Text); };
        draft.TextChanged += (_, _) => { if (!updating) state.SetDraft(draft.Text); };
        var actions = new NativeActionWrap { Gap = 10, RowGap = 8, RightAligned = true, Margin = new(0, 18, 0, 0) };
        var cancel = new Button { Content = design.T("handoff.cancel") }; design.ActionButton(cancel);
        var regenerate = new Button { Content = design.T("handoff.regenerate") }; design.ActionButton(regenerate);
        var start = new Button { Content = design.T("handoff.start") }; design.ActionButton(start, true);
        foreach (var button in new[] { cancel, regenerate, start }) { design.BindTypography(button, size: 13.5); actions.Children.Add(button); }
        cancel.Click += (_, _) => state.Close(); regenerate.Click += async (_, _) => await state.GenerateAsync(lifetime.Token);
        start.Click += async (_, _) => { if (startHandoff != null && state.CanStart) { CloseContextUsage(); await state.StartAsync(startHandoff); } };
        var scroll = new ScrollViewer { Content = body, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var layout = new Grid { Padding = new(22) }; layout.RowDefinitions.Add(new()); layout.RowDefinitions.Add(new() { Height = GridLength.Auto });
        layout.Children.Add(scroll); Grid.SetRow(actions, 1); layout.Children.Add(actions);
        var card = new Border { Child = layout, CornerRadius = new(16), Background = design.Brush("bg-elev"),
            BorderBrush = design.Brush("border-strong"), BorderThickness = new(1), HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle,
            Shadow = new Microsoft.UI.Xaml.Media.ThemeShadow(), Translation = new(0, 0, 24) };
        AutomationProperties.SetAutomationId(card, "handoff-modal"); overlay.Children.Add(card);
        updateHandoffDialog = () =>
        {
            updating = true;
            if (instruction.Text != state.Instruction) instruction.Text = state.Instruction;
            if (draft.Text != state.Draft) draft.Text = state.Draft;
            updating = false;
            instruction.IsEnabled = draft.IsEnabled = !state.Generating && !state.Starting;
            cancel.IsEnabled = close.IsEnabled = !state.Starting;
            regenerate.IsEnabled = state.Available && !state.Generating && !state.Starting && !state.Uncertain;
            start.IsEnabled = state.CanStart && startHandoff != null;
            progress.Visibility = state.Generating || state.Starting ? Visibility.Visible : Visibility.Collapsed;
            generating.Text = state.Starting ? design.L("正在打开新对话…", "Opening new session…") : design.T("handoff.generating");
            error.Text = state.Error ?? "";
            if (state.Uncertain) error.Text += "\n" + design.L("请检查会话列表确认结果。不会自动再次创建或发送。", "Check the session list to confirm the result. Creation and sending are not retried automatically.");
            error.Visibility = error.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        };
        handoffRoot = XamlRoot;
        void Fit() { if (handoffRoot == null) return; overlay.Width = handoffRoot.Size.Width; overlay.Height = handoffRoot.Size.Height;
            card.Width = Math.Max(0, Math.Min(680, overlay.Width - 32)); card.MaxHeight = Math.Max(0, overlay.Height - 32);
            draft.MaxHeight = Math.Max(240, overlay.Height * .5); }
        handoffResize = (_, _) => Fit(); handoffRoot.Changed += handoffResize;
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, e) => e.Handled = HandleHandoffEscape(); overlay.KeyboardAccelerators.Add(escape);
        overlay.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        handoffDialog = new Popup { XamlRoot = XamlRoot, Child = overlay, IsLightDismissEnabled = false };
        Fit(); updateHandoffDialog(); handoffDialog.IsOpen = true; scroll.Focus(FocusState.Programmatic);
    }
    private bool HandleHandoffEscape() { if (handoffDialog == null) return false; model.Handoff.Close(); return true; }
    private void CloseHandoffDialog()
    {
        if (handoffRoot != null && handoffResize != null) handoffRoot.Changed -= handoffResize;
        handoffRoot = null; handoffResize = null; updateHandoffDialog = null;
        if (handoffDialog != null) { handoffDialog.IsOpen = false; handoffDialog.Child = null; handoffDialog = null; }
    }
}
