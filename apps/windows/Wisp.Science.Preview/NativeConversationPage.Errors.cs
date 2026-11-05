using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private FrameworkElement ErrorCard(string message, NativeErrorTarget? target, bool modelView)
    {
        var body = new StackPanel();
        var head = new Grid { Padding = new(13, 11, 13, 9), ColumnSpacing = 9, RowSpacing = 9 };
        head.ColumnDefinitions.Add(new()); head.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        head.RowDefinitions.Add(new() { Height = GridLength.Auto }); head.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var summary = new Grid { ColumnSpacing = 9, VerticalAlignment = VerticalAlignment.Top };
        summary.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); summary.ColumnDefinitions.Add(new());
        var badge = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 5 };
        badge.Children.Add(new Ellipse { Width = 5, Height = 5, Fill = design.Brush("err"), VerticalAlignment = VerticalAlignment.Center });
        var label = design.Text(design.T("chat.error").ToUpperInvariant(), 10);
        label.FontWeight = Microsoft.UI.Text.FontWeights.Bold; label.CharacterSpacing = 40; label.Foreground = design.Brush("err");
        badge.Children.Add(label);
        var color = design.Brush("err").Color;
        summary.Children.Add(new Border { Child = badge, Padding = new(8, 2, 8, 2), CornerRadius = new(5),
            Background = new SolidColorBrush(Windows.UI.Color.FromArgb(31, color.R, color.G, color.B)), VerticalAlignment = VerticalAlignment.Top });
        var title = design.Text(message, 13); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.IsTextSelectionEnabled = true; Grid.SetColumn(title, 1); summary.Children.Add(title); head.Children.Add(summary);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6,
            HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top };
        Button Action(string key, string id)
        {
            var button = new Button { Content = design.T(key), Padding = new(10, 4, 10, 4), MinWidth = 0, MinHeight = 0,
                CornerRadius = new(5), BorderThickness = new(1), BorderBrush = design.Brush("border"),
                Background = design.Brush("bg-app"), Foreground = design.Brush("text-muted") };
            design.BindTypography(button, size: 11.5); AutomationProperties.SetName(button, design.T(key));
            AutomationProperties.SetAutomationId(button, id); actions.Children.Add(button); return button;
        }
        if (!modelView && model.ShowsErrorResume(target))
        {
            var resume = Action("chat.resume", "message-error-resume"); resume.IsEnabled = model.CanResumeError(target);
            resume.Click += async (_, _) =>
            {
                if (target == null || !model.CanResumeError(target)) return;
                followLatest = true; userScrollPending = false;
                await model.ResumeErrorAsync(target, lifetime.Token);
            };
        }
        var copy = Action("msg.copy", "message-error-copy"); ToolTipService.SetToolTip(copy, design.T("ctx.copy_message"));
        copy.Click += (_, _) => { try { var data = new DataPackage(); data.SetText(message); Clipboard.SetContent(data); } catch { } };
        Grid.SetColumn(actions, 1); head.Children.Add(actions);
        head.SizeChanged += (_, _) =>
        {
            actions.Measure(new Size(double.PositiveInfinity, double.PositiveInfinity));
            var stacked = head.ActualWidth - 26 < 320 + actions.DesiredSize.Width + 9;
            Grid.SetColumnSpan(summary, stacked ? 2 : 1);
            Grid.SetRow(actions, stacked ? 1 : 0); Grid.SetColumn(actions, stacked ? 0 : 1); Grid.SetColumnSpan(actions, stacked ? 2 : 1);
        };
        body.Children.Add(head);
        if (NativeErrorPresentation.HintKey(message) is { } hintKey)
        {
            var hint = design.Text(design.T(hintKey), 12.5); hint.Foreground = design.Brush("text-muted");
            hint.IsTextSelectionEnabled = true; design.BindLineHeight(hint, 12.5, 1.58);
            hint.Margin = new(13, 0, 13, 12); body.Children.Add(hint);
        }
        var card = new Border { Child = body, BorderBrush = design.Brush("border"), BorderThickness = new(1),
            Background = design.Brush("bg-elev"), CornerRadius = new(8), HorizontalAlignment = HorizontalAlignment.Stretch };
        AutomationProperties.SetAutomationId(card, "message-error-card"); AutomationProperties.SetName(card, design.T("chat.error"));
        return card;
    }
}
