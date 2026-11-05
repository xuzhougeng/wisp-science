using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private TextBlock? contextHealthLine, contextNudgeText;
    private Border? contextNudge;
    private Button? contextNudgeCompact, contextNudgeHandoff;
    private Border BuildContextNudge()
    {
        var body = new NativeActionWrap { Gap = 14, RowGap = 10, SpaceBetween = true, VerticallyCentered = true };
        contextNudgeText = design.Text("", 12.5); contextNudgeText.VerticalAlignment = VerticalAlignment.Center; body.Children.Add(contextNudgeText);
        var actions = new NativeActionWrap { Gap = 12, RowGap = 8 };
        Button Link(string key, string id)
        {
            var text = design.Text(design.T(key), 12.5); text.TextDecorations = Windows.UI.Text.TextDecorations.Underline;
            text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; text.Foreground = design.Brush("clay-strong");
            var button = new Button { Content = text, Padding = new(0), MinHeight = 26, MinWidth = 0 };
            design.QuietButton(button); button.Padding = new(0); AutomationProperties.SetAutomationId(button, id);
            AutomationProperties.SetName(button, design.T(key)); actions.Children.Add(button); return button;
        }
        contextNudgeCompact = Link("context_usage.nudge_compact", "context-usage-compact");
        contextNudgeCompact.Click += (_, _) => { CloseContextUsage(); model.OpenCompaction(); };
        var fresh = Link("context_usage.nudge_new_session", "context-usage-new-session");
        fresh.Click += async (_, _) => { CloseContextUsage(); await create(); };
        contextNudgeHandoff = Link("context_usage.nudge_handoff", "context-usage-handoff");
        contextNudgeHandoff.Click += async (_, _) => await model.Handoff.OpenAsync(lifetime.Token);
        body.Children.Add(actions);
        contextNudge = new Border { Child = body, Padding = new(10, 8, 10, 8), BorderThickness = new(1), CornerRadius = new(6), Visibility = Visibility.Collapsed };
        return contextNudge;
    }
    private void RefreshContextHealth()
    {
        var usage = NativeContextUsage.Latest(model.Snapshot?.Items ?? [], design.Localization);
        if (usage == null || contextHealthLine == null || contextNudge == null || contextNudgeText == null) return;
        var health = NativeContextHealth.Read(model.ContextDetails.Context?.State, usage);
        contextHealthLine.Text = health.Line(design.Localization) ?? "";
        contextHealthLine.Foreground = design.Brush(health.Tone);
        contextHealthLine.Visibility = contextHealthLine.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        contextNudge.Visibility = health.ShowNudge(usage) ? Visibility.Visible : Visibility.Collapsed;
        contextNudgeText.Text = design.T(health.NudgeKey(usage));
        var tone = usage.Tone == "err" || health.Level == "split-now" ? "err" : "warn";
        var color = ((SolidColorBrush)design.Brush(tone)).Color;
        contextNudge.BorderBrush = new SolidColorBrush(Windows.UI.Color.FromArgb(82, color.R, color.G, color.B));
        contextNudge.Background = new SolidColorBrush(Windows.UI.Color.FromArgb(20, color.R, color.G, color.B));
        if (contextNudgeCompact != null) contextNudgeCompact.IsEnabled = model.CanOpenCompaction;
        if (contextNudgeHandoff != null) contextNudgeHandoff.IsEnabled = model.Handoff.Available && startHandoff != null;
        foreach (var button in new[] { contextNudgeCompact, contextNudgeHandoff })
            if (button?.Content is TextBlock text) { text.Opacity = button.IsEnabled ? 1 : .5;
                text.TextDecorations = button.IsEnabled ? Windows.UI.Text.TextDecorations.Underline : Windows.UI.Text.TextDecorations.None; }
    }
}
