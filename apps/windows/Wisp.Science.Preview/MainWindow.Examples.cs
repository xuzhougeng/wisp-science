using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private NativeExamplePage? examplePage;
    private bool openingExampleCopy;
    private FrameworkElement ExampleProjectCard()
    {
        var content = Stack(0); var title = Row(8);
        var name = Text(design.T("projects.example"), 14); name.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.Children.Add(name);
        var tag = Text(design.T("projects.example_tag"), 11, "text-muted"); tag.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.Children.Add(new Border { Child = tag, Background = design.Brush("bg-sunken"), Padding = new(7, 1, 7, 1), CornerRadius = new(8), VerticalAlignment = VerticalAlignment.Center });
        content.Children.Add(title); content.Children.Add(Text(design.Localization.Format("projects.sessions_n", ("n", NativeExampleCatalog.Entries.Count.ToString())), 13, "text-faint"));
        var open = ContentButton(content, OpenExampleProject, "project-example", design.T("projects.example")); open.Padding = new(18, 16, 18, 16);
        var card = Card(open); card.Padding = new(0); return card;
    }
    private void OpenExampleProject()
    {
        if (windowClosed || examplePage != null) return;
        model.GoHome();
        examplePage = new(design, CloseExampleProject, info => _ = CopyExampleAsync(info)); Render();
    }
    private void CloseExampleProject()
    {
        examplePage?.Dispose(); examplePage = null; Render();
    }
    private async Task CopyExampleAsync(NativeExampleInfo info)
    {
        var owner = examplePage; if (owner == null || openingExampleCopy || workspaceSheet != null) return;
        openingExampleCopy = true;
        NativeSettingsClient? host;
        try { host = await ConnectHostAsync(); } finally { openingExampleCopy = false; }
        if (host == null || windowClosed || examplePage != owner) return;
        var copy = new NativeExampleCopy(host, info, model.Projects);
        MountSheet(new NativeExampleCopyPage(copy, design, (project, session) =>
        {
            if (windowClosed || examplePage != owner) return;
            CloseSheet(); CloseExampleProject(); _ = model.OpenProjectAsync(project, session);
        }, CloseSheet));
    }
}
