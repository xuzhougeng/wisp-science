using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly NativeActionWrap runtimeStrip = new() { Gap = 2, VerticallyCentered = true, Margin = new(18, 0, 18, 4) };
    private Popup? runtimeDialog;
    private Grid? runtimeOverlay, runtimeColumns;
    private Border? runtimeModalCard;
    private ScrollViewer? runtimeCardsScroll;
    private XamlRoot? runtimeRoot;
    private Windows.Foundation.TypedEventHandler<XamlRoot, XamlRootChangedEventArgs>? runtimeResize;
    private StackPanel? runtimeBody;
    private string? runtimeContext, runtimeBinding, runtimeVisualKey;
    private Button? runtimeReturnFocus;
    private readonly Dictionary<string, Button> runtimeContextButtons = [];
    private readonly Dictionary<string, Button> runtimeLanguageButtons = [];
    private string RuntimeBinding => model.CurrentProjectId + "/" + model.CurrentSessionId;

    public void DismissRuntimeSurfaces()
    {
        CloseRuntimeInterpreter(false);
        CloseRuntimeEnvironment(false);
        CloseRuntimeDialog(false);
    }

    private void RefreshRuntimeStrip()
    {
        if (disposed) return;
        var runtime = model.Runtimes;
        if (HasRuntimeModal && runtimeBinding != RuntimeBinding) DismissRuntimeSurfaces();
        // A polling response with unchanged contents must not remove the focused button.
        var key = RuntimeBinding + design.Dark + design.Localization.Locale + design.Typography
            + JsonSerializer.Serialize(runtime.Slots.Select(s => new { s.Context.Id, s.Context.Kind, s.Label, s.Language, s.Status,
                s.Info?.RuntimeId, s.Info?.Generation, s.Info?.Interpreter, s.Info?.LastError })) + runtime.Error + runtime.Busy + runtime.Snapshot?.ReadOnly;
        if (runtimeVisualKey == key) return;
        runtimeVisualKey = key; runtimeStrip.Children.Clear(); runtimeContextButtons.Clear();
        runtimeStrip.Visibility = runtime.Available && !model.AssistantMode ? Visibility.Visible : Visibility.Collapsed;
        AutomationProperties.SetName(runtimeStrip, design.T("runtime.strip_title"));
        if (runtime.Error != null && runtime.Slots.Length == 0)
        {
            var retry = RuntimeButton(design.L("重新读取运行环境", "Reload runtimes"), 12);
            ToolTipService.SetToolTip(retry, runtime.Error);
            retry.Click += async (_, _) => await runtime.RefreshAsync(lifetime.Token);
            runtimeStrip.Children.Add(retry);
        }
        foreach (var group in runtime.Slots.GroupBy(s => s.Context.Id))
        {
            var slot = group.First();
            var host = RuntimeButton(slot.Label, 12); host.MaxWidth = 160;
            host.BorderThickness = new(0); host.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            var label = new Grid { ColumnSpacing = 6 };
            label.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); label.ColumnDefinitions.Add(new());
            label.Children.Add(design.Icon(slot.Context.Kind == "local" ? "monitor" : "server", 14, design.ColorText("text-faint")));
            var text = design.Text(slot.Label, 12); text.TextTrimming = TextTrimming.CharacterEllipsis; text.TextWrapping = TextWrapping.NoWrap;
            text.Foreground = design.Brush(runtime.AllIdle ? "text-muted" : "text"); Grid.SetColumn(text, 1); label.Children.Add(text); host.Content = label;
            AutomationProperties.SetAutomationId(host, "runtime-context-" + slot.Context.Id);
            runtimeContextButtons[slot.Context.Id] = host;
            host.Click += async (_, _) => await ShowRuntimesAsync(slot.Context.Id, host);
            if (runtime.AllIdle) runtimeStrip.Children.Add(host);
            else
            {
                var controls = new NativeActionWrap { Gap = 6, VerticallyCentered = true }; controls.Children.Add(host);
                foreach (var item in group)
                {
                    var chip = RuntimeButton(item.LanguageLabel + "  " + NativeRuntimeStripModel.StatusLabel(item.Status, design.Localization), 11.5);
                    chip.CornerRadius = new(999); chip.Padding = new(8, 3, 6, 3); chip.Background = design.Brush("bg-sunken");
                    var chipContent = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
                    var language = design.Text(item.LanguageLabel, 11.5); language.FontWeight = Microsoft.UI.Text.FontWeights.Bold; chipContent.Children.Add(language);
                    chipContent.Children.Add(RuntimeStatusBadge(item.Status, true)); chip.Content = chipContent;
                    chip.Click += async (_, _) =>
                    {
                        if (!model.Runtimes.Inspector.Pinned) await ShowRuntimesAsync(item.Context.Id, chip);
                        await OpenRuntimeEnvironment(item.Context.Id, item.Language, chip);
                    };
                    controls.Children.Add(chip);
                }
                runtimeStrip.Children.Add(new Border { Child = controls, Padding = new(6, 6, 8, 6), CornerRadius = new(8),
                    Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1), Margin = new(0, 0, 6, 0) });
            }
        }
        if (runtime.AllIdle && runtime.Slots.Length > 0)
        {
            var summary = design.Text(runtime.IdleSummary(design.Localization), 11.5); summary.Foreground = design.Brush("text-faint"); summary.Margin = new(4, 0, 0, 0);
            runtimeStrip.Children.Add(summary);
        }
        RenderRuntimeDialog();
        RefreshRuntimeEnvironment();
    }
    private Button RuntimeButton(string label, double size = 12)
    {
        var button = new Button { Content = design.Text(label, size), Padding = new(6, 4, 6, 4), MinWidth = 0, MinHeight = 0,
            CornerRadius = new(5), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1) };
        AutomationProperties.SetName(button, label); return button;
    }
    private Border RuntimeStatusBadge(string status, bool compact = false)
    {
        var label = design.Text(NativeRuntimeStripModel.StatusLabel(status, design.Localization).ToUpperInvariant(), 10);
        label.FontWeight = Microsoft.UI.Text.FontWeights.Bold; label.CharacterSpacing = 40;
        label.Foreground = design.Brush(status is "busy" or "starting" or "stopping" ? "clay-strong" : "text-faint");
        Brush background = design.Brush(status is "busy" or "starting" or "stopping" ? "clay-soft" : "bg-sunken");
        if (status is "ready" or "dead")
        {
            var color = status == "ready" ? Windows.UI.Color.FromArgb(255, 53, 110, 84) : Windows.UI.Color.FromArgb(255, 195, 34, 34);
            label.Foreground = new SolidColorBrush(color);
            background = new SolidColorBrush(status == "dead" ? Windows.UI.Color.FromArgb(31, 228, 103, 103) : Windows.UI.Color.FromArgb(31, 53, 110, 84));
        }
        return new Border { CornerRadius = new(999), Padding = compact ? new(6, 1, 6, 1) : new(7, 2, 7, 2), Background = background, Child = label };
    }
    public async Task ShowPanelRuntimesAsync(string context, Button trigger, bool configure)
    {
        var binding = RuntimeBinding;
        await model.Runtimes.RefreshAsync(lifetime.Token);
        if (disposed || !trigger.IsLoaded || RuntimeBinding != binding) return;
        if (!configure) await ShowRuntimesAsync(context, trigger);
        else if (model.Runtimes.Snapshot is { ReadOnly: false }) ShowRuntimeInterpreter(context, "python", trigger, fromPanel: true);
    }
    private Task ShowRuntimesAsync(string context, Button trigger)
    {
        if (runtimeDialog != null || XamlRoot == null) return Task.CompletedTask;
        runtimeBinding = RuntimeBinding; runtimeContext = context; runtimeReturnFocus = trigger;
        modelMenu.Hide(); effortMenu.Hide(); composerOptions.Hide(); composeMenu.Hide();
        runtimeOverlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
        NativeDialogVisuals.Backdrop(runtimeOverlay, design);
        RegisterRuntimeEscape(runtimeOverlay);
        var layout = new Grid { Padding = new(22), RowSpacing = 16 };
        layout.RowDefinitions.Add(new() { Height = GridLength.Auto }); layout.RowDefinitions.Add(new());
        var header = new Grid(); header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = new StackPanel { Spacing = 3 };
        var heading = design.Text(design.T("contexts.runtimes"), 18); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.Children.Add(heading); title.Children.Add(design.Text(model.Runtimes.Slots.FirstOrDefault(s => s.Context.Id == context)?.Label ?? context, 12)); header.Children.Add(title);
        var close = design.ToolButton(design.L("关闭", "Close"), "close"); close.Click += (_, _) => CloseRuntimeDialog(); Grid.SetColumn(close, 1); header.Children.Add(close); layout.Children.Add(header);
        runtimeBody = new() { Spacing = 8 };
        runtimeCardsScroll = new ScrollViewer { Content = runtimeBody, Padding = new(1), HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        runtimeColumns = new Grid(); runtimeColumns.ColumnDefinitions.Add(new()); runtimeColumns.ColumnDefinitions.Add(new() { Width = new(0) }); runtimeColumns.Children.Add(runtimeCardsScroll);
        Grid.SetRow(runtimeColumns, 1); layout.Children.Add(runtimeColumns);
        runtimeModalCard = new Border { Child = layout, CornerRadius = new(16), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle,
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
        runtimeOverlay.Children.Add(runtimeModalCard);
        runtimeRoot = XamlRoot; runtimeResize = (_, _) => ResizeRuntimeDialog(); runtimeRoot.Changed += runtimeResize;
        runtimeDialog = new Popup { XamlRoot = XamlRoot, Child = runtimeOverlay, IsLightDismissEnabled = false };
        RenderRuntimeDialog(); ResizeRuntimeDialog(); runtimeDialog.IsOpen = true;
        close.Focus(FocusState.Programmatic); return Task.CompletedTask;
    }
    private void RegisterRuntimeEscape(UIElement target)
    {
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, e) => e.Handled = HandleRuntimeEscape(); target.KeyboardAccelerators.Add(escape);
        target.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
    }
    private void ResizeRuntimeDialog()
    {
        if (runtimeRoot == null || runtimeOverlay == null || runtimeModalCard == null || runtimeColumns == null) return;
        var size = runtimeRoot.Size; var environment = model.Runtimes.Inspector.Selected != null && !model.Runtimes.Inspector.Pinned;
        runtimeOverlay.Width = size.Width; runtimeOverlay.Height = size.Height;
        runtimeModalCard.Width = Math.Min(environment ? size.Width > 900 ? 1080 : 680 : 480, Math.Max(0, size.Width - 40));
        runtimeModalCard.Height = Math.Min(620, Math.Max(0, size.Height - 40));
        var split = environment && size.Width > 900;
        runtimeColumns.ColumnSpacing = split ? 12 : 0;
        runtimeColumns.ColumnDefinitions[0].Width = new(1, GridUnitType.Star);
        runtimeColumns.ColumnDefinitions[1].Width = split ? new(1.44, GridUnitType.Star) : new(0);
        if (runtimeEnvironmentView != null) Grid.SetColumn(runtimeEnvironmentView, split ? 1 : 0);
    }
    private void CloseRuntimeDialog(bool restoreFocus = true)
    {
        if (runtimeDialog == null) return;
        CloseRuntimeInterpreter(false);
        if (!model.Runtimes.Inspector.Pinned) CloseRuntimeEnvironment(false);
        if (runtimeRoot != null && runtimeResize != null) runtimeRoot.Changed -= runtimeResize;
        runtimeRoot = null; runtimeResize = null;
        runtimeDialog.IsOpen = false; runtimeDialog.Child = null; runtimeDialog = null;
        runtimeOverlay = null; runtimeModalCard = null; runtimeColumns = null; runtimeCardsScroll = null; runtimeBody = null;
        var button = runtimeReturnFocus is { IsLoaded: true } original ? original : runtimeContextButtons.GetValueOrDefault(runtimeContext ?? "");
        runtimeContext = null; runtimeReturnFocus = null;
        if (restoreFocus && !disposed && runtimeBinding == RuntimeBinding && button is { IsLoaded: true }) button.Focus(FocusState.Programmatic);
    }
    private void RenderRuntimeDialog()
    {
        if (runtimeBody == null || runtimeContext == null) return;
        runtimeBody.Children.Clear(); runtimeConfigureButtons.Clear(); runtimeLanguageButtons.Clear();
        var runtime = model.Runtimes;
        var count = runtime.Slots.Count(s => s.Context.Id == runtimeContext);
        var tools = new Grid(); tools.ColumnDefinitions.Add(new()); tools.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        tools.Children.Add(design.Text(design.T("contexts.runtimes").ToUpperInvariant() + "  " + count, 11));
        var refresh = design.ToolButton(design.T("runtime.refresh"), "refresh"); refresh.IsEnabled = !runtime.Busy;
        refresh.Click += async (_, _) => await runtime.RefreshAsync(lifetime.Token); Grid.SetColumn(refresh, 1); tools.Children.Add(refresh); runtimeBody.Children.Add(tools);
        runtimeBody.Children.Add(new Border { Background = design.Brush("clay-soft"), CornerRadius = new(5), Padding = new(8), Child = design.Text(design.T("runtime.state_warning"), 11) });
        if (runtime.Error != null) runtimeBody.Children.Add(design.Text(runtime.Error, 12));
        foreach (var slot in runtime.Slots.Where(s => s.Context.Id == runtimeContext))
        {
            var card = new StackPanel { Spacing = 8 };
            var row = new Grid(); row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var title = RuntimeButton(slot.LanguageLabel, 13); title.BorderThickness = new(0); title.Padding = new(0); title.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
            var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 }; var titleText = design.Text(slot.LanguageLabel, 13); titleText.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
            label.Children.Add(titleText); label.Children.Add(design.Icon("chevron-right", 10)); title.Content = label;
            AutomationProperties.SetName(title, design.Localization.Format("runtime.open_environment", ("language", slot.LanguageLabel)));
            title.Click += async (_, _) => await OpenRuntimeEnvironment(slot.Context.Id, slot.Language, title); row.Children.Add(title);
            runtimeLanguageButtons[slot.Language] = title;
            var status = RuntimeStatusBadge(slot.Status);
            Grid.SetColumn(status, 1); row.Children.Add(status); card.Children.Add(row);
            var name = sessionReferenceScope?.Invoke()?.ProjectName;
            card.Children.Add(design.Text((string.IsNullOrEmpty(name) ? "" : name + " · ") + slot.Label, 11));
            if (slot.Info?.Interpreter is { Length: > 0 } path) card.Children.Add(design.Text(path, 11));
            if (slot.Info?.LastError is { Length: > 0 } error) card.Children.Add(design.Text(error, 12));
            if (slot.Status == "unavailable") card.Children.Add(design.Text(design.T("runtime.unavailable_hint"), 12));
            var actions = new NativeActionWrap { Gap = 6, RightAligned = true };
            void Action(string action)
            {
                var button = RuntimeButton(design.T("runtime." + action), 11);
                button.IsEnabled = !runtime.Busy && runtime.Snapshot is { ReadOnly: false };
                button.Click += async (_, _) => await runtime.ActAsync(action, slot, lifetime.Token); actions.Children.Add(button);
            }
            if (runtime.Available)
            {
                var configure = RuntimeButton(design.T("runtime.configure"), 11); configure.IsEnabled = !runtime.Busy;
                configure.Click += (_, _) => ShowRuntimeInterpreter(slot.Context.Id, slot.Language, configure); actions.Children.Add(configure);
                runtimeConfigureButtons[slot.Language] = configure;
            }
            if (slot.Status == "missing") Action("start");
            if (slot.Status is "ready" or "busy" or "starting") Action("stop");
            if (slot.Status is "ready" or "busy" or "dead") Action("restart");
            if (slot.Status == "dead") Action("dismiss");
            card.Children.Add(actions);
            runtimeBody.Children.Add(new Border { Child = card, Padding = new(12), CornerRadius = new(10), BorderThickness = new(1), BorderBrush = design.Brush("border"), Background = design.Brush("bg-elev") });
        }
    }
}
