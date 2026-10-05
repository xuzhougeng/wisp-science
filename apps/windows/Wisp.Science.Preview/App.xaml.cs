using Microsoft.UI.Xaml;

namespace Wisp.Science.Preview;

public partial class App : Application
{
    private readonly List<MainWindow> windows = [];
    public App() => InitializeComponent();
    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        OpenWindow();
    }

    internal void OpenWindow(string? database = null, string? project = null, string? session = null)
    {
        var window = new MainWindow(database, project, session);
        windows.Add(window);
        window.Closed += (_, _) => windows.Remove(window);
        window.Activate();
    }
}
