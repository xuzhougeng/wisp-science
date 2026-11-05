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
    private NativeRuntimeInterpreterModel? runtimeInterpreter;
    private Popup? runtimeInterpreterPopup;
    private XamlRoot? runtimeInterpreterRoot;
    private Windows.Foundation.TypedEventHandler<XamlRoot, XamlRootChangedEventArgs>? runtimeInterpreterResize;
    private Action? runtimeInterpreterChanged;
    private SettingsTextMenus? runtimeInterpreterMenus;
    private Button? runtimeInterpreterReturnFocus;
    private string? runtimeInterpreterLanguage;
    private readonly Dictionary<string, Button> runtimeConfigureButtons = [];
    public bool HasRuntimeModal => runtimeDialog != null || runtimeInterpreter != null;

    public bool HandleRuntimeEscape()
    {
        if (runtimeInterpreter != null)
        {
            if (runtimeInterpreterMenus?.CloseTop() != true && runtimeInterpreter.CanSave) CloseRuntimeInterpreter();
            return true;
        }
        if (runtimeEnvironmentMenus?.CloseTop() == true) return true;
        if (model.Runtimes.Inspector.Selected != null && (!model.Runtimes.Inspector.Pinned || runtimeDialog == null)) { CloseRuntimeEnvironment(); return true; }
        if (runtimeDialog == null) return false;
        CloseRuntimeDialog(); return true;
    }

    private void CloseRuntimeInterpreter(bool restoreFocus = true)
    {
        if (runtimeInterpreter == null) return;
        runtimeInterpreter.Changed -= runtimeInterpreterChanged; runtimeInterpreter.Dispose(); runtimeInterpreter = null;
        runtimeInterpreterMenus?.Dispose(); runtimeInterpreterMenus = null;
        if (runtimeInterpreterRoot != null && runtimeInterpreterResize != null) runtimeInterpreterRoot.Changed -= runtimeInterpreterResize;
        runtimeInterpreterRoot = null; runtimeInterpreterResize = null; runtimeInterpreterChanged = null;
        if (runtimeInterpreterPopup != null) { runtimeInterpreterPopup.IsOpen = false; runtimeInterpreterPopup.Child = null; runtimeInterpreterPopup = null; }
        var button = runtimeInterpreterReturnFocus is { IsLoaded: true } original ? original
            : runtimeConfigureButtons.GetValueOrDefault(runtimeInterpreterLanguage ?? "");
        if (restoreFocus && !disposed && runtimeBinding == RuntimeBinding && button is { IsLoaded: true }) button.Focus(FocusState.Programmatic);
        runtimeInterpreterReturnFocus = null; runtimeInterpreterLanguage = null;
    }

    private void ShowRuntimeInterpreter(string contextId, string language, Button trigger, bool fromPanel = false)
    {
        if (runtimeInterpreter != null || !fromPanel && runtimeDialog == null || XamlRoot == null
            || model.Runtimes.InterpreterEditor(contextId) is not { } editor) return;
        if (fromPanel) runtimeBinding = RuntimeBinding;
        runtimeInterpreter = editor; runtimeInterpreterReturnFocus = trigger; runtimeInterpreterLanguage = language;
        var menus = runtimeInterpreterMenus = new SettingsTextMenus();
        var overlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
        // A Popup has its own visual tree. Register an unscoped accelerator while
        // mounted so Escape works immediately, including before input gets focus.
        var escape = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        escape.Invoked += (_, e) => { if (ReferenceEquals(runtimeInterpreter, editor)) e.Handled = HandleRuntimeEscape(); };
        overlay.KeyboardAccelerators.Add(escape);
        overlay.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        NativeDialogVisuals.Backdrop(overlay, design);
        AutomationProperties.SetName(overlay, design.T("runtime_config.title"));
        var body = new StackPanel { Padding = new(22), Spacing = 14 };
        var inputs = new List<Control>();
        TextBlock Label(string text, double size = 12, string color = "text-muted")
        { var result = design.Text(text, size); result.Foreground = design.Brush(color); return result; }
        Button Button(string text, bool primary = false)
        {
            var label = Label(text, 13.5, primary ? "on-clay" : "text");
            var button = new Button { Content = label, Padding = new(16, 9, 16, 9), MinHeight = 0, MinWidth = 0,
                CornerRadius = new(8), Background = design.Brush(primary ? "clay" : "bg-elev"),
                BorderBrush = design.Brush(primary ? "clay" : "border-strong"), BorderThickness = new(1) };
            AutomationProperties.SetName(button, text); inputs.Add(button); return button;
        }
        var header = new Grid(); header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var title = Label(design.T("runtime_config.title"), 17, "text"); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.VerticalAlignment = VerticalAlignment.Center; header.Children.Add(title);
        var close = design.ToolButton(design.T("hosts.cancel"), "close"); close.Width = close.Height = 28;
        close.MinWidth = close.MinHeight = 0; close.Padding = new(0); inputs.Add(close);
        close.Click += (_, _) => { if (editor.CanSave) CloseRuntimeInterpreter(); };
        Grid.SetColumn(close, 1); header.Children.Add(close); body.Children.Add(header);
        body.Children.Add(Label(design.Localization.Format("runtime_config.scope", ("context", editor.ContextLabel))));
        TextBox Field(bool python)
        {
            var group = new StackPanel { Spacing = 5 };
            var label = Label(design.T(python ? "runtime_config.python" : "runtime_config.r")); group.Children.Add(label);
            var input = new TextBox { Text = python ? editor.Values.Python : editor.Values.Rscript,
                PlaceholderText = design.T(python ? "runtime_config.python_placeholder" : "runtime_config.r_placeholder"),
                MinWidth = 0, MinHeight = 0, Padding = new(11, 9, 11, 9), CornerRadius = new(8), BorderThickness = new(1),
                Background = design.Brush("bg-input"), Foreground = design.Brush("text"), BorderBrush = design.Brush("border-strong"),
                IsSpellCheckEnabled = false, IsTextPredictionEnabled = false };
            design.BindTypography(input, code: true, size: 12.5); AutomationProperties.SetLabeledBy(input, label); menus.Register(input);
            inputs.Add(input); input.TextChanged += (_, _) => editor.SetPath(python, input.Text);
            var frame = NativeDialogVisuals.Input(input, design);
            if (editor.Context.Kind == "local" && pickAttachment != null)
            {
                var row = new Grid { ColumnSpacing = 8 }; row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.Children.Add(frame);
                var browse = Button(design.T("runtime_config.browse")); browse.Padding = new(14, 8, 14, 8); browse.VerticalAlignment = VerticalAlignment.Center;
                browse.Click += async (_, _) =>
                {
                    await editor.BrowseAsync(python, pickAttachment);
                    if (ReferenceEquals(runtimeInterpreter, editor)) input.Text = python ? editor.Values.Python : editor.Values.Rscript;
                };
                Grid.SetColumn(browse, 1); row.Children.Add(browse); group.Children.Add(row);
            }
            else group.Children.Add(frame);
            body.Children.Add(group); return input;
        }
        var python = Field(true); Field(false);
        body.Children.Add(Label(design.T("runtime_config.hint"), 11.5, "text-faint"));
        var error = Label("", 12, "err"); error.Visibility = Visibility.Collapsed;
        AutomationProperties.SetLiveSetting(error, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive); body.Children.Add(error);
        var actions = new NativeActionWrap { Gap = 10, RightAligned = true, Margin = new(0, 4, 0, 0) };
        var cancel = Button(design.T("hosts.cancel")); cancel.Click += (_, _) => { if (editor.CanSave) CloseRuntimeInterpreter(); }; actions.Children.Add(cancel);
        var save = Button(design.T("settings.save"), true); actions.Children.Add(save); body.Children.Add(actions);
        save.Click += async (_, _) =>
        {
            if (await editor.SaveAsync() && ReferenceEquals(runtimeInterpreter, editor))
            { CloseRuntimeInterpreter(); await model.Runtimes.RefreshAsync(lifetime.Token); }
        };
        runtimeInterpreterChanged = () =>
        {
            if (!ReferenceEquals(runtimeInterpreter, editor)) return;
            foreach (var input in inputs) input.IsEnabled = editor.CanSave;
            save.Opacity = editor.CanSave ? 1 : .5;
            error.Text = editor.Error ?? ""; error.Visibility = editor.Error == null ? Visibility.Collapsed : Visibility.Visible;
        };
        editor.Changed += runtimeInterpreterChanged;
        var scroll = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        var card = new Border { Child = scroll, CornerRadius = new(16), Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1),
            HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle,
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
        overlay.Children.Add(card);
        var root = runtimeInterpreterRoot = XamlRoot;
        void Resize()
        {
            overlay.Width = root.Size.Width; overlay.Height = root.Size.Height;
            card.Width = Math.Min(560, Math.Max(0, root.Size.Width - 40)); card.MaxHeight = Math.Max(0, root.Size.Height - 40);
            body.Width = Math.Max(0, card.Width - 2);
        }
        runtimeInterpreterResize = (_, _) => Resize(); root.Changed += runtimeInterpreterResize; Resize();
        runtimeInterpreterPopup = new Popup { XamlRoot = root, Child = overlay, IsLightDismissEnabled = false };
        overlay.Loaded += (_, _) => DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low,
            () => { if (ReferenceEquals(runtimeInterpreter, editor)) python.Focus(FocusState.Programmatic); });
        runtimeInterpreterPopup.IsOpen = true;
    }
}
