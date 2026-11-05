using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

/// The same retained conversion is shown here and in the studio dialog.
internal sealed class NativeWorkflowConversionNotice : UserControl
{
    public NativeWorkflowConversionNotice(WispDesign design, NativeWorkflowConversionModel state, string projectName, Action open, bool canOpen)
    {
        var frame = new Border { Background = design.Brush("bg-elev"), BorderBrush = design.Brush("border-strong"), BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(8), Padding = new Thickness(16, 14, 16, 14) }; MaxWidth = 520;
        HorizontalAlignment = HorizontalAlignment.Right; VerticalAlignment = VerticalAlignment.Top; Margin = new Thickness(20, 64, 20, 20);
        var row = new Grid { ColumnSpacing = 12 };
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var icon = design.Icon("branch", 22, design.ColorText("clay")); icon.VerticalAlignment = VerticalAlignment.Center; row.Children.Add(icon);
        var content = new StackPanel { Spacing = 4, VerticalAlignment = VerticalAlignment.Center };
        var status = design.Text(design.T(state.StatusKey), 13); status.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        AutomationProperties.SetLiveSetting(status, AutomationLiveSetting.Polite); content.Children.Add(status);
        var project = design.Text(projectName, 12); project.Foreground = design.Brush("text-muted"); content.Children.Add(project);
        var elapsed = design.Text("", 12); elapsed.Foreground = design.Brush("text-muted"); content.Children.Add(elapsed); Grid.SetColumn(content, 1); row.Children.Add(content);
        var label = design.T(state.Busy ? "workflow_studio.conversion.view_progress" : "workflow_studio.conversion.view_result");
        var view = new Button { Content = label, IsEnabled = canOpen, VerticalAlignment = VerticalAlignment.Center }; design.ActionButton(view);
        AutomationProperties.SetName(view, label); view.Click += (_, _) => open();
        if (!canOpen) ToolTipService.SetToolTip(view, design.L("请返回主界面后查看原项目的转换。", "Return to the app to view the source project's conversion."));
        Grid.SetColumn(view, 2); row.Children.Add(view); frame.Child = row; Content = frame;
        void Tick() => elapsed.Text = design.Localization.Format("workflow_studio.conversion.elapsed", ("time", $"{(int)state.Elapsed.TotalMinutes}:{state.Elapsed.Seconds:00}"));
        var timer = new DispatcherTimer { Interval = TimeSpan.FromSeconds(1) }; timer.Tick += (_, _) => Tick();
        Loaded += (_, _) => { Tick(); if (state.Busy) timer.Start(); }; Unloaded += (_, _) => timer.Stop(); Tick();
    }
}
