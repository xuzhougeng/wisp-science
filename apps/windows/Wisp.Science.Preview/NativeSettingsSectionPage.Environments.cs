using System.Globalization;
using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private readonly Dictionary<string, FrameworkElement> environmentProbeFeedback = [];
    private TextBlock EnvironmentText(string text, double size = 12, string color = "text-muted", bool strong = false)
        => CredentialText(text, size, color, strong);
    private Button EnvironmentButton(string label, Func<Task> action, bool primary = false, bool small = false)
    {
        var button = Button(label, action); StyleGeneralButton(button, primary);
        button.Padding = small ? new Thickness(9, 6, 9, 6) : new Thickness(12, 7, 12, 7);
        button.MinHeight = button.MinWidth = 0; button.CornerRadius = new CornerRadius(6);
        if (!primary) button.Background = new SolidColorBrush(Microsoft.UI.Colors.Transparent);
        else
        {
            button.Resources["ButtonBackgroundDisabled"] = new SolidColorBrush(Design.Brush("clay").Color);
            button.Resources["ButtonBorderBrushDisabled"] = new SolidColorBrush(Design.Brush("clay").Color);
            button.Resources["ButtonForegroundDisabled"] = new SolidColorBrush(Design.Brush("on-clay").Color);
        }
        if (button.Content is TextBlock text) { Design.BindTypography(text, small ? 11.5 : 12); text.Foreground = Design.Brush(primary ? "on-clay" : "text-muted"); }
        return button;
    }
    private async Task EnvironmentAction(Func<Task<bool>> action, bool refreshContexts = false)
    {
        var success = await action();
        // The standalone dialog closes on confirmed save. The next Compute menu
        // load reads fresh hosts/contexts; do not refresh a detached settings page.
        if (standaloneEnvironmentHost) return;
        if (success && refreshContexts && !model.Closed) await model.RefreshEnvironmentContextsAsync();
        if (!model.Closed && model.EnvironmentEditor is not ("host" or "runtime" or "remove")) Render();
    }
    private void Environments()
    {
        Body.Padding = new Thickness(24, 18, 24, 22); Body.Spacing = 0; Form.Spacing = 14;
        environmentProbeFeedback.Clear();
        Form.Children.Add(EnvironmentText(T("environments.hint")));
        var defaults = new StackPanel { Spacing = 6 };
        var defaultLabel = EnvironmentText(T("environments.default_analysis"), 13, "text", true); defaults.Children.Add(defaultLabel);
        var options = new[] { (Id: "", Label: T("compute.default_local")) }.Concat(NativeEnvironments.RemoteOptions(model.Environments)).ToArray();
        var choice = new ComboBox { HorizontalAlignment = HorizontalAlignment.Stretch, MinHeight = 36, Padding = new Thickness(10, 0, 10, 0),
            CornerRadius = new CornerRadius(6), Background = Design.Brush("bg-input"), BorderBrush = Design.Brush("border-strong"), BorderThickness = new Thickness(1) };
        Design.BindTypography(choice, size: 13); AutomationProperties.SetLabeledBy(choice, defaultLabel); choices.Add(choice);
        foreach (var option in options) choice.Items.Add(new ComboBoxItem { Content = option.Label, Tag = option.Id });
        choice.SelectedIndex = Math.Max(0, Array.FindIndex(options, option => option.Id == (model.DefaultEnvironment ?? "")));
        choice.SelectionChanged += async (_, _) =>
        {
            if (choice.SelectedItem is ComboBoxItem selected && (string)selected.Tag != (model.DefaultEnvironment ?? ""))
                await EnvironmentAction(() => model.SetDefaultEnvironmentAsync((string)selected.Tag));
        };
        defaults.Children.Add(choice); defaults.Children.Add(EnvironmentText(T("environments.default_analysis_hint")));
        Form.Children.Add(new Border { Child = defaults, Padding = new Thickness(14, 12, 14, 12), CornerRadius = new CornerRadius(8),
            Background = Design.Brush("bg-sunken"), BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(1) });
        if (model.EnvironmentCleanupError is { Length: > 0 } cleanup)
            Form.Children.Add(EnvironmentText(T("environments.trust_cleanup_failed") + ": " + cleanup, 12, "err"));
        var actions = new NativeActionWrap { Gap = 10 };
        var add = EnvironmentButton(T("hosts.add"), () => { model.OpenEnvironmentHost(); return Task.CompletedTask; }, primary: true);
        var content = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 7 };
        content.Children.Add(Design.Icon("plus", 16, Design.ColorText("on-clay"))); content.Children.Add(EnvironmentText(T("hosts.add"), 12, "on-clay", true)); add.Content = content;
        actions.Children.Add(add); actions.Children.Add(EnvironmentButton(T("hosts.import"), () => EnvironmentAction(() => model.ImportEnvironmentsAsync(false), true)));
        var wsl = EnvironmentButton(T("contexts.import_wsl"), () => EnvironmentAction(() => model.ImportEnvironmentsAsync(true)));
        var toolbar = PermissionRow(actions, wsl, 350); Form.Children.Add(toolbar);
        if (model.EnvironmentNoticeKey is { } notice) Form.Children.Add(EnvironmentText(T(notice), 12, "ok"));
        if (model.Error != null && model.EnvironmentEditor == null) Form.Children.Add(EnvironmentText(model.Error, 12, "err"));
        var list = new StackPanel { BorderBrush = Design.Brush("border"), BorderThickness = new Thickness(0, 1, 0, 0) };
        if (model.Environments.Length == 0) list.Children.Add(EnvironmentText(T("environments.empty")));
        foreach (var context in model.Environments)
        {
            var id = S(context, "id"); var ssh = S(context, "kind") == "ssh"; var alias = id.StartsWith("ssh:", StringComparison.Ordinal) ? id[4..] : id;
            var copy = new StackPanel { Spacing = 4 };
            copy.Children.Add(EnvironmentText(S(context, "kind") == "local" ? T("compute.local") : NativeEnvironments.Label(context), 14, "text", true));
            var subtitle = EnvironmentText(NativeEnvironments.Connection(context, model.EnvironmentHosts) + " · " + NativeEnvironments.Capabilities(context), 12, "text-faint");
            subtitle.TextTrimming = TextTrimming.CharacterEllipsis; subtitle.TextWrapping = TextWrapping.NoWrap; copy.Children.Add(subtitle);
            var feedback = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, Visibility = Visibility.Collapsed };
            feedback.Children.Add(new ProgressRing { IsActive = true, Width = 11, Height = 11, Foreground = Design.Brush("clay") });
            feedback.Children.Add(EnvironmentText(T(ssh ? "contexts.probing_ssh" : "contexts.probing_local"), 11.5, "clay")); copy.Children.Add(feedback); environmentProbeFeedback[id] = feedback;
            var rowActions = new NativeActionWrap { Gap = 6 };
            if (ssh) rowActions.Children.Add(EnvironmentButton(T("environments.edit"), () => { model.OpenEnvironmentHost(alias); return Task.CompletedTask; }, small: true));
            var configure = EnvironmentButton(T("runtime.configure"), () => { model.OpenEnvironmentRuntime(id); return Task.CompletedTask; }, small: true);
            AutomationProperties.SetName(configure, T("contexts.configure_interpreters")); rowActions.Children.Add(configure);
            rowActions.Children.Add(EnvironmentButton(T("contexts.probe"), () => EnvironmentAction(() => model.ProbeEnvironmentAsync(id)), small: true));
            if (ssh)
            {
                var remove = Design.ToolButton(T("environments.remove"), "close"); remove.Width = remove.Height = 28; remove.MinWidth = remove.MinHeight = 0;
                remove.Padding = new Thickness(0); remove.Content = Design.Icon("close", 15, Design.ColorText("text-faint"));
                remove.Click += async (_, _) => await EnvironmentAction(() => model.RequestRemoveEnvironmentHostAsync(alias)); rowActions.Children.Add(remove);
            }
            else rowActions.Children.Add(new Border { Width = 28, Height = 28 });
            var info = PermissionRow(copy, rowActions, 190);
            var row = new Grid { ColumnSpacing = 12, Padding = new Thickness(2, 13, 2, 13) };
            row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new());
            row.Children.Add(new Border { Width = 36, Height = 36, VerticalAlignment = VerticalAlignment.Center, CornerRadius = new CornerRadius(8), Background = Design.Brush("bg-sunken"),
                Child = Design.Icon("server", 18, Design.ColorText("text-faint")) }); Grid.SetColumn(info, 1); row.Children.Add(info); list.Children.Add(row);
            foreach (var edge in model.EnvironmentTrust.Where(edge => S(edge, "source_context_id") == id || S(edge, "destination_context_id") == id))
            {
                var source = S(edge, "source_context_id"); var destination = S(edge, "destination_context_id");
                static string Short(string value) => value.StartsWith("ssh:", StringComparison.Ordinal) ? value[4..] : value;
                var trustCopy = new Grid { ColumnSpacing = 10, RowSpacing = 3 };
                trustCopy.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); trustCopy.ColumnDefinitions.Add(new());
                trustCopy.RowDefinitions.Add(new() { Height = GridLength.Auto }); trustCopy.RowDefinitions.Add(new() { Height = GridLength.Auto });
                var trustTitle = EnvironmentText(Short(source) + " → " + Short(destination), 12, "text-muted", true); trustCopy.Children.Add(trustTitle);
                var target = S(edge, "destination_target") + (edge["destination_port"] != null ? ":" + edge["destination_port"] : "");
                var date = long.TryParse(NativeEnvironments.Text(edge, "verified_at"), out var timestamp)
                    ? DateTimeOffset.FromUnixTimeSeconds(timestamp).LocalDateTime.ToString("d", CultureInfo.GetCultureInfo(Design.Localization.Locale == "zh" ? "zh-CN" : "en-US")) : "";
                var trustDetail = EnvironmentText(T(B(edge, "managed") ? "environments.trust_managed" : "environments.trust_verified") + " · " + target + " · " + date, 12, "text-faint");
                trustDetail.TextWrapping = TextWrapping.NoWrap; trustDetail.TextTrimming = TextTrimming.CharacterEllipsis;
                Grid.SetColumn(trustDetail, 1); trustCopy.Children.Add(trustDetail);
                void ArrangeTrust()
                {
                    var stacked = trustCopy.ActualWidth < 460; Grid.SetColumnSpan(trustTitle, stacked ? 2 : 1);
                    Grid.SetColumn(trustDetail, stacked ? 0 : 1); Grid.SetRow(trustDetail, stacked ? 1 : 0); Grid.SetColumnSpan(trustDetail, stacked ? 2 : 1);
                }
                trustCopy.Loaded += (_, _) => ArrangeTrust(); trustCopy.SizeChanged += (_, _) => ArrangeTrust();
                var trust = PermissionRow(trustCopy, EnvironmentButton(T("environments.trust_revoke"), () => EnvironmentAction(() => model.RevokeEnvironmentTrustAsync(source, destination)), small: true), 240);
                trust.Margin = new Thickness(50, 0, 2, 12); list.Children.Add(trust);
            }
            list.Children.Add(new Border { Height = 1, Background = Design.Brush("border") });
        }
        Form.Children.Add(list);
    }
}
