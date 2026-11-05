using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Windows.Foundation;
using Wisp.ProjectBrowser;
using Windows.ApplicationModel.DataTransfer;

namespace Wisp.Science.Preview;

/// <summary>Shared selection actions for editable and highlighted source; no file writes.</summary>
internal sealed class NativeSourceSelectionActions : IDisposable
{
    private readonly FrameworkElement editor;
    private readonly TextBox? editable;
    private readonly Func<NativeDocumentSelection?> capture;
    private readonly Func<string> text;
    private readonly Action clear;
    private readonly WispDesign design;
    private readonly Func<bool> current;
    private readonly Func<NativeDocumentSelection, bool> quote;
    private readonly FlyoutBase context;
    private readonly Func<string>? copyText;
    private readonly MenuFlyoutItem? copyItem;
    private string copySelection = "";
    private readonly Popup popup = new();
    private readonly StackPanel content = new() { Spacing = 4 };
    private readonly TextBlock error;
    private readonly List<Control> actions = [];
    private readonly List<AppBarButton> contextActions = [];
    private readonly PointerEventHandler released, outside, wheel;
    private readonly KeyboardAccelerator keyboardMenu;
    private UIElement? inputRoot;
    private XamlRoot? xamlRoot;
    private NativeDocumentSelection? selection;
    private string? capturedText;
    private Point anchor;
    private bool menuOpen, disposed;

    public NativeSourceSelectionActions(TextBox editor, string original, WispDesign design,
        Func<bool> current, Func<NativeDocumentSelection, bool> quote)
        : this(editor, design, current, quote,
            () => NativeSourceSelection.Capture(original, editor.Text, editor.SelectionStart, editor.SelectionLength)?.Selection,
            () => editor.Text, () => editor.Select(editor.SelectionStart, 0))
    {
        editable = editor; editor.SelectionFlyout = null; editor.TextChanged += TextChanged;
    }

    public NativeSourceSelectionActions(TextBlock body, string source, WispDesign design,
        Func<bool> current, Func<NativeDocumentSelection, bool> quote)
        : this(body, design, current, quote,
            () => NativeSourceSelection.CaptureHighlighted(source,
                body.Inlines.OfType<Microsoft.UI.Xaml.Documents.Run>()
                    .Select(run => new NativeSourceRun(run.ContentStart.Offset, run.ContentEnd.Offset, run.Text)).ToArray(),
                body.SelectionStart.Offset, body.SelectionEnd.Offset, body.SelectedText)?.Selection,
            () => source, () => body.Select(body.ContentStart, body.ContentStart))
    { body.SelectionFlyout = null; }

    public static NativeSourceSelectionActions ForPreview(FrameworkElement target, Func<string> selected, Action clear,
        string displayed, WispDesign design, Func<bool> current, Func<NativeDocumentSelection, bool> quote)
    {
        if (target is RichTextBlock rich) rich.SelectionFlyout = null;
        else if (target is TextBlock text) text.SelectionFlyout = null;
        return new(target, design, current, quote, () =>
        {
            var value = NativeDocumentSelection.FromPreview(selected());
            return value != null && NativeArtifactSelection.ContainsPreviewText(displayed, value.Text) ? value : null;
        }, () => displayed, clear, selected, () =>
        {
            if (target is RichTextBlock block) block.Select(block.ContentStart, block.ContentEnd);
            else if (target is TextBlock text) text.Select(text.ContentStart, text.ContentEnd);
        });
    }

    private NativeSourceSelectionActions(FrameworkElement editor, WispDesign design,
        Func<bool> current, Func<NativeDocumentSelection, bool> quote,
        Func<NativeDocumentSelection?> capture, Func<string> text, Action clear, Func<string>? copyText = null, Action? selectAll = null)
    {
        this.editor = editor; this.design = design; this.current = current; this.quote = quote;
        this.capture = capture; this.text = text; this.clear = clear;
        this.copyText = copyText;
        context = copyText == null ? new TextCommandBarFlyout() : new MenuFlyout();
        if (context is MenuFlyout previewMenu)
        {
            // Native RichTextBlock Copy omits InlineUIContainer code chips.
            // Use the same visible-text projection as quoting for both Copy
            // and Ctrl+C, while leaving editable source platform commands intact.
            copyItem = new MenuFlyoutItem { Text = NativeTranscriptLabels.Actions(design.Localization).Copy,
                Icon = new ImageIcon { Source = design.Icon("copy", 16).Source } };
            copyItem.Click += (_, _) => CopyCaptured(); previewMenu.Items.Add(copyItem);
            var all = new MenuFlyoutItem { Text = design.L("全选", "Select all") };
            all.Click += (_, _) => { if (Current()) selectAll?.Invoke(); }; previewMenu.Items.Add(all);
            previewMenu.Items.Add(new MenuFlyoutSeparator());
        }
        var row = new NativeActionWrap { Gap = 2, RowGap = 2 };
        content.Children.Add(row);
        error = design.Text("", 12); error.Foreground = design.Brush("clay-strong"); error.Visibility = Visibility.Collapsed;
        content.Children.Add(error);
        popup.Child = new Border { Child = content, Padding = new(4), CornerRadius = new(10),
            Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new(1) };
        foreach (var (key, icon, jump) in new[] { ("selection.add_to_chat", "plus", false), ("selection.add_to_chat_and_jump", "chat", true) })
        {
            var label = design.T(key);
            var button = design.ToolButton(label, icon, true);
            button.Click += (_, _) => Apply(jump); row.Children.Add(button); actions.Add(button);
            // Keep the platform's Copy/Select all and editable Cut/Paste/Undo
            // commands instead of replacing the editor's standard context menu.
            if (context is MenuFlyout preview)
            {
                var command = new MenuFlyoutItem { Text = label, Icon = new ImageIcon { Source = design.Icon(icon, 16).Source } };
                command.Click += (_, _) => Apply(jump); preview.Items.Add(command); actions.Add(command);
            }
            else
            {
                var command = new AppBarButton { Label = label, Icon = new ImageIcon { Source = design.Icon(icon, 16).Source } };
                AutomationProperties.SetName(command, label);
                command.Click += (_, _) => Apply(jump); contextActions.Add(command); actions.Add(command);
            }
        }
        context.Opening += (_, _) => { popup.IsOpen = false; Capture(); menuOpen = true; };
        // TextCommandBarFlyout rebuilds its editing commands during opening.
        // Append only after that rebuild, while preserving the platform commands.
        context.Opened += (_, _) =>
        {
            if (context is TextCommandBarFlyout native)
                foreach (var command in contextActions)
                    if (!native.SecondaryCommands.Contains(command)) native.SecondaryCommands.Add(command);
        };
        context.Closed += (_, _) => menuOpen = false;
        editor.ContextFlyout = context;
        // F10 is a system key and does not reliably reach TextBox's preview
        // key event. Scope its accelerator to this editor, not the whole app.
        keyboardMenu = new() { Key = Windows.System.VirtualKey.F10,
            Modifiers = Windows.System.VirtualKeyModifiers.Shift, ScopeOwner = editor };
        keyboardMenu.Invoked += (_, e) =>
        {
            if (!Current()) return;
            e.Handled = true; if (!menuOpen) context.ShowAt(editor);
        };
        editor.KeyboardAccelerators.Add(keyboardMenu);
        released = (_, e) =>
        {
            if (e.GetCurrentPoint(editor).Properties.PointerUpdateKind != Microsoft.UI.Input.PointerUpdateKind.LeftButtonReleased) return;
            anchor = e.GetCurrentPoint((UIElement)editor.XamlRoot.Content).Position;
            editor.DispatcherQueue.TryEnqueue(() =>
            {
                if (!Current() || !design.SelectionPopupEnabled) return;
                Capture(); if (selection == null) return;
                context.Hide(); popup.XamlRoot = editor.XamlRoot;
                var root = (FrameworkElement)editor.XamlRoot.Content;
                ((FrameworkElement)popup.Child).MaxWidth = Math.Max(160, Math.Min(570, root.ActualWidth - 24));
                popup.Child.Measure(new Size(((FrameworkElement)popup.Child).MaxWidth, double.PositiveInfinity));
                Position(); popup.IsOpen = true;
            });
        };
        outside = (_, _) => popup.IsOpen = false;
        wheel = (_, _) => popup.IsOpen = false;
        editor.AddHandler(UIElement.PointerReleasedEvent, released, true);
        popup.Opened += (_, _) =>
        {
            inputRoot = (UIElement)editor.XamlRoot.Content; xamlRoot = editor.XamlRoot;
            inputRoot.AddHandler(UIElement.PointerPressedEvent, outside, true);
            inputRoot.AddHandler(UIElement.PointerWheelChangedEvent, wheel, true);
            xamlRoot.Changed += RootChanged;
        };
        popup.Closed += (_, _) => UnhookRoot();
        ((FrameworkElement)popup.Child).SizeChanged += (_, _) => { if (popup.IsOpen && editor.IsLoaded) Position(); };
        editor.Unloaded += Unloaded; editor.PreviewKeyDown += KeyDown;
        design.SelectionPopupChanged += PreferenceChanged;
    }
    private bool Current() => !disposed && editor.IsLoaded && current();
    private void Capture()
    {
        selection = Current() ? capture() : null;
        capturedText = Current() ? text() : null;
        copySelection = Current() ? copyText?.Invoke() ?? "" : "";
        if (copyItem != null) copyItem.IsEnabled = copySelection.Length > 0;
        foreach (var action in actions) action.IsEnabled = selection != null;
        error.Visibility = Visibility.Collapsed;
    }
    private bool CopyCaptured()
    {
        if (!Current() || capturedText != text() || copySelection.Length == 0) return false;
        try { var data = new DataPackage(); data.SetText(copySelection); Clipboard.SetContent(data); Close(); return true; }
        catch { return false; }
    }
    private void Apply(bool jump)
    {
        if (!Current() || selection is not { } selected || capturedText != text()) { Close(); return; }
        var value = selected with { Jump = jump };
        if (!quote(value))
        {
            error.Text = design.L("未能加入聊天，请检查当前会话后重试。", "Could not add the quote. Check the current conversation and try again.");
            error.Visibility = Visibility.Visible;
            // A context command closes its menu. Keep retry feedback visible
            // without discarding the selected text or the source editor.
            if (!popup.IsOpen) { popup.XamlRoot = editor.XamlRoot; anchor = editor.TransformToVisual((UIElement)editor.XamlRoot.Content).TransformPoint(new Point(12, 40)); Position(); popup.IsOpen = true; }
            return;
        }
        Close();
        if (editor.IsLoaded) clear();
    }
    private void Position()
    {
        var root = (FrameworkElement)editor.XamlRoot.Content;
        var child = (FrameworkElement)popup.Child;
        child.MaxWidth = Math.Max(160, Math.Min(570, root.ActualWidth - 24));
        child.Measure(new Size(child.MaxWidth, double.PositiveInfinity));
        var width = child.ActualWidth > 0 ? child.ActualWidth : child.DesiredSize.Width;
        var height = child.ActualHeight > 0 ? child.ActualHeight : child.DesiredSize.Height;
        popup.HorizontalOffset = Math.Clamp(anchor.X - width / 2, 12, Math.Max(12, root.ActualWidth - width - 12));
        popup.VerticalOffset = Math.Clamp(anchor.Y - height - 20 >= 12 ? anchor.Y - height - 20 : anchor.Y + 20,
            12, Math.Max(12, root.ActualHeight - height - 12));
    }
    private void RootChanged(XamlRoot _, XamlRootChangedEventArgs __) => popup.IsOpen = false;
    private void KeyDown(object _, KeyRoutedEventArgs e)
    {
        if (!Current()) return;
        // RichTextBlock consumes Ctrl+C before a KeyboardAccelerator runs.
        // Handle it on the editor's preview route so inline-code containers
        // are copied with their visible text instead of disappearing.
        if (copyText != null && e.Key == Windows.System.VirtualKey.C
            && (Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Control)
                & Windows.UI.Core.CoreVirtualKeyStates.Down) != 0
            && (Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(Windows.System.VirtualKey.Menu)
                & Windows.UI.Core.CoreVirtualKeyStates.Down) == 0)
        {
            Capture(); e.Handled = CopyCaptured(); return;
        }
        if (e.Key != Windows.System.VirtualKey.Application) return;
        e.Handled = true;
        if (!menuOpen) context.ShowAt(editor);
    }
    private void PreferenceChanged(bool enabled) { if (!enabled) popup.IsOpen = false; }
    private void TextChanged(object _, TextChangedEventArgs __) { Close(); selection = null; }
    private void Unloaded(object _, RoutedEventArgs __) => Close();
    private void Close() { context.Hide(); popup.IsOpen = false; }
    private void UnhookRoot()
    {
        inputRoot?.RemoveHandler(UIElement.PointerPressedEvent, outside);
        inputRoot?.RemoveHandler(UIElement.PointerWheelChangedEvent, wheel);
        if (xamlRoot != null) xamlRoot.Changed -= RootChanged;
        inputRoot = null; xamlRoot = null;
    }
    public bool HandleEscape() { if (!menuOpen && !popup.IsOpen) return false; Close(); return true; }
    public bool IsOpen => menuOpen || popup.IsOpen;
    public void DismissPopup() => popup.IsOpen = false;
    public void Dispose()
    {
        if (disposed) return; disposed = true; Close(); UnhookRoot();
        design.SelectionPopupChanged -= PreferenceChanged;
        editor.RemoveHandler(UIElement.PointerReleasedEvent, released);
        editor.KeyboardAccelerators.Remove(keyboardMenu);
        editor.Unloaded -= Unloaded; editor.PreviewKeyDown -= KeyDown;
        if (editable != null) editable.TextChanged -= TextChanged;
        if (editor.ContextFlyout == context) editor.ContextFlyout = null;
    }
}
