using System.Text.Json;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Action? openComputeSettings;
    private readonly Action? addComputeHost;
    private ComboBox? computeDefault;
    private TextBox? computeSearch;
    private StackPanel? computeResources;
    private TextBlock? computeError;
    private Button? computeReload;
    private readonly List<Control> computeMutations = [];
    private string computeQuery = "", computeFingerprint = "";
    private bool updatingCompute;

    private void ClearComputeMenu()
    {
        computeDefault = null; computeSearch = null; computeResources = null; computeError = null; computeReload = null;
        computeMutations.Clear(); computeFingerprint = "";
    }
    private void ShowComputeOptions()
    {
        optionSubmenu.Hide(); ClearComputeMenu();
        var owner = optionsOwner;
        var body = new Grid { Width = 274, MaxHeight = 384 };
        body.RowDefinitions.Add(new() { Height = GridLength.Auto });
        body.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) });
        body.RowDefinitions.Add(new() { Height = GridLength.Auto });
        var header = new StackPanel { Spacing = 4 };
        var hint = design.Text(design.T("compute.menu_hint"), 12); hint.Foreground = design.Brush("text-muted"); hint.TextWrapping = TextWrapping.Wrap;
        hint.Margin = new(12, 2, 12, 8); header.Children.Add(hint);
        var label = design.Text(design.T("compute.session_default"), 12); label.Foreground = design.Brush("text-muted"); label.Margin = new(8, 0, 8, 0); header.Children.Add(label);
        var selector = computeDefault = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch, Margin = new(8, 0, 8, 8), MinHeight = 34 };
        design.BindTypography(selector, size: 13); AutomationProperties.SetName(selector, label.Text);
        AutomationProperties.SetAutomationId(selector, "compute-default-analysis"); header.Children.Add(selector);
        selector.SelectionChanged += async (_, _) =>
        {
            if (updatingCompute || owner != (model.CurrentProjectId, model.CurrentSessionId) || selector.SelectedItem is not ComboBoxItem { Tag: string id }) return;
            await SaveSessionOptionAsync(() => model.Options.SetDefaultContextAsync(id, lifetime.Token)); RefreshComputeMenu();
        };
        var search = computeSearch = new TextBox { Text = computeQuery, PlaceholderText = design.T("compute.search"),
            Margin = new(0), BorderThickness = new(0), MinHeight = 34, Padding = new(0, 6, 5, 6), Background = design.Brush("bg-input") };
        design.BindTypography(search, size: 13); AutomationProperties.SetName(search, design.T("compute.search"));
        AutomationProperties.SetAutomationId(search, "compute-search");
        var searchRow = new Grid { ColumnSpacing = 8, Padding = new(9, 0, 0, 0) };
        searchRow.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); searchRow.ColumnDefinitions.Add(new());
        searchRow.Children.Add(design.Icon("search", 15)); Grid.SetColumn(search, 1); searchRow.Children.Add(search);
        header.Children.Add(new Border { Child = searchRow, Background = design.Brush("bg-input"), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1), CornerRadius = new(6), Margin = new(6, 4, 6, 7) });
        header.Children.Add(new Border { Height = 1, Background = design.Brush("border"), Margin = new(12, 1, 12, 5) });
        search.TextChanged += (_, _) => { computeQuery = search.Text; RefreshComputeMenu(); };
        body.Children.Add(header);
        computeResources = new StackPanel();
        var resources = new ScrollViewer { Content = computeResources, VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
        Grid.SetRow(resources, 1); body.Children.Add(resources);
        var footer = new StackPanel(); Grid.SetRow(footer, 2); body.Children.Add(footer);
        computeError = design.Text("", 12); computeError.TextWrapping = TextWrapping.Wrap; computeError.Margin = new(8, 4, 8, 4); footer.Children.Add(computeError);
        computeReload = new Button { Content = design.L("重新读取", "Reload"), Margin = new(8, 0, 8, 4) }; design.QuietButton(computeReload);
        computeReload.Click += async (_, _) => { if (owner == (model.CurrentProjectId, model.CurrentSessionId)) await model.Options.LoadAsync(lifetime.Token); };
        footer.Children.Add(computeReload);
        if (addComputeHost != null)
        {
            footer.Children.Add(new Border { Height = 1, Background = design.Brush("border"), Margin = new(0, 4, 0, 0) });
            var add = new Button { Content = design.Text(design.T("compute.add_host"), 12.5), MinHeight = 38, HorizontalAlignment = HorizontalAlignment.Stretch };
            AutomationProperties.SetAutomationId(add, "compute-add-host"); design.QuietButton(add);
            add.Click += (_, _) =>
            {
                optionSubmenu.Hide(); composerOptions.Hide();
                if (owner == (model.CurrentProjectId, model.CurrentSessionId)) addComputeHost();
            };
            footer.Children.Add(add);
        }
        if (openComputeSettings != null)
        {
            footer.Children.Add(new Border { Height = 1, Background = design.Brush("border"), Margin = new(0, 4, 0, 0) });
            var manage = new Button { Content = design.Text(design.T("compute.manage"), 12.5), MinHeight = 38, HorizontalAlignment = HorizontalAlignment.Stretch };
            design.QuietButton(manage);
            manage.Click += (_, _) => { optionSubmenu.Hide(); composerOptions.Hide(); if (owner == (model.CurrentProjectId, model.CurrentSessionId)) openComputeSettings(); };
            footer.Children.Add(manage);
        }
        RefreshComputeMenu(); optionSubmenu.Content = body;
        optionSubmenuAnchor = computeChoice;
        optionSubmenu.Placement = FlyoutPlacementMode.RightEdgeAlignedBottom;
        optionSubmenu.ShowAt(computeChoice);
    }

    private void RefreshComputeMenu()
    {
        if (computeDefault == null || computeSearch == null || computeResources == null) return;
        var state = model.Options;
        computeDefault.IsEnabled = state.CanEditCompute;
        foreach (var control in computeMutations) control.IsEnabled = state.CanEditCompute;
        computeError!.Text = state.ErrorText(design.Localization) ?? "";
        computeError.Visibility = computeReload!.Visibility = state.Error == null ? Visibility.Collapsed : Visibility.Visible;
        computeReload.IsEnabled = !state.Busy;
        var choices = NativeComputeMenu.DefaultChoices(state.Contexts, design.Localization);
        var selected = NativeComputeMenu.DefaultId(state.Contexts?.DefaultContext?.ContextId, state.GlobalDefaultContext);
        var resources = NativeComputeMenu.Resources(state.Contexts, state.SshAliases, state.GlobalDefaultContext, computeQuery);
        var fingerprint = JsonSerializer.Serialize(new { choices, selected, resources });
        if (fingerprint == computeFingerprint) return;
        computeFingerprint = fingerprint; updatingCompute = true;
        try
        {
            computeDefault.Items.Clear();
            foreach (var choice in choices) computeDefault.Items.Add(new ComboBoxItem { Content = choice.Label, Tag = choice.Id });
            computeDefault.SelectedItem = computeDefault.Items.OfType<ComboBoxItem>().FirstOrDefault(item => (string)item.Tag == selected);
        }
        finally { updatingCompute = false; }
        var owner = optionsOwner;
        computeResources.Children.Clear(); computeMutations.Clear();
        foreach (var resource in resources)
        {
            var row = new Grid { ColumnSpacing = 4, MinHeight = 38, Padding = new(0, 2, 5, 2) };
            row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var toggle = new Button { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, Padding = new(9, 6, 0, 6), MinHeight = 34 };
            design.QuietButton(toggle);
            var content = new Grid { ColumnSpacing = 8 };
            content.ColumnDefinitions.Add(new() { Width = new GridLength(22) }); content.ColumnDefinitions.Add(new()); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            content.Children.Add(design.Icon(resource.Icon, 15));
            var names = new StackPanel { Spacing = 2, VerticalAlignment = VerticalAlignment.Center };
            var name = design.Text(resource.Name, 14); name.MaxLines = 1; name.TextTrimming = TextTrimming.CharacterEllipsis; names.Children.Add(name);
            var badges = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
            foreach (var key in new[] { resource.WslDefault ? "compute.wsl_default" : null, resource.Default ? "compute.analysis_default" : null }.OfType<string>())
            {
                var badge = design.Text(design.T(key), 10); badge.Foreground = design.Brush("clay-strong");
                badges.Children.Add(new Border { Child = badge, BorderBrush = design.Brush("clay"), BorderThickness = new(1), CornerRadius = new(8), Padding = new(5, 0, 5, 0) });
            }
            if (badges.Children.Count > 0) names.Children.Add(badges);
            Grid.SetColumn(names, 1); content.Children.Add(names);
            var status = design.Text(design.T(resource.StateKey), 11); status.Foreground = design.Brush(resource.Attached ? "clay-strong" : "text-faint");
            status.VerticalAlignment = VerticalAlignment.Center; status.MaxWidth = 90; status.TextWrapping = TextWrapping.Wrap;
            Grid.SetColumn(status, 2); content.Children.Add(status); toggle.Content = content;
            AutomationProperties.SetName(toggle, resource.Name + ": " + design.T(resource.StateKey));
            AutomationProperties.SetAutomationId(toggle, "compute-resource-" + resource.Id); ToolTipService.SetToolTip(toggle, resource.Name);
            toggle.Click += async (_, _) => { if (owner == (model.CurrentProjectId, model.CurrentSessionId)) await SaveSessionOptionAsync(() => state.SetContextAsync(resource.Id, !resource.Attached, lifetime.Token)); };
            row.Children.Add(toggle);
            var star = design.ToolButton(design.T(resource.Default ? "compute.clear_default" : "compute.set_default"), "star");
            star.Width = 22; star.MinWidth = 0; star.Padding = new(0); star.Height = 28; star.MinHeight = 0;
            star.Foreground = design.Brush(resource.Default ? "clay-strong" : "text-faint"); star.VerticalAlignment = VerticalAlignment.Center;
            AutomationProperties.SetAutomationId(star, "compute-default-" + resource.Id);
            star.Click += async (_, _) => { if (owner == (model.CurrentProjectId, model.CurrentSessionId)) await SaveSessionOptionAsync(() => state.SetDefaultContextAsync(resource.Default ? "local" : resource.Id, lifetime.Token)); };
            Grid.SetColumn(star, 1); row.Children.Add(star); computeResources.Children.Add(row);
            toggle.IsEnabled = star.IsEnabled = state.CanEditCompute; computeMutations.Add(toggle); computeMutations.Add(star);
        }
    }
}
