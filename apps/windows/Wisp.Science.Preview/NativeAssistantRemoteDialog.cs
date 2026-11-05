using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Media.Imaging;
using Windows.Storage.Streams;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeAssistantRemoteDialog : UserControl, IDisposable
{
    private readonly NativeAssistantRemoteModel model;
    private readonly WispDesign design;
    private readonly DispatcherTimer timer = new() { Interval = TimeSpan.FromSeconds(1) };
    private readonly Border dialog;
    private readonly TextBlock state, detail, error, expired, bindText;
    private readonly ToggleSwitch enabled = new() { MinWidth = 0, OnContent = null, OffContent = null, VerticalAlignment = VerticalAlignment.Center };
    private readonly Button bind, unbind, refresh, closeButton;
    private readonly NativeActionWrap actions = new() { Gap = 8, Margin = new Thickness(0, 18, 0, 18) };
    private readonly StackPanel qr = new() { Spacing = 13, Margin = new Thickness(0, 0, 0, 13), Visibility = Visibility.Collapsed };
    private string? renderedImage;
    private bool rendering, loaded, disposed, ticking;
    private int ticks;

    public NativeAssistantRemoteDialog(NativeAssistantRemoteModel model, WispDesign design, Action close)
    {
        this.model = model; this.design = design; design.BindTypography(this);
        HorizontalContentAlignment = HorizontalAlignment.Stretch; VerticalContentAlignment = VerticalAlignment.Stretch;
        var overlay = new Grid(); NativeDialogVisuals.Backdrop(overlay, design);
        var dismiss = new Border { Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent) };
        dismiss.PointerPressed += (_, _) => close(); overlay.Children.Add(dismiss);
        // The visible topmost dialog participates in NativeAssistantPage's window-level Escape stack.
        var form = new StackPanel();
        var header = new Grid { ColumnSpacing = 12 };
        header.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var heading = new StackPanel { Spacing = 12 };
        var title = Text(L("远程接入", "Remote access"), 18, false); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        heading.Children.Add(title); heading.Children.Add(Text(L("科研助理 · 微信", "Research assistant · WeChat"))); header.Children.Add(heading);
        closeButton = design.ToolButton(L("关闭远程接入", "Close remote access"), "close"); closeButton.VerticalAlignment = VerticalAlignment.Top;
        closeButton.Click += (_, _) => close(); Grid.SetColumn(closeButton, 1); header.Children.Add(closeButton); form.Children.Add(header);
        closeButton.Loaded += (_, _) => { if (!disposed) closeButton.Focus(FocusState.Programmatic); };
        var intro = Text(L("在微信里直接与科研助理对话，延续桌面的同一条长期对话。用自然语言说明项目和需求，即可统筹所有可见项目。", "Talk to your research assistant from WeChat. The same ongoing conversation manages all visible projects — describe the project and task in your own words."));
        intro.Margin = new Thickness(0, 26, 0, 0); form.Children.Add(intro);
        var channel = new Grid { ColumnSpacing = 16, Padding = new Thickness(0, 16, 0, 16) };
        channel.ColumnDefinitions.Add(new() { Width = new(1, GridUnitType.Star) }); channel.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var channelCopy = new StackPanel { Spacing = 4 }; var name = Text(design.T("channels.weixin.title"), 14, false); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        channelCopy.Children.Add(name); state = Text(""); channelCopy.Children.Add(state); channel.Children.Add(channelCopy);
        PreferenceToggleSwitch.Apply(enabled, design); AutomationProperties.SetName(enabled, L("启用科研助理微信接入", "Enable assistant WeChat"));
        enabled.Resources["ToggleSwitchFillOffDisabled"] = design.Brush("border-strong").Color;
        enabled.Resources["ToggleSwitchStrokeOffDisabled"] = design.Brush("border-strong").Color;
        enabled.Resources["ToggleSwitchKnobFillOffDisabled"] = design.Brush("on-clay");
        AutomationProperties.SetAutomationId(enabled, "assistant-weixin-enabled"); Grid.SetColumn(enabled, 1); channel.Children.Add(enabled);
        enabled.Toggled += async (_, _) => { if (!rendering) await model.SetEnabledAsync(enabled.IsOn); };
        form.Children.Add(new Border { Child = channel, BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 1), Margin = new Thickness(0, 20, 0, 0) });
        detail = Text(""); detail.Margin = new Thickness(0, 13, 0, 0); form.Children.Add(detail);
        bind = ActionButton("", model.StartBindingAsync, true); bindText = (TextBlock)bind.Content;
        unbind = ActionButton(L("解除绑定", "Unbind"), model.UnbindAsync);
        refresh = ActionButton(L("刷新状态", "Refresh status"), () => model.RefreshAsync());
        actions.Children.Add(bind); actions.Children.Add(refresh); form.Children.Add(actions); form.Children.Add(qr);
        expired = Text(L("二维码已过期，请重新扫码绑定。", "QR code expired. Scan again to retry.")); expired.Margin = new Thickness(0, 0, 0, 13); form.Children.Add(expired);
        error = Text(""); error.Foreground = design.Brush("err"); error.Margin = new Thickness(0, 0, 0, 13); AutomationProperties.SetLiveSetting(error, Microsoft.UI.Xaml.Automation.Peers.AutomationLiveSetting.Assertive); form.Children.Add(error);
        form.Children.Add(new Border { BorderBrush = design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(0, 16, 0, 0),
            Child = Text(L("此处的绑定和开关独立于设置页的远程接入。仅接收扫码所有者的一对一消息。请保持 Wisp 运行；需要审批时，请在桌面助理或对应项目中处理。", "This binding and switch are separate from Settings → Remote Access. Only the scanning owner's direct messages are accepted. Keep Wisp running; handle approvals in the desktop assistant or project.")) });
        var scroll = new ScrollViewer { Content = form, Padding = new Thickness(24), HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalScrollMode = ScrollMode.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        dialog = new Border { Child = scroll, MaxWidth = 520, HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center,
            Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(14), Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
        AutomationProperties.SetName(dialog, L("远程接入", "Remote access")); AutomationProperties.SetAutomationId(dialog, "assistant-remote");
        overlay.Children.Add(dialog); Content = overlay; TabFocusNavigation = KeyboardNavigationMode.Cycle;
        SizeChanged += (_, _) => Resize(); model.Changed += Render;
        timer.Tick += async (_, _) =>
        {
            if (ticking || disposed) return; ticking = true;
            try { if (model.QrImage != null) await model.PollAsync(); else if (++ticks >= 3) { ticks = 0; await model.RefreshAsync(false); } }
            finally { ticking = false; }
        };
        Loaded += async (_, _) => { if (loaded) return; loaded = true; Resize(); closeButton.Focus(FocusState.Programmatic); timer.Start(); await model.RefreshAsync(); };
        Render();
    }
    private void Resize() { dialog.Width = Math.Max(0, Math.Min(520, ActualWidth - 40)); dialog.MaxHeight = Math.Max(0, ActualHeight - 40); }
    private void Render()
    {
        if (disposed) return; rendering = true;
        var status = model.Status;
        state.Text = status == null ? L("正在读取…", "Loading…") : design.T(!status.Bound ? "channels.weixin.not_bound" : "channels.state." + (status.State is "running" or "connecting" or "error" ? status.State : "stopped"));
        detail.Text = status?.Detail ?? ""; detail.Visibility = detail.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        enabled.IsOn = status?.Enabled == true; enabled.IsEnabled = model.CanChange; enabled.Opacity = enabled.IsEnabled ? 1 : .5;
        bindText.Text = status?.Bound == true ? L("重新扫码绑定", "Bind again") : L("扫码绑定", "Scan to bind"); bind.IsEnabled = model.CanBind;
        if (status?.Bound == true && !actions.Children.Contains(unbind)) actions.Children.Insert(1, unbind);
        else if (status?.Bound != true) actions.Children.Remove(unbind);
        unbind.IsEnabled = model.CanChange;
        refresh.IsEnabled = !model.Busy; error.Text = model.Error ?? ""; error.Visibility = model.Error != null ? Visibility.Visible : Visibility.Collapsed;
        foreach (var button in new[] { bind, unbind, refresh }) button.Opacity = button.IsEnabled ? 1 : .5;
        expired.Visibility = model.Expired ? Visibility.Visible : Visibility.Collapsed;
        if (renderedImage != model.QrImage)
        {
            renderedImage = model.QrImage; qr.Children.Clear(); qr.Visibility = renderedImage == null ? Visibility.Collapsed : Visibility.Visible;
            if (renderedImage != null) _ = RenderQr(renderedImage);
        }
        rendering = false;
    }
    private async Task RenderQr(string data)
    {
        try
        {
            using var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream)) { writer.WriteBytes(Convert.FromBase64String(data["data:image/svg+xml;base64,".Length..])); await writer.StoreAsync(); writer.DetachStream(); }
            stream.Seek(0); var source = new SvgImageSource(); var result = await source.SetSourceAsync(stream);
            if (disposed || renderedImage != data) return;
            if (result != SvgImageSourceLoadStatus.Success) throw new InvalidDataException(L("无法呈现绑定二维码。", "Unable to display the binding QR code."));
            var image = new Image { Source = source, Width = 220, Height = 220 }; AutomationProperties.SetName(image, L("微信绑定二维码", "WeChat binding QR code"));
            qr.Children.Add(new Border { Child = image, Background = new SolidColorBrush(Microsoft.UI.Colors.White), CornerRadius = new CornerRadius(8), HorizontalAlignment = HorizontalAlignment.Center });
            var hint = Text(L("请用所有者的微信扫码，并在手机上确认。", "Scan with the owner's WeChat account and confirm on your phone.")); hint.TextAlignment = TextAlignment.Center; qr.Children.Add(hint);
        }
        catch (Exception ex) { if (!disposed && renderedImage == data) model.ImageFailed(ex.Message); }
    }
    private Button ActionButton(string text, Func<Task> action, bool primary = false)
    {
        var button = new Button { Content = text }; design.ActionButton(button, primary);
        button.Padding = new Thickness(12, 8, 12, 8); button.MinHeight = 34; button.CornerRadius = new CornerRadius(7);
        if (primary) ((TextBlock)button.Content).Foreground = design.Brush("on-clay");
        button.Resources["ButtonBackgroundDisabled"] = design.Brush(primary ? "clay" : "bg-elev");
        button.Resources["ButtonBorderBrushDisabled"] = design.Brush(primary ? "clay" : "border");
        button.Resources["ButtonForegroundDisabled"] = design.Brush(primary ? "on-clay" : "text");
        button.Click += async (_, _) => await action(); return button;
    }
    private TextBlock Text(string value, double size = 13, bool muted = true)
    {
        var text = design.Text(value, size); text.Foreground = design.Brush(muted ? "text-muted" : "text");
        var leading = size == 13 ? 1.7 : 1.2;
        text.LineHeight = design.FontSize(size) * leading; text.LineStackingStrategy = LineStackingStrategy.BlockLineHeight;
        text.RegisterPropertyChangedCallback(TextBlock.FontSizeProperty, (sender, _) =>
        { var label = (TextBlock)sender; label.LineHeight = label.FontSize * leading; });
        return text;
    }
    private string L(string zh, string en) => design.L(zh, en);
    public void Dispose() { if (disposed) return; disposed = true; timer.Stop(); model.Changed -= Render; model.Dispose(); qr.Children.Clear(); }
}
