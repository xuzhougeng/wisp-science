using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeSettingsAuthTerminal : UserControl, IDisposable
{
    private readonly NativeAuthTerminalModel model;
    private readonly CancellationTokenSource lifetime = new();
    private readonly TextBox output = new() { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MaxHeight = 300,
        FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Consolas") };
    private readonly PasswordBox input = new() { Header = "输入后发送（不保存在设置中）" };
    private readonly TextBlock status = new() { TextWrapping = TextWrapping.Wrap };
    private readonly Button send = new() { Content = "发送" }, interrupt = new() { Content = "中断" }, close = new() { Content = "关闭授权终端" }, resume = new() { Content = "已检查，恢复输入" };
    private bool disposed;
    public NativeSettingsAuthTerminal(NativeAuthTerminalModel model, WispDesign design, Action closed)
    {
        this.model = model;
        design.BindTypography(this); design.BindTypography(output, true); design.BindTypography(input, true);
        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(design.Text("授权终端", 18));
        body.Children.Add(new TextBlock { Text = "离开页面仅停止显示；使用关闭按钮结束授权终端。", TextWrapping = TextWrapping.Wrap });
        body.Children.Add(output); body.Children.Add(status); body.Children.Add(input);
        var buttons = new StackPanel { Spacing = 8 }; buttons.Children.Add(send); buttons.Children.Add(interrupt); buttons.Children.Add(resume); buttons.Children.Add(close); body.Children.Add(buttons);
        async Task Send()
        {
            if (!model.CanWrite) return;
            var value = input.Password; input.Password = "";
            await model.WriteAsync(value + "\r");
        }
        send.Click += async (_, _) => await Send();
        input.KeyDown += async (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) { e.Handled = true; await Send(); } };
        interrupt.Click += async (_, _) => await model.WriteAsync("\u0003");
        resume.Click += (_, _) => model.AcknowledgeInput();
        close.Click += async (_, _) => { if (await model.CloseAsync() && !disposed) closed(); };
        model.Changed += Render; Content = body; design.ApplyTypography(this); Render(); _ = Poll();
    }
    private async Task Poll()
    {
        try { while (!disposed && await model.ReadAsync() && model.Running) await Task.Delay(750, lifetime.Token); }
        catch (OperationCanceledException) { }
    }
    private void Render()
    {
        if (disposed) return;
        output.Text = model.Output; status.Text = model.Error ?? (model.Running ? "授权进程运行中" : "授权进程已结束或正在读取状态");
        input.IsEnabled = send.IsEnabled = interrupt.IsEnabled = model.CanWrite;
        close.IsEnabled = !model.Busy; resume.Visibility = model.InputUncertain ? Visibility.Visible : Visibility.Collapsed;
    }
    public void Dispose()
    {
        if (disposed) return;
        disposed = true; input.Password = ""; model.Changed -= Render; lifetime.Cancel(); model.Dispose(); lifetime.Dispose();
    }
}
