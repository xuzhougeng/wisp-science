using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
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
    private PasswordBox? deviceTokenField;
    private JsonNode? syncResult;
    private void ClearDeviceToken() { if (deviceTokenField != null) deviceTokenField.Password = ""; deviceTokenField = null; }
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
    private void Channels()
    {
        var status = model.Values["channels_status"];
        if (status == null) { Results.Children.Add(Mute("尚未读取通道状态，请刷新。")); return; }
        if (channelDetail.Length > 0) Form.Children.Add(Button("返回远程接入", () => LeaveChannelsAsync(() => { channelDetail = ""; Render(); })));
        switch (channelDetail)
        {
            case "feishu": Feishu(status); break;
            case "weixin": Weixin(status); break;
            case "device": DeviceBridge(status["device"]); break;
            default:
                ProjectSync();
                var card = Card("消息与设备接入"); card.Children.Add(Mute("连接消息平台与设备，远程查看项目并发起对话。"));
                foreach (var (id, name, enabled) in new[] { ("feishu", "飞书 / Lark", B(status, "feishu_enabled")), ("weixin", "微信 / iLink", B(status, "weixin_enabled")), ("device", "StickS3 设备桥接", B(status["device"], "enabled")) })
                    card.Children.Add(Button(name + (enabled ? " · 已启用" : " · 未启用"), () => { channelDetail = id; Render(); return Task.CompletedTask; }));
                break;
        }
    }
    private void Feishu(JsonNode status)
    {
        var card = Card("飞书 / Lark"); card.Children.Add(Mute(S(status, "feishu_detail") is { Length: > 0 } detail ? detail : S(status, "feishu_state")));
        card.Children.Add(Button("配置", () =>
        {
            Editor("飞书 / Lark", new() { ["enabled"] = B(status, "feishu_enabled"), ["international"] = B(status, "feishu_international"), ["app_id"] = S(status, "feishu_app_id"), ["app_secret"] = "" }, "set_feishu_channel", null,
                d => { Boolean(d, "enabled", "启用"); Boolean(d, "international", "使用 Lark"); Text(d, "app_id", "App ID"); Secure(d, "app_secret", "App Secret（留空保留已有密钥）"); }); return Task.CompletedTask;
        }));
        card.Children.Add(Button("设置所有者", () =>
        { Editor("飞书所有者", new() { ["open_id"] = S(status, "feishu_owner_open_id") }, "set_feishu_owner", null, d => Text(d, "open_id", "所有者 Open ID")); return Task.CompletedTask; }));
        if (B(status, "feishu_bound")) Action(card, "解除飞书绑定", "feishu_unbind", new(), true);
        if (S(status, "feishu_pending_owner_open_id").Length > 0)
        {
            card.Children.Add(Mute("待确认所有者：" + S(status, "feishu_pending_owner_open_id")));
            Action(card, "确认所有者", "confirm_feishu_pending_owner", new(), true);
            Action(card, "拒绝所有者", "reject_feishu_pending_owner", new());
        }
        ChannelScan(card, "feishu", B(status, "feishu_international"));
    }
    private void Weixin(JsonNode status)
    {
        var card = Card("微信 / iLink"); card.Children.Add(Mute(S(status, "weixin_detail") is { Length: > 0 } detail ? detail : S(status, "weixin_state")));
        Toggle(card, "启用微信通道", B(status, "weixin_enabled"), "set_weixin_channel", new());
        if (B(status, "weixin_bound")) Action(card, "解除微信绑定", "weixin_unbind", new(), true);
        ChannelScan(card, "weixin", false);
    }
    private void ChannelScan(StackPanel card, string kind, bool international)
    {
        if (channelBinding == null) return;
        if (!channelBinding.Active) card.Children.Add(Button("扫码绑定", async () =>
        { if (await channelBinding.StartAsync(kind, international) && !model.Closed) Render(); }));
        card.Children.Add(Mute(channelBinding.Status));
        if (channelBinding.Kind != kind || channelBinding.Binding is not { } binding) return;
        var qr = new StackPanel(); card.Children.Add(qr); _ = RenderBindingQr(qr, binding);
        card.Children.Add(Button("检查扫码状态", async () =>
        {
            if (await channelBinding.PollAsync() && !model.Closed)
            { if (!channelBinding.Active) await Reload(); else Render(); }
        }));
        card.Children.Add(Button("取消绑定", CancelChannelBinding));
    }
    private async Task RenderBindingQr(StackPanel target, JsonObject binding)
    {
        try
        {
            const string prefix = "data:image/svg+xml;base64,";
            var data = S(binding, "qr_image");
            if (!data.StartsWith(prefix, StringComparison.Ordinal) || data.Length > 2000000) throw new InvalidDataException("绑定服务未返回有效二维码图像。");
            var bytes = Convert.FromBase64String(data[prefix.Length..]);
            using var stream = new InMemoryRandomAccessStream();
            using (var writer = new DataWriter(stream)) { writer.WriteBytes(bytes); await writer.StoreAsync(); writer.DetachStream(); }
            stream.Seek(0); var source = new SvgImageSource();
            var loaded = await source.SetSourceAsync(stream);
            if (model.Closed || !ReferenceEquals(channelBinding?.Binding, binding)) return;
            if (loaded != SvgImageSourceLoadStatus.Success) throw new InvalidDataException("无法呈现绑定二维码。");
            var image = new Image { Source = source, Width = 220, Height = 220, HorizontalAlignment = HorizontalAlignment.Left };
            Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(image, "绑定二维码"); target.Children.Add(image);
        }
        catch (Exception ex) { if (!model.Closed && ReferenceEquals(channelBinding?.Binding, binding)) target.Children.Add(Warn(ex.Message)); }
    }
    private void DeviceBridge(JsonNode? device)
    {
        var card = Card("设备桥接"); card.Children.Add(Mute(S(device, "detail"))); card.Children.Add(Mute(S(device, "url")));
        card.Children.Add(Button("配置", () =>
        {
            Editor("设备桥接", new() { ["enabled"] = B(device, "enabled"), ["mode"] = S(device, "mode"), ["bind_ipv4"] = S(device, "bindIpv4"), ["port"] = device?["port"]?.DeepClone() }, "set_device_bridge", null,
                d => { Boolean(d, "enabled", "启用"); Choice(Form, "模式", S(d, "mode"), [("lan", "局域网 / 本机")], value => d["mode"] = value); Text(d, "bind_ipv4", "监听 IPv4"); Integer(d, "port", "端口", 1, 65535); }); return Task.CompletedTask;
        }));
        var token = new PasswordBox { Header = "访问令牌", Visibility = Visibility.Collapsed }; deviceTokenField = token;
        card.Children.Add(Button("显示访问令牌", async () =>
        { if (await model.InvokeAsync("get_device_bridge_token", new(), value => token.Password = value?.GetValue<string>() ?? "")) token.Visibility = Visibility.Visible; }));
        card.Children.Add(token);
        card.Children.Add(Button("复制令牌", () =>
        { if (token.Password.Length > 0) { var data = new DataPackage(); data.SetText(token.Password); Clipboard.SetContent(data); } return Task.CompletedTask; }));
        card.Children.Add(Button("隐藏令牌", () => { token.Password = ""; token.Visibility = Visibility.Collapsed; return Task.CompletedTask; }));
        Action(card, "重置设备访问令牌", "rotate_device_bridge_token", new(), true); Action(card, "撤销设备访问令牌", "revoke_device_bridge_token", new(), true);
    }
    private void ProjectSync()
    {
        Preference("项目同步配置", "get_settings", "set_settings", "settings", d =>
        {
            Choice(Form, "同步方式", S(d, "sync_backend"), [("relay", "中继服务"), ("folder", "同步文件夹")], value => d["sync_backend"] = value);
            Text(d, "sync_relay_url", "中继 URL"); Secure(d, "sync_relay_token", "中继令牌（留空保留已有令牌）"); PathField(d, "sync_folder", "同步文件夹", directory: true);
        });
        var card = Card("同步操作");
        async Task Sync(string command, JsonObject args)
        { if (await model.InvokeAsync(command, args, value => syncResult = value?.DeepClone())) Render(); }
        if (projectScoped)
        {
            card.Children.Add(Button("立即同步", () => Sync("sync_project", new() { ["id"] = settingsProjectId })));
            card.Children.Add(Button("生成加入代码", () => Sync("project_sync_code", new() { ["id"] = settingsProjectId })));
        }
        else card.Children.Add(Mute("选择项目作用域后可同步或生成加入代码。"));
        card.Children.Add(Button("加入同步项目", () =>
        { Editor("加入同步项目", new() { ["code"] = "" }, "join_synced_project", null, d => Text(d, "code", "加入代码", true)); return Task.CompletedTask; }));
        if (syncResult is JsonValue code && code.TryGetValue<string>(out var text))
            card.Children.Add(new TextBox { Header = "同步结果 / 加入代码", Text = text, IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap });
        else if (syncResult != null) Summary(card, syncResult);
        if (projectScoped && syncResult is JsonObject && S(syncResult, "status") == "conflict")
            foreach (var (strategy, label) in new[] { ("local", "保留本地版本"), ("remote", "采用远端版本") })
                card.Children.Add(Button(label, () => { Confirm(label + "解决同步冲突？", () => _ = Sync("resolve_project_sync", new() { ["id"] = settingsProjectId, ["strategy"] = strategy })); return Task.CompletedTask; }));
    }
}
