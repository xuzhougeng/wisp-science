using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;

namespace Wisp.Science.Preview;

/// <summary>A destination stays visible while its service starts or fails.</summary>
internal sealed class NativeConnectionPage : WorkspaceSheet
{
    private readonly Action retry;
    public NativeConnectionPage(WispDesign design, string title, Action close, Action retry)
        : base(design, title, close)
    {
        this.retry = retry;
        Body.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        Body.Children.Add(design.Text("正在连接桌面服务…可以随时返回。"));
    }
    public void Failed(string message)
    {
        Body.Children.Clear();
        Body.Children.Add(Design.Text(message));
        var button = new Button { Content = "重新连接" };
        button.Click += (_, _) => retry(); Body.Children.Add(button);
    }
}
