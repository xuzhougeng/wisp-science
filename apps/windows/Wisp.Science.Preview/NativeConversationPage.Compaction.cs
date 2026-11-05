using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private Popup? compactDialog;
    private XamlRoot? compactRoot;
    private Windows.Foundation.TypedEventHandler<XamlRoot, XamlRootChangedEventArgs>? compactResize;
    private (string? Project, string? Session) compactOwner;
    private Action? updateCompactDialog;

    private void RefreshCompactionDialog()
    {
        var state = model.Compaction;
        var owner = (model.CurrentProjectId, model.CurrentSessionId);
        if (compactOwner != owner || state?.Visible != true || model.ShowingHistory) CloseCompactionDialog();
        if (state?.Visible != true || model.ShowingHistory || !IsLoaded || XamlRoot == null) return;
        if (compactDialog != null) { updateCompactDialog?.Invoke(); return; }
        compactOwner = owner;
        var semantic = state.Semantic;
        var overlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
        NativeDialogVisuals.Backdrop(overlay, design);
        var body = new StackPanel { Spacing = 14, Padding = new(22) };
        var head = new Grid { ColumnSpacing = 12 };
        head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var titles = new StackPanel { Spacing = 6 };
        var title = design.Text(design.T("compact.title"), 18); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        titles.Children.Add(title); titles.Children.Add(Note("compact.subtitle", 13)); head.Children.Add(titles);
        var close = new Button { Content = design.Icon("close", 18), VerticalAlignment = VerticalAlignment.Top };
        design.QuietButton(close); AutomationProperties.SetName(close, design.T("compact.close")); Grid.SetColumn(close, 1); head.Children.Add(close);
        close.Click += (_, _) => model.DismissCompaction(); body.Children.Add(head);
        var modes = new Grid { ColumnSpacing = 8 };
        modes.ColumnDefinitions.Add(new()); modes.ColumnDefinitions.Add(new());
        Button Mode(string label, string hint, bool value)
        {
            var content = new StackPanel { Spacing = 4 };
            var name = design.Text(design.T(label), 13); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            content.Children.Add(name); content.Children.Add(Note(hint, 12));
            var button = new Button { Content = content, Padding = new(12, 10, 12, 10), BorderThickness = new(1),
                CornerRadius = new(8), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                VerticalAlignment = VerticalAlignment.Stretch, VerticalContentAlignment = VerticalAlignment.Top };
            AutomationProperties.SetName(button, design.T(label));
            button.Click += (_, _) => model.UpdateCompactionDraft(value, model.Compaction?.Instruction ?? "");
            return button;
        }
        var regular = Mode("compact.mode_regular", "compact.mode_regular_hint", false);
        var summary = Mode("compact.mode_semantic", "compact.mode_semantic_hint", true);
        modes.Children.Add(regular); Grid.SetColumn(summary, 1); modes.Children.Add(summary); body.Children.Add(modes);
        var instructionField = new StackPanel { Spacing = 6 };
        instructionField.Children.Add(design.Text(design.T("compact.instruction_label"), 13));
        var instruction = new TextBox { Text = state.Instruction, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 96,
            PlaceholderText = design.T("compact.instruction_placeholder"), HorizontalAlignment = HorizontalAlignment.Stretch,
            BorderThickness = new(1), CornerRadius = new(8), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong") };
        AutomationProperties.SetName(instruction, design.T("compact.instruction_label")); design.BindTypography(instruction, size: 13);
        instruction.TextChanged += (_, _) => model.UpdateCompactionDraft(semantic, instruction.Text);
        instructionField.Children.Add(instruction); body.Children.Add(instructionField);
        var hint = Note("compact.hint_semantic", 12); body.Children.Add(hint);
        var progress = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        progress.Children.Add(new ProgressRing { IsActive = true, Width = 18, Height = 18 });
        var progressText = Note("compact.running", 13); var progressGrid = new Grid { ColumnSpacing = 8 };
        progressGrid.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); progressGrid.ColumnDefinitions.Add(new());
        progressGrid.Children.Add(progress); Grid.SetColumn(progressText, 1); progressGrid.Children.Add(progressText); body.Children.Add(progressGrid);
        AutomationProperties.SetLiveSetting(progressText, AutomationLiveSetting.Polite);
        var error = design.Text("", 13); error.Foreground = design.Brush("clay-strong"); body.Children.Add(error);
        AutomationProperties.SetLiveSetting(error, AutomationLiveSetting.Assertive);
        var actions = new NativeActionWrap { Gap = 10, RowGap = 8, RightAligned = true };
        var cancel = new Button { Content = design.T("compact.cancel") }; design.ActionButton(cancel);
        var reload = new Button { Content = design.L("重新读取", "Reload") }; design.ActionButton(reload);
        var start = new Button { Content = design.T("compact.start") }; design.ActionButton(start, true);
        foreach (var button in new[] { cancel, reload, start }) { design.BindTypography(button, size: 13.5); actions.Children.Add(button); }
        cancel.Click += (_, _) => model.DismissCompaction(); reload.Click += async (_, _) => await model.RefreshAsync(lifetime.Token);
        start.Click += async (_, _) => await model.StartCompactionAsync(semantic, instruction.Text, lifetime.Token); body.Children.Add(actions);
        var contentScroll = new ScrollViewer { Content = body, IsTabStop = true, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var card = new Border { Child = contentScroll, CornerRadius = new(16), Background = design.Brush("bg-elev"),
            BorderBrush = design.Brush("border-strong"), BorderThickness = new(1), HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle,
            Shadow = new ThemeShadow(), Translation = new(0, 0, 24) };
        AutomationProperties.SetAutomationId(card, "compact-modal"); AutomationProperties.SetName(card, title.Text); overlay.Children.Add(card);
        var wasBusy = false;
        updateCompactDialog = () =>
        {
            if (model.Compaction is not { } current) return;
            semantic = current.Semantic;
            if (instruction.Text != current.Instruction) instruction.Text = current.Instruction;
            var locked = current.Busy || current.Uncertain;
            regular.IsEnabled = summary.IsEnabled = instruction.IsEnabled = !locked;
            regular.BorderBrush = design.Brush(semantic ? "border-strong" : "clay-strong");
            summary.BorderBrush = design.Brush(semantic ? "clay-strong" : "border-strong");
            regular.Resources["ButtonBorderBrushDisabled"] = regular.BorderBrush;
            summary.Resources["ButtonBorderBrushDisabled"] = summary.BorderBrush;
            regular.Background = summary.Background = design.Brush("bg-elev");
            instructionField.Visibility = semantic ? Visibility.Visible : Visibility.Collapsed;
            hint.Text = design.T(semantic ? "compact.hint_semantic" : "compact.hint_regular");
            progressGrid.Visibility = current.Busy ? Visibility.Visible : Visibility.Collapsed;
            actions.Visibility = close.Visibility = current.Busy ? Visibility.Collapsed : Visibility.Visible;
            if (current.Busy && !wasBusy && compactDialog?.IsOpen == true) contentScroll.Focus(FocusState.Programmatic);
            wasBusy = current.Busy;
            reload.Visibility = current.Uncertain || model.ConnectionError != null ? Visibility.Visible : Visibility.Collapsed;
            start.IsEnabled = model.CanRunCompaction;
            error.Text = current.Uncertain ? design.L("压缩结果尚未确认。请重新读取核对；再次提交会沿用原请求，不会自动重试。", "Compaction is unconfirmed. Reload to check; submitting again reuses the original request. Nothing is retried automatically.") + "\n" + current.Error
                : current.Error ?? model.ConnectionError ?? "";
            error.Visibility = error.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        };
        compactRoot = XamlRoot;
        void Fit() { if (compactRoot == null) return; overlay.Width = compactRoot.Size.Width; overlay.Height = compactRoot.Size.Height;
            card.Width = Math.Max(0, Math.Min(560, overlay.Width - 32)); card.MaxHeight = Math.Max(0, overlay.Height - 32); }
        compactResize = (_, _) => Fit(); compactRoot.Changed += compactResize;
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, e) => e.Handled = HandleCompactionEscape(); overlay.KeyboardAccelerators.Add(escape);
        overlay.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        compactDialog = new Popup { XamlRoot = XamlRoot, Child = overlay, IsLightDismissEnabled = false };
        Fit(); updateCompactDialog(); compactDialog.IsOpen = true;
        contentScroll.Focus(FocusState.Programmatic);

        TextBlock Note(string key, double size) { var text = design.Text(design.T(key), size); text.Foreground = design.Brush("text-muted"); text.LineHeight = design.FontSize(size) * 1.5; return text; }
    }

    private bool HandleCompactionEscape()
    {
        if (compactDialog == null) return false;
        model.DismissCompaction(); return true;
    }
    private void CloseCompactionDialog()
    {
        if (compactRoot != null && compactResize != null) compactRoot.Changed -= compactResize;
        compactRoot = null; compactResize = null; updateCompactDialog = null;
        if (compactDialog != null) { compactDialog.IsOpen = false; compactDialog.Child = null; compactDialog = null; }
    }
}
