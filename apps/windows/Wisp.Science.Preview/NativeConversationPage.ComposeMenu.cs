using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Flyout composeMenu = new() { Placement = FlyoutPlacementMode.TopEdgeAlignedLeft };
    private readonly Dictionary<string, Button> composeActions = [];
    private readonly Action<string>? composeNavigate;
    private bool composeMenuOpen;
    private void BuildComposeMenu()
    {
        var body = new StackPanel { Width = 296, Spacing = 2 };
        var title = design.Text(design.T("composer.compose"), 12); title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        title.Margin = new Thickness(10, 4, 10, 4); body.Children.Add(title);
        foreach (var entry in NativeComposeMenu.Entries)
        {
            if (entry.Id is "attach" or "review")
            {
                var group = design.Text(design.T(entry.Id == "attach" ? "composer.group_add" : "composer.group_session").ToUpperInvariant(), 10.5);
                group.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
                group.Foreground = design.Brush("text-muted"); group.Margin = new Thickness(10, 8, 10, 5); body.Children.Add(group);
            }
            var row = new Grid { ColumnSpacing = 12 };
            row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            row.Children.Add(new Border { Child = design.Icon(entry.Icon, 18), Width = 32, Height = 32, CornerRadius = new CornerRadius(6), Background = design.Brush("bg-sunken") });
            var copy = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
            copy.Children.Add(design.Text(design.T(entry.LabelKey), 14));
            var detail = design.Text(design.T(entry.DetailKey), 12); detail.Foreground = design.Brush("text-faint");
            detail.TextWrapping = TextWrapping.NoWrap; detail.TextTrimming = TextTrimming.CharacterEllipsis; copy.Children.Add(detail);
            Grid.SetColumn(copy, 1); row.Children.Add(copy);
            var chevron = design.Icon("chevron-right", 12); Grid.SetColumn(chevron, 2); row.Children.Add(chevron);
            var button = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
            design.QuietButton(button); button.Padding = new Thickness(10, 8, 10, 8); button.CornerRadius = new CornerRadius(8);
            AutomationProperties.SetName(button, design.T(entry.LabelKey)); AutomationProperties.SetAutomationId(button, "compose-" + entry.Id);
            ToolTipService.SetToolTip(button, design.T(entry.DetailKey));
            button.Click += async (_, _) => await RunComposeActionAsync(entry.Id);
            composeActions[entry.Id] = button; body.Children.Add(button);
        }
        var style = new Style(typeof(FlyoutPresenter));
        style.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        style.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        style.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(10)));
        style.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(10, 6, 10, 8)));
        composeMenu.FlyoutPresenterStyle = style;
        composeMenu.Content = new ScrollViewer { Content = body, MaxHeight = 600, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
        composeMenu.Opened += (_, _) => { composeMenuOpen = true; refreshComposerFocus?.Invoke(); };
        composeMenu.Closed += (_, _) => { composeMenuOpen = false; refreshComposerFocus?.Invoke(); };
        attach.Click += (_, _) =>
        {
            if (composeMenuOpen) composeMenu.Hide();
            else { RefreshComposeActions(); composeMenu.ShowAt(attach); }
        };
    }
    public bool HandleComposeMenuEscape()
    {
        if (!composeMenuOpen) return false;
        composeMenu.Hide(); return true;
    }
    private void RefreshComposeActions()
    {
        foreach (var (id, button) in composeActions) button.IsEnabled = !preparingSession && !clipboardBusy && (id switch {
            "attach" => CanPickAttachments && (model.CanAttach || model.CanPrepareProjectDraft),
            "files" => composeNavigate != null && (model.ComposeSessionReady || model.CanPrepareProjectDraft),
            "review" => model.CanReviewConversation,
            "share" => composeNavigate != null && model.CanShareConversation,
            "save-skill" => model.CanCompose,
            "skills" => composeNavigate != null,
            _ => false
        });
    }
    private async Task RunComposeActionAsync(string id)
    {
        RefreshComposeActions();
        if (disposed || !composeActions.TryGetValue(id, out var button) || !button.IsEnabled) return;
        composeMenu.Hide();
        switch (id)
        {
            case "attach": await DoAttachAsync(); break;
            case "review": await model.ReviewConversationAsync(lifetime.Token); break;
            case "save-skill": model.DraftSkill(design.T("composer.skill_prompt")); FocusComposer(); break;
            case "files":
                if (model.ComposeSessionReady || model.CanPrepareProjectDraft) composeNavigate?.Invoke(id);
                break;
            default: composeNavigate?.Invoke(id); break;
        }
    }
}
