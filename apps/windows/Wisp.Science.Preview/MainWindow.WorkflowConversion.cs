using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class MainWindow
{
    private readonly Dictionary<string, NativeWorkflowConversionModel> workflowConversions = [];
    private NativeWorkflowConversionNotice? conversionNotice;
    private NativeWorkflowConversionModel GetWorkflowConversion(INativeSettingsClient _, string? projectId)
    {
        var key = model.DatabasePath + "\n" + projectId;
        if (workflowConversions.TryGetValue(key, out var existing)) return existing;
        var conversion = new NativeWorkflowConversionModel(projectId ?? ""); workflowConversions[key] = conversion;
        conversion.Changed += () => DispatcherQueue.TryEnqueue(RefreshWorkflowConversionNotice);
        return conversion;
    }
    private void RefreshWorkflowConversionNotice()
    {
        if (windowClosed) return;
        var conversion = workflowConversions.Where(p => p.Key.StartsWith(model.DatabasePath + "\n", StringComparison.Ordinal)).Select(p => p.Value)
            .Where(c => c.RequestId != null && !c.DialogOpen).OrderByDescending(c => c.StartedAt).FirstOrDefault();
        if (conversionNotice != null) { root.Children.Remove(conversionNotice); conversionNotice = null; }
        if (conversion == null) return;
        conversionNotice = new NativeWorkflowConversionNotice(design, conversion,
            model.Projects.FirstOrDefault(p => p.Id == conversion.ProjectId)?.Name ?? conversion.ProjectId, () =>
        {
            if (settingsPage == null) OpenSettingsSection("workflow-conversion", conversion.ProjectId);
            else if (settingsPage.ScopeProject == conversion.ProjectId) settingsPage.ShowWorkflowConversion();
        }, settingsPage == null || settingsPage.ScopeProject == conversion.ProjectId);
        root.Children.Add(conversionNotice);
    }
}
