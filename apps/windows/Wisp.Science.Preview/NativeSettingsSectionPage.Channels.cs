using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media.Imaging;
using Wisp.ProjectBrowser;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage.Streams;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private NativeChannelBindingModel? channelBinding;
    private string channelDetail = "";
    private string? channelOwnerDraft;
    private string channelNotice = "";
    private readonly List<PasswordBox> channelSecrets = [];
    private Border ChannelBadge(string state, string? label = null)
    {
        var text = Design.Text(label ?? NativeChannelSettings.StateLabel(state, Design.Localization), 11);
        text.Foreground = Design.Brush(NativeChannelSettings.StateColor(state));
        text.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        Design.BindLineHeight(text, 11, 1.35);
        return new Border { Child = text, Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(99),
            Padding = new Thickness(6, 2, 6, 2), VerticalAlignment = VerticalAlignment.Center };
    }
    private Button ChannelButton(string label, Func<Task> action, bool primary = false)
    {
        // Choose the final style once; the generic helper infers a primary
        // style from Chinese Save before this page has selected its style.
        var button = ModelButton(label, action); StyleModelButton(button, 13.5, new Thickness(16, 9, 16, 9), primary);
        if (primary && button.Content is TextBlock text) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        return button;
    }
    private bool ChannelHasChanges => section == "channels" && channelDetail == "feishu" && channelOwnerDraft != null
        && channelOwnerDraft != S(model.Values["channels_status"], "feishu_owner_open_id");
    private void ClearChannelSecrets()
    {
        // Disconnected password controls must not retain a draft or a displayed token.
        // Their change handlers ignore clearing after the owning draft is replaced.
        foreach (var input in channelSecrets) { input.Tag = null; input.Password = ""; }
        channelSecrets.Clear();
    }
    private void ChannelChanged()
    {
        if (channelBinding?.Error is { } error && !model.Closed) model.Fail(error);
        LockConfirmation();
    }
    private async Task LeaveChannelsAsync(Action leave)
    {
        if (channelBinding?.Active == true && !await channelBinding.CancelAsync()) return;
        if (!model.Closed) leave();
    }
    private async Task CancelChannelBinding()
    { if (channelBinding != null && await channelBinding.CancelAsync() && !model.Closed) Render(); }
    private void NavigateChannel(string id)
    {
        RequestLeave(() =>
        {
            model.Discard(); invalidFields.Clear(); channelOwnerDraft = null; channelNotice = ""; channelDetail = id;
            labInviteCode = ""; labKnowledgeProject = "";
            Render();
            // Resolve the new content extent before resetting: WinUI otherwise
            // anchors the newly mounted list to the old detail's focused input.
            BodyScroll.UpdateLayout(); BodyScroll.ChangeView(null, 0, null, true);
            if (id == "lab") _ = RefreshLab();
        }, switchingCategory: true);
    }
    private bool HandleChannelEscape()
    {
        if (section != "channels") return false;
        if (channelBinding?.Active == true) { _ = CancelChannelBinding(); return true; }
        if (channelDetail.Length > 0) { NavigateChannel(""); return true; }
        RequestLeave(() => base.HandleEscape(), closingSettings: true); return true;
    }
    private async Task ChannelAction(string command, JsonObject args, bool refreshOwner = false)
    {
        if (!await model.InvokeAsync(command, args)) { if (!model.Closed) Render(); return; }
        if (model.Closed) return;
        if (await model.InvokeAsync("channels_status", new(), value => model.Values["channels_status"] = value?.DeepClone()))
        {
            if (refreshOwner) channelOwnerDraft = S(model.Values["channels_status"], "feishu_owner_open_id");
            channelNotice = L("操作已完成。", "Done.");
            Render();
        }
    }
    private void Channels()
    {
        Form.Spacing = 14;
        Body.Padding = new Thickness(24, 24, 24, 32);
        var status = model.Values["channels_status"];
        if (status == null) { Form.Children.Add(ModelHint(L("尚未读取通道状态，请刷新。", "Channel status is unavailable. Refresh to try again."))); return; }
        HeaderActions.Children.Clear();
        if (channelNotice.Length > 0) Form.Children.Add(ModelHint(channelNotice));
        if (channelDetail.Length > 0)
        {
            var title = channelDetail switch { "feishu" => T("channels.feishu.title"), "weixin" => T("channels.weixin.title"), "device" => T("channels.device.title"), "remote" => T("channels.remote.title"), "lab" => T("channels.lab.title"), _ => T("projects.sync.join") };
            SettingsBreadcrumb(T("settings.nav.channels"), title, () => NavigateChannel(""));
        }
        switch (channelDetail)
        {
            case "feishu": Feishu(status); break;
            case "weixin": Weixin(status); break;
            case "device": DeviceBridge(status["device"]); break;
            case "remote": RemoteAccess(status["remote"]); break;
            case "lab": Lab(); break;
            default:
                ProjectSync();
                var syncCard = (FrameworkElement)Form.Children[Form.Children.Count - 1];
                var card = InlineCard(T("channels.messages_devices"));
                Design.BindTypography((TextBlock)card.Children[0], 15);
                var messagesCard = (FrameworkElement)Form.Children[Form.Children.Count - 1];
                card.Children.Add(ModelHint(T("channels.desc")));
                var list = new StackPanel();
                card.Children.Add(new Border { Child = list, Margin = new Thickness(0, 16, 0, 20), BorderThickness = new Thickness(0, 1, 0, 0), BorderBrush = Design.Brush("border") });
                foreach (var id in new[] { "feishu", "weixin", "device", "remote", "lab" })
                    ChannelRow(list, status, id, id is "feishu" or "weixin" ? B(status, id + "_enabled") : B(status[id], "enabled"));
                var routing = new StackPanel { Spacing = 4 };
                routing.Children.Add(ModelHint(T("channels.routing.desc")));
                var commands = new NativeActionWrap { Gap = 6, Margin = new Thickness(6, 0, 0, 0) };
                foreach (var command in new[] { "/status", "/project", "/session", "/new" })
                {
                    var code = Design.Text(command, 11); code.FontFamily = Design.Font(true);
                    commands.Children.Add(new Border { Child = code, Padding = new Thickness(7, 1, 7, 1), CornerRadius = new CornerRadius(99),
                        Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1) });
                }
                routing.Children.Add(commands);
                card.Children.Add(new Border { Child = routing, Background = Design.Brush("bg-sunken"), CornerRadius = new CornerRadius(8), Padding = new Thickness(14) });
                ArrangeChannelOverview(syncCard, messagesCard);
                break;
        }
    }
    private void ArrangeChannelOverview(FrameworkElement sync, FrameworkElement messages)
    {
        Form.Children.Remove(sync); Form.Children.Remove(messages);
        var grid = new Grid { ColumnSpacing = 20, RowSpacing = 20 };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1.1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        grid.RowDefinitions.Add(new() { Height = GridLength.Auto }); grid.RowDefinitions.Add(new() { Height = GridLength.Auto });
        sync.VerticalAlignment = messages.VerticalAlignment = VerticalAlignment.Top;
        grid.Children.Add(sync); grid.Children.Add(messages); Form.Children.Add(grid);
        void Resize()
        {
            var columns = NativeSettingsLayout.RemoteColumns(XamlRoot?.Size.Width ?? ActualWidth);
            Grid.SetColumnSpan(sync, columns ? 1 : 2); Grid.SetColumnSpan(messages, columns ? 1 : 2);
            Grid.SetColumn(messages, columns ? 1 : 0); Grid.SetRow(messages, columns ? 0 : 1);
        }
        grid.Loaded += (_, _) => Resize(); grid.SizeChanged += (_, _) => Resize();
    }
    private void ChannelRow(StackPanel list, JsonNode status, string id, bool enabled)
    {
        var title = T($"channels.{id}.title");
        var (state, label, hint) = NativeChannelSettings.Overview(status, id, Design.Localization);
        var row = new Grid { ColumnSpacing = 12, Padding = new Thickness(0, 16, 0, 16) };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var copy = new StackPanel { Spacing = 2 }; var name = new NativeActionWrap { Gap = 6, VerticallyCentered = true };
        var heading = Design.Text(title, 14); heading.FontWeight = Microsoft.UI.Text.FontWeights.Medium; Design.BindLineHeight(heading, 14, 1.35); name.Children.Add(heading);
        if (label.Length > 0) name.Children.Add(ChannelBadge(state, label)); copy.Children.Add(name);
        if (hint.Length > 0)
        {
            var description = ModelHint(hint); Design.BindLineHeight(description, 12, 1.4); description.TextWrapping = TextWrapping.NoWrap; description.TextTrimming = TextTrimming.CharacterEllipsis;
            ToolTipService.SetToolTip(description, hint); copy.Children.Add(description);
        }
        var open = new Button { Content = copy, Padding = new Thickness(0), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch }; Design.QuietButton(open); AutomationProperties.SetName(open, L("打开", "Open ") + title); open.Click += (_, _) => NavigateChannel(id); row.Children.Add(open);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Center };
        open.Padding = new Thickness(0); open.MinHeight = 0;
        var toggle = new ToggleSwitch { IsOn = enabled, IsEnabled = NativeChannelSettings.CanEnable(status, id), OnContent = null, OffContent = null, MinWidth = 0 };
        if (id == "remote") toggle.IsEnabled &= ChannelSupports("set_remote_access");
        AutomationProperties.SetName(toggle, L("启用", "Enable ") + title); StylePreferenceToggle(toggle);
        // These two WinUI animation resources are Colors; the knob uses a Brush.
        toggle.Resources["ToggleSwitchFillOffDisabled"] = Design.Brush("border-strong").Color;
        toggle.Resources["ToggleSwitchStrokeOffDisabled"] = Design.Brush("border-strong").Color;
        toggle.Resources["ToggleSwitchKnobFillOffDisabled"] = Design.Brush("on-clay");
        toggle.Opacity = toggle.IsEnabled ? 1 : .42;
        toggle.Toggled += async (_, _) =>
        {
            var args = id == "feishu" ? NativeChannelSettings.FeishuDraft(status) : id == "device" ? NativeChannelSettings.DeviceDraft(status["device"]) : id == "remote" ? NativeChannelSettings.RemoteDraft(status["remote"]) : new JsonObject();
            args["enabled"] = toggle.IsOn;
            var wire = new JsonObject(); foreach (var pair in args) wire[pair.Key switch { "app_id" => "appId", "app_secret" => "appSecret", "bind_ipv4" => "bindIpv4", "relay_url" => "relayUrl", "relay_token" => "relayToken", _ => pair.Key }] = pair.Value?.DeepClone();
            await ChannelAction(id switch { "feishu" => "set_feishu_channel", "weixin" => "set_weixin_channel", "remote" => "set_remote_access", _ => "set_device_bridge" }, wire);
            if (!model.Closed) Render();
        };
        if (id != "lab") actions.Children.Add(toggle); var next = Design.ToolButton(L("查看", "View ") + title, "chevron-right"); next.Padding = new Thickness(0); next.MinWidth = 12; next.Width = 12; next.Click += (_, _) => NavigateChannel(id); actions.Children.Add(next); Grid.SetColumn(actions, 1); row.Children.Add(actions);
        list.Children.Add(new Border { Child = row, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, id == "lab" ? 0 : 1) });
    }
    private StackPanel ChannelCopy(string title, string hint)
    {
        var copy = new StackPanel { Spacing = 4 }; var name = Design.Text(title, 13); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; copy.Children.Add(name);
        var description = ModelHint(hint); Design.BindLineHeight(description, 12, 1.5);
        description.MaxWidth = 720; description.HorizontalAlignment = HorizontalAlignment.Left; copy.Children.Add(description); return copy;
    }
    private CheckBox ChannelCheck(string label, bool value, Action<bool>? changed = null)
    {
        var check = ModelCheck(label, value, changed ?? (_ => { }));
        if (check.Content is Border { Child: TextBlock text }) Design.BindTypography(text, 12);
        return check;
    }
    private Grid ChannelColumns(FrameworkElement left, FrameworkElement right, bool equal = false, bool formGrid = false)
    {
        var row = new Grid { ColumnSpacing = 14 };
        row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = equal ? new GridLength(1, GridUnitType.Star) : GridLength.Auto });
        row.RowDefinitions.Add(new() { Height = GridLength.Auto }); row.RowDefinitions.Add(new() { Height = GridLength.Auto });
        // Bind rows and footer notes align to the action's center. Preserve an
        // explicit bottom alignment used by a folder-picker action.
        if (!equal && left.VerticalAlignment == VerticalAlignment.Stretch) left.VerticalAlignment = VerticalAlignment.Center;
        if (!equal && right.VerticalAlignment == VerticalAlignment.Stretch) right.VerticalAlignment = VerticalAlignment.Center;
        row.Children.Add(left); Grid.SetColumn(right, 1); row.Children.Add(right);
        void Resize() { var narrow = NativeSettingsLayout.ChannelFieldsStack(XamlRoot?.Size.Width ?? ActualWidth, formGrid); row.RowSpacing = narrow ? 12 : 0; Grid.SetColumnSpan(left, narrow ? 2 : 1); Grid.SetColumnSpan(right, narrow ? 2 : 1); Grid.SetColumn(right, narrow ? 0 : 1); Grid.SetRow(right, narrow ? 1 : 0); }
        row.Loaded += (_, _) => Resize(); row.SizeChanged += (_, _) => Resize();
        return row;
    }
    private void ChannelDivider(StackPanel target, string text)
    {
        var grid = new Grid { ColumnSpacing = 12, Margin = new Thickness(0, 14, 0, 2) };
        grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); grid.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); grid.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        grid.Children.Add(new Border { Height = 1, Background = Design.Brush("border"), VerticalAlignment = VerticalAlignment.Center });
        var label = Design.Text(text, 11); label.Foreground = Design.Brush("text-faint"); Grid.SetColumn(label, 1); grid.Children.Add(label);
        var line = new Border { Height = 1, Background = Design.Brush("border"), VerticalAlignment = VerticalAlignment.Center }; Grid.SetColumn(line, 2); grid.Children.Add(line); target.Children.Add(grid);
    }
    private StackPanel ChannelField(StackPanel target, Control input, string label)
    {
        var group = new StackPanel { Spacing = 5 };
        var caption = Design.Text(label, 12); caption.Foreground = Design.Brush("text-muted"); caption.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        group.Children.Add(caption); group.Children.Add(input); target.Children.Add(group);
        Design.BindTypography(input, size: 14); AutomationProperties.SetLabeledBy(input, caption);
        input.Padding = new Thickness(11, 9, input is ComboBox ? 0 : 11, 9);
        PreferenceFieldAppearance.Apply(input, Design); return group;
    }
    private TextBox ChannelInput(StackPanel target, string label, string value, Action<string> changed, string placeholder = "")
    {
        var input = new TextBox { Text = value, PlaceholderText = placeholder, MinHeight = 36, CornerRadius = new CornerRadius(8) };
        AutomationProperties.SetName(input, label); textMenus.Register(input);
        input.TextChanged += (_, _) => { changed(input.Text); LockConfirmation(); }; ChannelField(target, input, label); return input;
    }
    private StackPanel ChannelSecret(StackPanel target, JsonObject draft, string key, string label, string placeholder)
    {
        var input = new PasswordBox { MinHeight = 36, Password = S(draft, key), PlaceholderText = placeholder, Tag = draft, CornerRadius = new CornerRadius(8) };
        AutomationProperties.SetName(input, label);
        input.PasswordChanged += (_, _) =>
        {
            if (ReferenceEquals(input.Tag, draft) && (ReferenceEquals(model.Draft, draft) || ReferenceEquals(model.Draft?["get_settings"], draft)))
            { draft[key] = input.Password; LockConfirmation(); }
        };
        channelSecrets.Add(input); return ChannelField(target, input, label);
    }
    private void ChannelSaveActions(StackPanel target)
    {
        var actions = new NativeActionWrap { HorizontalAlignment = HorizontalAlignment.Right };
        var save = ChannelButton(T("settings.save"), async () => { if (await model.SaveAsync()) { channelNotice = T("channels.saved"); await Reload(); } }, primary: true); actions.Children.Add(save);
        foreach (var button in actions.Children.OfType<Button>()) button.MinWidth = 84;
        var content = ChannelColumns(ModelHint(T("channels.secret_note")), actions);
        target.Children.Add(new Border { Child = content, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(0, 16, 0, 0), Margin = new Thickness(0, 4, 0, 0) });
    }
    private void Feishu(JsonNode status)
    {
        if (model.Draft == null) model.Edit(NativeChannelSettings.FeishuDraft(status), "set_feishu_channel");
        var draft = model.Draft!; channelOwnerDraft ??= S(status, "feishu_owner_open_id"); var bound = B(status, "feishu_bound");
        var actions = new NativeActionWrap(); var create = ChannelButton(bound ? T("channels.feishu.recreate") : T("channels.feishu.bind"), () => StartChannelBinding("feishu", B(draft, "international")), primary: true); actions.Children.Add(create);
        if (channelBinding?.Active == true) actions.Children.Add(ChannelButton(L("取消绑定", "Cancel binding"), CancelChannelBinding));
        if (bound) actions.Children.Add(ChannelButton(T("channels.feishu.unbind"), () => { RequestLeave(() => Confirm(L("移除本地飞书绑定？飞书平台上的应用不会被删除。", "Remove the local Feishu binding? The app on Feishu will remain."), () => { model.Discard(); _ = ChannelAction("feishu_unbind", new(), true); })); return Task.CompletedTask; }));
        Form.Children.Add(ChannelColumns(ChannelCopy(bound ? T("channels.feishu.bound_app") : T("channels.feishu.scan_title"), bound ? S(status, "feishu_app_id") + " · " + (B(status, "feishu_international") ? T("channels.feishu.region_lark") : T("channels.feishu.region_cn")) : T("channels.feishu.scan_hint")), actions));
        ChannelScan(Form, "feishu"); ChannelDivider(Form, T("channels.feishu.manual"));
        // The reference groups these three rows in settings-form-grid (12px),
        // independently of the outer settings-pane's 14px spacing.
        var credentials = new StackPanel { Spacing = 12 }; Form.Children.Add(credentials);
        var international = ChannelCheck(T("channels.feishu.international"), B(draft, "international"), value => draft["international"] = value);
        international.IsEnabled = channelBinding?.Active != true;
        credentials.Children.Add(international);
        ChannelInput(credentials, "App ID", S(draft, "app_id"), value => draft["app_id"] = value, "cli_xxxxxxxx");
        ChannelSecret(credentials, draft, "app_secret", "App Secret · " + (B(status, "feishu_has_secret") ? L("已保存", "Stored") : T("channels.feishu.not_bound")), B(status, "feishu_has_secret") ? T("settings.key_stored") : "");
        ChannelDivider(Form, T("channels.feishu.owner_title"));
        var owner = S(status, "feishu_owner_open_id");
        if (S(status, "feishu_pending_owner_open_id") is { Length: > 0 } pending)
        {
            var pairing = ChannelCopy(T("channels.feishu.pending_title"), L("有飞书用户向机器人发了消息。请在本机确认后才会将其绑定为所有者。\n", "A Feishu user has messaged the bot. Confirm here before binding them as the owner.\n") + pending); var pairActions = new NativeActionWrap();
            pairActions.Children.Add(ChannelButton(T("channels.feishu.pending_confirm"), () => { Confirm(L("确认将此飞书用户绑定为所有者？", "Bind this Feishu user as the owner?"), () => _ = ChannelAction("confirm_feishu_pending_owner", new(), true)); return Task.CompletedTask; }));
            pairActions.Children.Add(ChannelButton(T("channels.feishu.pending_reject"), () => ChannelAction("reject_feishu_pending_owner", new(), true))); pairing.Children.Add(pairActions); Form.Children.Add(Design.Card(pairing));
        }
        var ownerInfo = ChannelCopy(owner.Length > 0 ? L("所有者：", "Owner: ") + owner : T("channels.feishu.owner_not_bound"), T("channels.feishu.owner_hint"));
        if (owner.Length > 0) ownerInfo.Children.Add(ChannelButton(T("channels.feishu.owner_clear"), () => { Confirm(L("清除已绑定的飞书所有者？", "Clear the bound Feishu owner?"), () => _ = ChannelAction("set_feishu_owner", new() { ["openId"] = "" }, true)); return Task.CompletedTask; })); Form.Children.Add(ownerInfo);
        ChannelInput(Form, T("channels.feishu.owner_open_id"), channelOwnerDraft, value => channelOwnerDraft = value, "ou_xxxxxxxx");
        var bindOwner = ChannelButton(T("channels.feishu.owner_save"), () => ChannelAction("set_feishu_owner", new() { ["openId"] = channelOwnerDraft?.Trim() ?? "" }, true));
        bindOwner.Margin = new Thickness(0, 4, 0, 0); Form.Children.Add(bindOwner);
        Form.Children.Add(ModelHint(T("channels.feishu.hint")));
        ChannelSaveActions(Form);
    }
    private async Task StartChannelBinding(string kind, bool international = false)
    {
        if (channelBinding == null || channelBinding.Active) return;
        RequestLeave(() => _ = Start());
        async Task Start()
        {
            model.Discard(); channelOwnerDraft = null;
            await channelBinding.StartAsync(kind, international);
            if (!model.Closed) Render();
        }
        await Task.CompletedTask;
    }
    private void Weixin(JsonNode status)
    {
        var bound = B(status, "weixin_bound"); var actions = new NativeActionWrap();
        if (bound) actions.Children.Add(ChannelButton(T("channels.weixin.unbind"), () => { Confirm(L("解除本机微信绑定？", "Remove the local WeChat binding?"), () => _ = ChannelAction("weixin_unbind", new())); return Task.CompletedTask; }));
        else { var bind = ChannelButton(T("channels.weixin.bind"), () => StartChannelBinding("weixin"), primary: true); actions.Children.Add(bind); }
        if (channelBinding?.Active == true) actions.Children.Add(ChannelButton(L("取消绑定", "Cancel binding"), CancelChannelBinding));
        Form.Children.Add(ChannelColumns(ChannelCopy(bound ? T("channels.weixin.bound_account") : T("channels.weixin.scan_title"), bound ? S(status, "weixin_detail") : T("channels.weixin.hint")), actions));
        ChannelScan(Form, "weixin");
    }
    private void ChannelScan(StackPanel target, string kind)
    {
        if (channelBinding == null) return;
        if (channelBinding.Status.Length > 0) target.Children.Add(ModelHint(channelBinding.Status));
        if (channelBinding.Kind != kind || channelBinding.Binding is not { } binding) return;
        var qr = new StackPanel(); var copy = ChannelCopy(kind == "feishu" ? T("channels.feishu.qr_title") : T("channels.weixin.qr_title"), kind == "feishu" ? T("channels.feishu.qr_hint") : T("channels.weixin.qr_hint"));
        copy.Children.Add(ChannelButton(L("检查扫码状态", "Check status"), async () =>
        {
            if (await channelBinding.PollAsync() && !model.Closed)
            {
                if (!channelBinding.Active) { model.Discard(); channelOwnerDraft = null; await Reload(); }
                else Render();
            }
        }));
        target.Children.Add(ChannelColumns(qr, copy, true)); _ = RenderBindingQr(qr, binding);
    }
    private async Task RenderBindingQr(StackPanel target, JsonObject binding)
    {
        try
        {
            const string prefix = "data:image/svg+xml;base64,"; var data = S(binding, "qr_image");
            if (!data.StartsWith(prefix, StringComparison.Ordinal) || data.Length > 2000000) throw new InvalidDataException(L("绑定服务未返回有效二维码图像。", "The binding service did not return a valid QR image."));
            using var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream)) { writer.WriteBytes(Convert.FromBase64String(data[prefix.Length..])); await writer.StoreAsync(); writer.DetachStream(); }
            stream.Seek(0); var source = new SvgImageSource(); var loaded = await source.SetSourceAsync(stream);
            if (model.Closed || !ReferenceEquals(channelBinding?.Binding, binding)) return;
            if (loaded != SvgImageSourceLoadStatus.Success) throw new InvalidDataException(L("无法呈现绑定二维码。", "Unable to display the binding QR code."));
            var image = new Image { Source = source, Width = 154, Height = 154 }; AutomationProperties.SetName(image, L("绑定二维码", "Binding QR code"));
            target.Children.Add(new Border { Child = image, Padding = new Thickness(8), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Background = new Microsoft.UI.Xaml.Media.SolidColorBrush(Microsoft.UI.Colors.White), HorizontalAlignment = HorizontalAlignment.Left });
        }
        catch (Exception ex) { if (!model.Closed && ReferenceEquals(channelBinding?.Binding, binding)) target.Children.Add(Warn(ex.Message)); }
    }
    private void DeviceBridge(JsonNode? device)
    {
        if (model.Draft == null) model.Edit(NativeChannelSettings.DeviceDraft(device), "set_device_bridge"); var draft = model.Draft!;
        Form.Children.Add(ChannelColumns(ChannelCopy(T("channels.device.heading"), T("channels.device.hint")), ChannelBadge(S(device, "state"))));
        Border Mode(string title, string hint, bool available)
        {
            var copy = new StackPanel { Spacing = 4 };
            var heading = Design.Text(title, 12.5); heading.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; copy.Children.Add(heading);
            var description = Design.Text(hint, 11.5); description.Foreground = Design.Brush("text-faint");
            Design.BindLineHeight(description, 11.5, 1.4); copy.Children.Add(description);
            var radio = new RadioButton { Content = copy, IsChecked = available, IsEnabled = available, GroupName = "device-mode",
                Padding = new Thickness(0), MinWidth = 0, HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch, VerticalAlignment = VerticalAlignment.Top };
            AutomationProperties.SetName(radio, title);
            radio.Loaded += (_, _) =>
            {
                // The reference label stacks its radio above the copy. Retain
                // the SDK's native selection, keyboard and automation states.
                radio.ApplyTemplate();
                if (Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChildrenCount(radio) == 0
                    || Microsoft.UI.Xaml.Media.VisualTreeHelper.GetChild(radio, 0) is not Grid root) return;
                var indicator = root.Children.OfType<Grid>().FirstOrDefault();
                var content = root.Children.OfType<ContentPresenter>().FirstOrDefault();
                if (indicator == null || content == null) return;
                root.ColumnDefinitions.Clear(); root.RowDefinitions.Clear();
                root.RowDefinitions.Add(new() { Height = GridLength.Auto }); root.RowDefinitions.Add(new() { Height = GridLength.Auto });
                indicator.Width = indicator.Height = 14; indicator.HorizontalAlignment = HorizontalAlignment.Left;
                indicator.Margin = new Thickness(4, 2, 0, 0);
                foreach (var ellipse in indicator.Children.OfType<Microsoft.UI.Xaml.Shapes.Ellipse>())
                    ellipse.Width = ellipse.Height = ellipse.Name == "CheckGlyph" ? 6 : 14;
                Grid.SetColumn(content, 0); Grid.SetRow(content, 1); content.Margin = new Thickness(0, 5, 0, 0);
            };
            return new Border { Child = radio, BorderBrush = Design.Brush(available ? "clay" : "border"), Background = Design.Brush("bg-input"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(8), Padding = new Thickness(12), Opacity = available ? 1 : .58 };
        }
        var modes = ChannelColumns(Mode(T("channels.device.mode_lan"), T("channels.device.mode_lan_hint"), true), Mode(L("中继服务器 · 规划中", "Relay server · Planned"), T("channels.device.mode_relay_hint"), false), true);
        modes.ColumnSpacing = 10; Form.Children.Add(modes);
        var deviceFields = new StackPanel { Spacing = 12 }; Form.Children.Add(deviceFields);
        var enabled = ChannelCheck(T("channels.device.toggle"), B(draft, "enabled")); enabled.Click += (_, _) => draft["enabled"] = enabled.IsChecked == true; deviceFields.Children.Add(enabled);
        var address = new StackPanel(); ChannelInput(address, T("channels.device.bind_ipv4"), S(draft, "bind_ipv4"), value => draft["bind_ipv4"] = value, "10.10.87.103");
        var port = new StackPanel(); ChannelInput(port, T("channels.device.port"), draft["port"]?.ToString() ?? "18766", value => { if (int.TryParse(value, out var number)) draft["port"] = number; else draft["port"] = value; }); deviceFields.Children.Add(ChannelColumns(address, port, true, formGrid: true));
        if (S(device, "url").Length > 0) Form.Children.Add(ModelHint(L("监听地址：", "Listening at: ") + S(device, "url")));
        if (S(device, "detail").Length > 0) Form.Children.Add(Warn(S(device, "detail")));
        Form.Children.Add(ModelHint(T("channels.device.bind_hint")));
        var stored = B(device, "hasToken"); var tokens = new NativeActionWrap { Gap = 10 };
        tokens.Children.Add(ChannelButton(stored ? T("channels.device.rotate_token") : T("channels.device.generate_token"), () => { Confirm(stored ? L("生成新令牌？旧令牌将立即失效。", "Generate a new token? The previous token will stop working immediately.") : L("生成设备访问令牌？", "Generate a device access token?"), () => _ = ChannelAction("rotate_device_bridge_token", new())); return Task.CompletedTask; }));
        var copy = ChannelButton(T("channels.device.copy_token"), () => model.InvokeAsync("get_device_bridge_token", new(), value => { if (value is JsonValue token && token.TryGetValue<string>(out var text) && text.Length > 0) { var data = new DataPackage(); data.SetText(text); Clipboard.SetContent(data); } })); copy.IsEnabled = stored; tokens.Children.Add(copy);
        var revoke = ChannelButton(T("channels.device.revoke_token"), () => { Confirm(L("撤销设备访问令牌？已有设备将失去访问权限。", "Revoke the device access token? Existing devices will lose access."), () => _ = ChannelAction("revoke_device_bridge_token", new())); return Task.CompletedTask; }); revoke.IsEnabled = stored; tokens.Children.Add(revoke);
        Form.Children.Add(new Border { Child = ChannelColumns(ChannelCopy(T("channels.device.token"), stored ? T("channels.device.token_stored") : T("channels.device.token_missing")), tokens),
            BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0), Padding = new Thickness(0, 14, 0, 0) });
        Form.Children.Add(new Border { Child = ModelHint(T("channels.device.security")), Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("warn"), BorderThickness = new Thickness(3, 0, 0, 0), Padding = new Thickness(12, 10, 12, 10) });
        ChannelSaveActions(Form);
    }
    private void ProjectSync()
    {
        if (model.Values["get_settings"] is not JsonObject prefs) { Form.Children.Add(ModelHint(L("尚未读取同步设置，请重新打开页面。", "Sync settings are unavailable. Reopen this page to try again."))); return; }
        model.EditWindowPreferences(WindowPreferences);
        var draft = model.Draft!["get_settings"]!.AsObject();
        var card = InlineCard(T("settings.sync.title")); card.Spacing = 14; ((FrameworkElement)card.Children[0]).Margin = new Thickness(0); Design.BindTypography((TextBlock)card.Children[0], 14);
        card.Children.Add(ModelHint(T("settings.sync.hint")));
        var backend = ConnectionChoice(T("settings.sync.backend")); backend.Items.Add(new ComboBoxItem { Content = T("settings.sync.relay"), Tag = "relay" }); backend.Items.Add(new ComboBoxItem { Content = T("settings.sync.folder"), Tag = "folder" });
        backend.SelectedIndex = S(draft, "sync_backend") == "folder" ? 1 : 0; choices.Add(backend); backend.Header = null; ChannelField(card, backend, T("settings.sync.backend"));
        var relay = new StackPanel { Spacing = 14 }; ChannelInput(relay, T("settings.sync.relay_url"), S(draft, "sync_relay_url"), value => draft["sync_relay_url"] = value, "https://sync.example.com");
        var tokenField = ChannelSecret(relay, draft, "sync_relay_token", T("settings.sync.relay_token"), B(prefs, "has_sync_relay_token") ? T("settings.key_stored") : T("settings.sync.token_placeholder")); tokenField.Children.Add(ModelHint(T("settings.sync.relay_hint"))); card.Children.Add(relay);
        var folder = new StackPanel { Spacing = 6 };
        var pathRow = new Grid { ColumnSpacing = 8 };
        pathRow.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        pathRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var pathLabel = T("settings.sync.folder_path");
        var caption = Design.Text(pathLabel, 12); caption.Foreground = Design.Brush("text-muted"); caption.FontWeight = Microsoft.UI.Text.FontWeights.Medium; folder.Children.Add(caption);
        var input = new TextBox { Text = S(draft, "sync_folder"), MinWidth = 0, MinHeight = 36, CornerRadius = new CornerRadius(8), Padding = new Thickness(11, 9, 11, 9) };
        AutomationProperties.SetLabeledBy(input, caption); AutomationProperties.SetName(input, pathLabel); textMenus.Register(input); PreferenceFieldAppearance.Apply(input, Design);
        Design.BindTypography(input, code: true, size: 12.5);
        input.TextChanged += (_, _) => { draft["sync_folder"] = input.Text; LockConfirmation(); };
        pathRow.Children.Add(input);
        if (pickPath != null)
        {
            var choose = ChannelButton(T("projects.choose_dir"), async () => { var path = await pickPath(true); if (!model.Closed && ReferenceEquals(model.Draft?["get_settings"], draft) && path != null) input.Text = path; });
            Grid.SetColumn(choose, 1); pathRow.Children.Add(choose);
        }
        folder.Children.Add(pathRow);
        folder.Children.Add(ModelHint(T("settings.sync.folder_hint"))); card.Children.Add(folder);
        void SelectBackend() { var isFolder = backend.SelectedIndex == 1; relay.Visibility = isFolder ? Visibility.Collapsed : Visibility.Visible; folder.Visibility = isFolder ? Visibility.Visible : Visibility.Collapsed; }
        SelectBackend(); backend.SelectionChanged += (_, _) => { draft["sync_backend"] = backend.SelectedIndex == 1 ? "folder" : "relay"; SelectBackend(); LockConfirmation(); };
        InlineSaveActions(); MovePreferenceFooter(card); card.Children.Add(ModelHint(T("settings.sync.join_hint")));
        var actions = new NativeActionWrap();
        var guide = Design.ToolButton(T("projects.sync.guide"), "doc", true); StyleModelButton(guide, 12, new Thickness(12, 8, 12, 8));
        guide.Click += async (_, _) => await Windows.System.Launcher.LaunchUriAsync(new Uri("https://github.com/xuzhougeng/wisp-science/blob/main/docs/" + (Design.Localization.Locale == "zh" ? "project-sync.zh-CN.md" : "project-sync.md")));
        actions.Children.Add(guide);
        var joinProject = Design.ToolButton(T("projects.sync.join"), "link", true); StyleModelButton(joinProject, 12, new Thickness(12, 8, 12, 8));
        joinProject.Click += (_, _) => { syncJoinOwner = joinProject; syncJoin?.Open(); }; actions.Children.Add(joinProject); card.Children.Add(actions);
    }
}
