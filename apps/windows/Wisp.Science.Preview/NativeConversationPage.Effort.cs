using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Flyout effortMenu = new() { Placement = FlyoutPlacementMode.TopEdgeAlignedRight };
    private bool effortMenuOpen;
    private string? effortMenuBinding;

    private void BuildEffortPicker()
    {
        efforts.Padding = new(10, 0, 10, 0);
        efforts.BorderThickness = new(1);
        efforts.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        AutomationProperties.SetAutomationId(efforts, "composer-effort-trigger");
        effortMenu.Opened += (_, _) => effortMenuOpen = true;
        effortMenu.Closed += (_, _) =>
        {
            effortMenuOpen = false;
            if (!disposed && efforts.IsLoaded && efforts.IsEnabled) efforts.Focus(FocusState.Programmatic);
        };
        efforts.Click += (_, _) =>
        {
            if (!model.CanSelectEffort) return;
            modelMenu.Hide(); composerOptions.Hide(); composeMenu.Hide();
            RebuildEffortMenu();
            effortMenu.ShowAt(efforts);
        };
        Unloaded += (_, _) => effortMenu.Hide();
    }

    private string EffortBinding => model.CurrentProjectId + "/" + model.CurrentSessionId + "/" + model.ComposerModelId;
    private void RefreshEffortPicker()
    {
        if (effortMenuOpen && (!model.CanSelectEffort || effortMenuBinding != EffortBinding)) effortMenu.Hide();
        efforts.Visibility = model.Effort.Available ? Visibility.Visible : Visibility.Collapsed;
        efforts.IsEnabled = !preparingSession && model.CanSelectEffort;
        efforts.Background = design.Brush("bg-elev"); efforts.BorderBrush = design.Brush("border-strong");
        efforts.Resources["ButtonBackgroundPointerOver"] = design.Brush("bg-sunken");
        efforts.Resources["ButtonForegroundPointerOver"] = design.Brush("text");
        var label = NativeComposerEffortModel.DisplayLabel(model.Effort.Value, design.Localization);
        AutomationProperties.SetName(efforts, design.T("composer.effort") + ": " + label);
        ToolTipService.SetToolTip(efforts, design.T("composer.effort"));
        var row = new Grid { ColumnSpacing = 5 };
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new());
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        row.Children.Add(design.Icon("brain", 14, design.ColorText("text-faint")));
        var text = design.Text(label, 12); text.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        text.Foreground = design.Brush("text-muted"); text.TextWrapping = TextWrapping.NoWrap;
        text.TextTrimming = TextTrimming.CharacterEllipsis; text.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(text, 1); row.Children.Add(text);
        var arrow = design.Icon("chevron-down", 11, design.ColorText("text-faint")); Grid.SetColumn(arrow, 2); row.Children.Add(arrow);
        efforts.Content = row;
    }

    private void RebuildEffortMenu()
    {
        effortMenuBinding = EffortBinding;
        var bound = effortMenuBinding;
        var presenter = new Style(typeof(FlyoutPresenter));
        presenter.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
        presenter.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
        presenter.Setters.Add(new Setter(Control.BorderThicknessProperty, new Thickness(1)));
        presenter.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(12)));
        presenter.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(6)));
        effortMenu.FlyoutPresenterStyle = presenter;
        var body = new StackPanel { MinWidth = 166, MaxWidth = Math.Max(166, Math.Min(340, (XamlRoot?.Size.Width ?? 360) - 28)) };
        var title = design.Text(design.T("settings.reasoning_effort").ToUpperInvariant(), 11);
        title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; title.CharacterSpacing = 40;
        title.Foreground = design.Brush("text-faint"); title.Margin = new(10, 4, 10, 6); body.Children.Add(title);
        if (model.Effort.Unverified)
        {
            var hint = design.Text(design.T("composer.effort.unverified"), 11);
            hint.Foreground = design.Brush("text-faint"); hint.MaxWidth = 220; hint.Margin = new(10, 4, 10, 6); body.Children.Add(hint);
        }
        foreach (var value in new[] { "" }.Concat(model.Effort.Options))
        {
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = new GridLength(14) });
            var label = NativeComposerEffortModel.DisplayLabel(value, design.Localization);
            var text = design.Text(label, 13); text.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
            text.Foreground = design.Brush("text"); text.TextWrapping = TextWrapping.NoWrap; text.TextTrimming = TextTrimming.CharacterEllipsis;
            row.Children.Add(text);
            if (value == model.Effort.Value)
            { var check = design.Icon("check", 14, design.ColorText("clay-strong")); Grid.SetColumn(check, 1); row.Children.Add(check); }
            var choice = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Stretch, Padding = new(10, 8, 10, 8), MinHeight = 0,
                CornerRadius = new(8), BorderThickness = new(0), Background = design.Brush("bg-elev") };
            choice.Resources["ButtonBackgroundPointerOver"] = design.Brush("bg-sunken");
            AutomationProperties.SetName(choice, label);
            AutomationProperties.SetAutomationId(choice, "composer-effort-" + (value.Length == 0 ? "default" : value));
            if (value == model.Effort.Value) AutomationProperties.SetHelpText(choice, design.L("当前选中", "Selected"));
            choice.Click += async (_, _) =>
            {
                effortMenu.Hide();
                if (!disposed && EffortBinding == bound && value != model.Effort.Value)
                    await model.SelectEffortAsync(value, lifetime.Token);
            };
            body.Children.Add(choice);
        }
        effortMenu.Content = new ScrollViewer { Content = body, MaxHeight = Math.Min(306, (XamlRoot?.Size.Height ?? 640) / 2 - 14),
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    }
}
