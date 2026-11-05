using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;
using static Wisp.ProjectBrowser.NativeSkillsModel;

namespace Wisp.Science.Preview;

internal sealed class NativePluginsPage : NativeActionPage
{
    private readonly NativePluginsModel model;
    private readonly Action close;
    private readonly Func<Task<string?>> pickZip;
    private readonly Func<NativePluginLaunch, Task> usePlugin;
    private readonly SettingsTextMenus menus = new();
    private readonly List<Control> dialogControls = [];
    private readonly HashSet<string> expanded = [];
    private readonly StackPanel rows = new() { Spacing = 0 };
    private readonly ScrollViewer listScroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly TextBlock empty = new() { TextWrapping = TextWrapping.Wrap, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(24, 18, 24, 24) };
    private TextBlock? summary, dialogError;
    private Button? submit;
    private bool picking;
    private bool starting;
    public event Action<FrameworkElement?>? ModalChanged;
    public NativePluginsPage(INativeSettingsClient client, string? project, WispDesign design, Action close, Func<Task<string?>> pickZip, Func<NativePluginLaunch, Task> usePlugin)
        : this(new NativePluginsModel(client, project, design.Localization), design, close, pickZip, usePlugin) { _ = Load(); }
    private NativePluginsPage(NativePluginsModel model, WispDesign design, Action close, Func<Task<string?>> pickZip, Func<NativePluginLaunch, Task> usePlugin) : base(design, design.T("settings.nav.plugins"), model, close)
    {
        this.model = model; this.close = close; this.pickZip = pickZip; this.usePlugin = usePlugin;
        UseSettingsLayout(); Form.MaxWidth = double.PositiveInfinity; Form.Spacing = 12; Footer.Visibility = Visibility.Collapsed;
        // The toolbar stays visible while installed rows scroll, as in WebView.
        Body.Padding = new Thickness(24, 14, 24, 0); Body.Spacing = 0;
        BodyScroll.Content = null; BodyScroll.VerticalScrollMode = ScrollMode.Disabled; BodyScroll.VerticalScrollBarVisibility = ScrollBarVisibility.Disabled;
        var listLayout = new Grid(); listLayout.RowDefinitions.Add(new() { Height = GridLength.Auto }); listLayout.RowDefinitions.Add(new() { Height = new GridLength(1, GridUnitType.Star) }); listLayout.RowDefinitions.Add(new() { Height = GridLength.Auto });
        listLayout.Children.Add(Body); listScroll.Content = rows; listScroll.HorizontalAlignment = HorizontalAlignment.Center; Grid.SetRow(listScroll, 1); listLayout.Children.Add(listScroll);
        empty.Foreground = Design.Brush("text-muted"); Design.BindTypography(empty, 12.5); empty.Visibility = Visibility.Collapsed; Grid.SetRow(empty, 2); listLayout.Children.Add(empty); BodyScroll.Content = listLayout;
        void Resize() { listLayout.Height = Math.Max(0, BodyScroll.ActualHeight); listScroll.Width = Math.Max(0, Math.Min(1040, ActualWidth) - 48); }
        SizeChanged += (_, _) => Resize(); BodyScroll.SizeChanged += (_, _) => Resize(); Loaded += (_, _) => Resize();
        model.Changed += Refresh; Render();
    }
    private TextBlock Text(string value, double size = 13, bool bold = false)
    { var text = Design.Text(value, size); if (bold) text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; return text; }
    private TextBox Input(string label, string value, string placeholder, Action<string>? changed = null)
    {
        var input = new TextBox { Text = value, PlaceholderText = placeholder, CornerRadius = new CornerRadius(10), Padding = new Thickness(11, 9, 11, 9),
            Background = Design.Brush("bg-input"), Foreground = Design.Brush("text"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        AutomationProperties.SetName(input, label.Length > 0 ? label : placeholder); menus.Register(input);
        if (changed != null) input.TextChanged += (_, _) => changed(input.Text); return input;
    }
    private string T(string key) => Design.T(key);
    private string Summary() => Design.Localization.Format("plugins.summary", ("enabled", model.EnabledCount.ToString()), ("total", model.Plugins.Count.ToString()));
    private StackPanel DialogField(string label, UIElement content)
    {
        var field = new StackPanel { Spacing = 5 };
        var caption = Text(label, 12); caption.Foreground = Design.Brush("text-muted"); field.Children.Add(caption); field.Children.Add(content); return field;
    }
    private Border DialogInput(TextBox input)
    {
        Design.BindTypography(input, code: true, size: 14);
        return NativeDialogVisuals.Input(input, Design);
    }
    private void Primary(Button button)
    {
        Design.ActionButton(button, true);
        if (button.Content is TextBlock text) { text.Foreground = Design.Brush("on-clay"); text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; }
        button.Resources["ButtonBackgroundDisabled"] = Design.Brush("clay");
        button.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay");
    }
    private async Task Load() { await model.LoadAsync(); if (!model.Closed) Render(); }
    private void Refresh()
    {
        SetContentEnabled(!model.Busy && !model.ModalOpen && !starting);
        listScroll.IsEnabled = !model.Busy && !model.ModalOpen && !starting;
        foreach (var control in dialogControls) control.IsEnabled = !model.Busy && !picking;
        if (submit != null) { submit.IsEnabled = !picking && (model.Installing ? model.CanInstall : !model.Busy); submit.Opacity = submit.IsEnabled ? 1 : .5; }
        if (dialogError != null) { dialogError.Text = model.Error ?? ""; dialogError.Visibility = model.Error == null ? Visibility.Collapsed : Visibility.Visible; }
    }
    public override void HandleEscape()
    {
        if (menus.CloseTop()) return;
        if (picking || starting || model.Busy) return;
        if (model.ModalOpen) { DismissModal(); return; }
        close();
    }
    public void RequestLeave(Action leave) { if (!model.Busy && !picking && !starting && !model.ModalOpen) leave(); }
    private void DismissModal() { if (!picking && model.CloseModal()) { HideModal(); Refresh(); } }
    private void HideModal() { ModalChanged?.Invoke(null); dialogControls.Clear(); submit = null; dialogError = null; }
    private void Render()
    {
        if (model.Closed) return;
        menus.CloseTop(); Form.Children.Clear(); rows.Children.Clear(); SettingsHeading(T("settings.nav.plugins"));
        if (!model.Loaded || model.ProjectId == null)
        {
            Form.Children.Add(Mute(model.Error != null ? Design.L("插件尚未载入，请重试。", "Could not load plugins. Please try again.")
                : !model.Loaded ? Design.L("正在载入插件…", "Loading plugins…") : Design.L("请先创建项目，再配置插件。", "Create a project before configuring plugins.")));
            if (!model.Busy) Form.Children.Add(Button(Design.L("重新载入", "Reload"), Load)); return;
        }
        var toolbar = new Grid { ColumnSpacing = 10, RowSpacing = 8 };
        toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); toolbar.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); toolbar.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto }); toolbar.RowDefinitions.Add(new() { Height = GridLength.Auto });
        summary = Text(Summary(), 13); summary.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        var summaryPill = new Border { Child = summary, Padding = new Thickness(12, 7, 12, 7), VerticalAlignment = VerticalAlignment.Center,
            Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(10) };
        toolbar.Children.Add(summaryPill);
        var search = Input("", model.Query, T("plugins.search"), value => { model.Query = value; DrawRows(); }); Grid.SetColumn(search, 1); toolbar.Children.Add(search);
        var install = Button(T("plugins.install_action"), () => { if (model.OpenInstall()) InstallDialog(); return Task.CompletedTask; }); Primary(install);
        install.Padding = new Thickness(12, 7, 12, 7); install.MinHeight = 0; install.CornerRadius = new CornerRadius(10);
        if (install.Content is TextBlock installCaption) { Design.BindTypography(installCaption, 12.5); installCaption.FontWeight = Microsoft.UI.Text.FontWeights.Medium; }
        Grid.SetColumn(install, 2); toolbar.Children.Add(install);
        void LayoutToolbar()
        {
            var narrow = NativeSettingsLayout.DefinitionFieldsStack(XamlRoot?.Size.Width ?? ActualWidth);
            Grid.SetColumnSpan(summaryPill, narrow ? 2 : 1); Grid.SetRow(search, narrow ? 1 : 0); Grid.SetColumn(search, narrow ? 0 : 1); Grid.SetColumnSpan(search, narrow ? 3 : 1);
        }
        toolbar.SizeChanged += (_, _) => LayoutToolbar(); toolbar.Loaded += (_, _) => LayoutToolbar();
        Form.Children.Add(new Border { Child = toolbar, Padding = new Thickness(0, 0, 0, 12), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
        if (model.Notice != null) Form.Children.Add(Mute(model.Notice)); DrawRows(); Design.ApplyTypography(Form); Refresh();
    }
    private void DrawRows()
    {
        rows.Children.Clear(); if (summary != null) summary.Text = Summary();
        foreach (var row in model.Visible)
        {
            var labels = new StackPanel { Spacing = 8 }; var heading = new NativeActionWrap(); heading.Children.Add(Text(S(row, "display_name"), 15, true)); heading.Children.Add(Mute("v" + S(row, "version"))); labels.Children.Add(heading);
            if (S(row, "description").Length > 0) labels.Children.Add(Mute(S(row, "description")));
            var state = new NativeActionWrap(); state.Children.Add(Mute(T(B(row, "enabled") ? "plugins.enabled_project" : "plugins.disabled_project"))); state.Children.Add(Mute(NativePluginsModel.RuntimeLabel(row, Design.Localization))); labels.Children.Add(state);
            var details = new StackPanel { Spacing = 6, Visibility = expanded.Contains(NativePluginsModel.Identity(row)) ? Visibility.Visible : Visibility.Collapsed };
            var skills = Strings(row["skill_names"]); var commands = Strings(row["commands"]);
            details.Children.Add(Mute(T("plugins.provides_skills") + ": " + (skills.Length > 0 ? string.Join(", ", skills) : row["skill_count"]?.ToString() ?? "0")));
            details.Children.Add(Mute(T("plugins.mcp_servers") + ": " + (row["mcp_server_count"]?.ToString() ?? "0") + (commands.Length > 0 ? " · " + string.Join(" · ", commands) : "")));
            details.Children.Add(Mute(T("plugins.verify") + ": " + S(row, "trust_state")));
            if (Strings(row["runtime_errors"]).Length > 0) details.Children.Add(Warn(T("plugins.runtime") + ": " + string.Join(" · ", Strings(row["runtime_errors"]))));
            var detail = Button(T("plugins.details"), () => { var key = NativePluginsModel.Identity(row); if (!expanded.Add(key)) expanded.Remove(key); details.Visibility = expanded.Contains(key) ? Visibility.Visible : Visibility.Collapsed; return Task.CompletedTask; }); Design.QuietButton(detail); detail.HorizontalAlignment = HorizontalAlignment.Left; labels.Children.Add(detail); labels.Children.Add(details);
            var actions = new NativeActionWrap { HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Top };
            var use = Button(T(B(row, "enabled") ? "plugins.use_new_session" : "plugins.enable_and_use"), () => Use(row));
            use.IsEnabled = S(row, "runtime_status") != "unavailable"; actions.Children.Add(use);
            var remove = Design.ToolButton(T("plugins.remove"), "close"); remove.Click += (_, _) => { if (model.RequestRemoval(row)) RemoveDialog(); }; actions.Children.Add(remove);
            var enabled = new ToggleSwitch { IsOn = B(row, "enabled"), OnContent = null, OffContent = null, MinWidth = 0 }; AutomationProperties.SetName(enabled, T(B(row, "enabled") ? "plugins.disable_project" : "plugins.enable_project") + ": " + S(row, "display_name"));
            PreferenceToggleSwitch.Apply(enabled, Design);
            enabled.Toggled += async (_, _) => { var next = enabled.IsOn; enabled.IsEnabled = false; if (await model.SetEnabledAsync(row, next)) await model.LoadAsync(); if (!model.Closed) Render(); }; actions.Children.Add(enabled);
            var content = new Grid { ColumnSpacing = 18, RowSpacing = 10, Padding = new Thickness(0, 18, 0, 18) };
            content.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.RowDefinitions.Add(new() { Height = GridLength.Auto }); content.RowDefinitions.Add(new() { Height = GridLength.Auto });
            content.Children.Add(labels); Grid.SetColumn(actions, 1); content.Children.Add(actions);
            content.SizeChanged += (_, e) => { var narrow = e.NewSize.Width < 760; Grid.SetColumnSpan(labels, narrow ? 2 : 1); Grid.SetRow(actions, narrow ? 1 : 0); Grid.SetColumn(actions, narrow ? 0 : 1); Grid.SetColumnSpan(actions, narrow ? 2 : 1); actions.HorizontalAlignment = narrow ? HorizontalAlignment.Left : HorizontalAlignment.Right; };
            rows.Children.Add(new Border { Child = content, BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 0, 0, 1) });
        }
        empty.Text = T(model.Plugins.Count == 0 ? "plugins.empty" : "plugins.no_match");
        empty.Visibility = rows.Children.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        Design.ApplyTypography(rows);
    }
    private async Task Use(JsonObject row)
    {
        if (starting || model.Busy) return; starting = true; Refresh();
        try { if (await model.UseAsync(row) && model.Launch is { } launch) await usePlugin(launch); }
        catch (Exception ex) { if (!model.Closed) model.Fail(ex.Message); }
        finally { starting = false; if (!model.Closed) Render(); }
    }
    private void InstallDialog()
    {
        var body = new StackPanel { Spacing = 14, Padding = new Thickness(22) }; dialogControls.Clear();
        var heading = new Grid { ColumnSpacing = 18 }; heading.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); heading.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var titles = new StackPanel { Spacing = 4 }; titles.Children.Add(Text(T("plugins.install_title"), 17, true)); titles.Children.Add(Mute(T("plugins.install_safety"))); heading.Children.Add(titles);
        var dismiss = Design.ToolButton(T("plugins.install_close"), "close"); dismiss.VerticalAlignment = VerticalAlignment.Top; dismiss.Click += (_, _) => DismissModal(); Grid.SetColumn(dismiss, 1); heading.Children.Add(dismiss); dialogControls.Add(dismiss); body.Children.Add(heading);
        var tabs = new NativeActionWrap { Gap = 0 };
        foreach (var remote in new[] { false, true })
        {
            var tab = Button(T(remote ? "plugins.source_remote" : "plugins.source_local"), () => { model.SetRemote(remote); InstallDialog(); return Task.CompletedTask; });
            tab.Background = Design.Brush(model.Remote == remote ? "bg-elev" : "bg-sunken"); tab.BorderThickness = new Thickness(0); tab.CornerRadius = new CornerRadius(6); tab.MinHeight = 0; tab.Padding = new Thickness(13, 7, 13, 7);
            if (tab.Content is TextBlock label) { Design.BindTypography(label, 12.5); label.Foreground = Design.Brush(model.Remote == remote ? "text" : "text-muted"); }
            dialogControls.Add(tab); tabs.Children.Add(tab);
        }
        body.Children.Add(new Border { Child = tabs, Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1), Padding = new Thickness(3), CornerRadius = new CornerRadius(6), HorizontalAlignment = HorizontalAlignment.Left });
        var fields = new StackPanel { Spacing = 12 }; body.Children.Add(fields);
        if (model.Remote)
        {
            var url = Input(T("plugins.url"), model.SourceUrl, "https://github.com/…/plugin.zip", value => model.EditInstall(url: value)); dialogControls.Add(url); fields.Children.Add(DialogField(T("plugins.url"), DialogInput(url)));
        }
        else
        {
            var source = new Grid { ColumnSpacing = 8 }; source.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); source.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var path = Input(T("plugins.zip_file"), model.LocalPath, T("plugins.no_zip_selected")); path.IsReadOnly = true; dialogControls.Add(path); source.Children.Add(DialogInput(path));
            var pick = Button(T("plugins.choose_zip"), async () =>
            {
                if (picking || model.Busy) return; picking = true; Refresh();
                try { var selected = await pickZip(); if (!model.Closed && model.Installing && selected != null) { model.EditInstall(path: selected); path.Text = selected; } }
                catch (Exception ex) { if (!model.Closed) model.Fail(ex.Message); }
                finally { picking = false; if (!model.Closed) Refresh(); }
            }); pick.Padding = new Thickness(13, 9, 13, 9); pick.CornerRadius = new CornerRadius(10); pick.Background = Design.Brush("bg-input"); pick.BorderBrush = Design.Brush("border-strong"); Grid.SetColumn(pick, 1); dialogControls.Add(pick); source.Children.Add(pick); fields.Children.Add(DialogField(T("plugins.zip_file"), source));
        }
        var checksumLabel = T(model.Remote ? "plugins.sha256_required" : "plugins.sha256_optional");
        var checksum = Input(checksumLabel, model.Checksum, T("plugins.sha256_hint"), value => model.EditInstall(checksum: value)); dialogControls.Add(checksum); fields.Children.Add(DialogField(checksumLabel, DialogInput(checksum)));
        dialogError = Warn(""); fields.Children.Add(dialogError);
        submit = Button(T(model.Remote ? "plugins.install_url" : "plugins.install_action"), async () => { if (await model.InstallAsync()) { HideModal(); await model.LoadAsync(); Render(); } else Refresh(); }); Primary(submit);
        submit.Padding = new Thickness(16, 9, 16, 9); submit.CornerRadius = new CornerRadius(10); if (submit.Content is TextBlock submitLabel) Design.BindTypography(submitLabel, 13.5);
        submit.HorizontalAlignment = HorizontalAlignment.Right; fields.Children.Add(submit);
        ShowModal(body, T("plugins.install_title"), dismiss); Refresh();
    }
    private void RemoveDialog()
    {
        if (model.Removal is not { } row) return;
        dialogControls.Clear(); var body = new StackPanel { Spacing = 16, Padding = new Thickness(24) };
        body.Children.Add(Text(T("plugins.remove"), 17, true)); body.Children.Add(Text(Design.Localization.Format("plugins.remove_confirm", ("plugin", S(row, "display_name") + " v" + S(row, "version")))));
        dialogError = Warn(""); body.Children.Add(dialogError); var actions = new NativeActionWrap { HorizontalAlignment = HorizontalAlignment.Right };
        var cancel = Button(T("settings.cancel"), () => { DismissModal(); return Task.CompletedTask; }); dialogControls.Add(cancel); actions.Children.Add(cancel);
        submit = Button(T("plugins.remove"), async () => { if (await model.RemoveAsync()) { HideModal(); Render(); } else Refresh(); }); actions.Children.Add(submit); body.Children.Add(actions); ShowModal(body, T("plugins.remove"), cancel); Refresh();
    }
    private void ShowModal(StackPanel body, string name, Control focus)
    {
        var overlay = new Grid(); NativeDialogVisuals.Backdrop(overlay, Design); AutomationProperties.SetName(overlay, name);
        var card = new Border { Child = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch },
            Background = Design.Brush("bg-elev"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1), CornerRadius = new CornerRadius(16), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle,
            Shadow = new ThemeShadow(), Translation = new System.Numerics.Vector3(0, 0, 24) };
        overlay.Children.Add(card); void Resize() { card.Width = Math.Min(620, Math.Max(0, overlay.ActualWidth - 40)); card.MaxHeight = Math.Max(0, overlay.ActualHeight - 40); body.Width = Math.Max(0, card.Width - 2); }
        overlay.SizeChanged += (_, _) => Resize(); overlay.Loaded += (_, _) => { Resize(); focus.Focus(FocusState.Programmatic); };
        overlay.Tapped += (_, e) => { if (ReferenceEquals(e.OriginalSource, overlay)) DismissModal(); };
        ModalChanged?.Invoke(overlay); Design.ApplyTypography(body);
    }
    public override void Dispose() { menus.Dispose(); HideModal(); model.Changed -= Refresh; base.Dispose(); }
}
