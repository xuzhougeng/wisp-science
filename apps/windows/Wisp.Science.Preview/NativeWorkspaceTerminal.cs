using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeWorkspaceTerminal : UserControl, IDisposable
{
    private readonly WorkspaceTerminalModel model;
    private readonly WispDesign design;
    private readonly Action hide;
    private readonly TextBox output = new() { IsReadOnly = true, AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Consolas") };
    private readonly TextBox input = new() { PlaceholderText = "输入后按 Enter 发送。完整 VT 渲染仍是后续工作。" };
    private readonly CancellationTokenSource lifetime = new();
    private bool disposed;

    public NativeWorkspaceTerminal(WorkspaceTerminalModel model, WispDesign design, Action hide)
    {
        this.model = model; this.design = design; this.hide = hide;
        design.BindTypography(this); design.BindTypography(output, true); design.BindTypography(input, true);
        Height = 260;
        var root = new Grid { Padding = new Thickness(10), Background = design.Brush("bg-sunken") };
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        root.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        root.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var toolbar = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        var refresh = new Button { Content = "刷新" };
        refresh.Click += async (_, _) => { await model.LoadAsync(lifetime.Token); Render(); };
        var open = new Button { Content = "新建本地终端" };
        open.Click += async (_, _) => { await model.OpenAsync("local", lifetime.Token); Render(); };
        var interrupt = new Button { Content = "中断" };
        interrupt.Click += async (_, _) => await model.WriteAsync([3], lifetime.Token);
        var close = new Button { Content = "关闭终端" };
        close.Click += async (_, _) => { await model.CloseSelectedAsync(lifetime.Token); Render(); };
        var dismiss = new Button { Content = "隐藏" };
        dismiss.Click += (_, _) => hide();
        toolbar.Children.Add(refresh); toolbar.Children.Add(open); toolbar.Children.Add(interrupt); toolbar.Children.Add(close); toolbar.Children.Add(dismiss);
        root.Children.Add(toolbar);
        Grid.SetRow(output, 1); root.Children.Add(output);
        input.KeyDown += async (_, e) =>
        {
            if (e.Key != Windows.System.VirtualKey.Enter) return;
            e.Handled = true;
            var text = input.Text;
            input.Text = "";
            await model.WriteAsync(System.Text.Encoding.UTF8.GetBytes(text + "\n"), lifetime.Token);
            Render();
        };
        Grid.SetRow(input, 2); root.Children.Add(input);
        Content = root;
        design.ApplyTypography(this);
        _ = LoopAsync();
    }

    public Task OpenContextAsync(string context) => model.OpenAsync(context, lifetime.Token);

    private async Task LoopAsync()
    {
        try
        {
            await model.LoadAsync(lifetime.Token);
            if (model.Terminals.Length == 0) await model.OpenAsync("local", lifetime.Token);
            Render();
            while (!lifetime.IsCancellationRequested)
            {
                await model.ReadAsync(lifetime.Token);
                Render();
                await Task.Delay(200, lifetime.Token);
            }
        }
        catch (OperationCanceledException) { }
    }

    private void Render()
    {
        if (disposed) return;
        output.Text = model.Output;
        input.IsEnabled = model.SelectedId != null && !model.InputUncertain && model.ExitCode is null;
        if (model.Error is { } error && !output.Text.Contains(error)) output.Text += "\n" + error;
    }

    public void Dispose()
    {
        if (disposed) return;
        disposed = true; lifetime.Cancel(); model.Detach(); lifetime.Dispose();
    }
}
