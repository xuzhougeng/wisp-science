using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeSessionTransferPage : WorkspaceSheet
{
    private readonly WorkspaceSessionTransfer model;
    private readonly ComboBox target = new() { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch, MinHeight = 36 };
    private readonly NativeSessionCheck artifacts;
    private readonly StackPanel details = new() { Spacing = 8 };
    private readonly TextBlock error, loading;
    private readonly Button confirm, cancel, retry;
    private bool disposed, started;
    private string? displayedFailure;
    public NativeSessionTransferPage(WorkspaceSessionTransfer model, WispDesign design, Action saved, Action close)
        : base(design, design.T(model.Mode == "copy" ? "session.copy_title" : "session.move_title"), close, modal: true, modalWidth: 520)
    {
        this.model = model; HideDismissButton();
        Body.Children.Add(Mute(design.T(model.Mode == "copy" ? "session.transfer_hint" : "session.move_hint").Replace("{title}", model.Source.Title)));
        var field = new StackPanel { Spacing = 6 }; field.Children.Add(design.Text(design.T("session.target_project"), 14));
        foreach (var project in model.Targets)
        {
            var label = design.Text(project.Name, 14); label.TextWrapping = TextWrapping.NoWrap; label.TextTrimming = TextTrimming.CharacterEllipsis;
            target.Items.Add(new ComboBoxItem { Content = label, Tag = project.Id });
        }
        target.SelectedIndex = model.Targets.Length > 0 ? 0 : -1;
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(target, design.T("session.target_project"));
        design.BindTypography(target, size: 14); PreferencePickerAppearance.Apply(target, design);
        target.SelectionChanged += async (_, _) => { if (target.SelectedItem is ComboBoxItem item && item.Tag is string id && id != model.TargetProjectId) await model.ChooseTargetAsync(id); };
        field.Children.Add(target); Body.Children.Add(field);
        artifacts = new NativeSessionCheck(design, design.T("session.artifacts.move"), 14, wrap: true) { Padding = new Thickness(0), Margin = new Thickness(0, 2, 0, 0) };
        artifacts.Click += (_, _) => model.ChooseArtifacts(!model.IncludeArtifacts);
        artifacts.Visibility = model.Mode == "move" ? Visibility.Visible : Visibility.Collapsed; Body.Children.Add(artifacts); Body.Children.Add(details);
        loading = Mute(design.T("loading")); Body.Children.Add(loading);
        error = Mute(""); error.Foreground = design.Brush("err"); Body.Children.Add(error);
        retry = new Button { Content = design.Text(design.T("session.artifacts.refresh"), 14), HorizontalAlignment = HorizontalAlignment.Left };
        retry.Click += async (_, _) => await model.LoadAsync(); Body.Children.Add(retry);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        cancel = new Button { Content = design.Text(design.T("settings.cancel"), 14) }; design.ActionButton(cancel, false); cancel.Click += (_, _) => HandleEscape(); actions.Children.Add(cancel);
        confirm = new Button { Content = design.Text(design.T(model.Mode == "copy" ? "session.copy_action" : "session.move_action"), 14) };
        design.ActionButton(confirm, true); ((TextBlock)confirm.Content).Foreground = design.Brush("on-clay");
        confirm.Click += async (_, _) => { if (await model.TransferAsync() && !disposed) saved(); }; actions.Children.Add(confirm);
        Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        model.Changed += Refresh; Refresh();
        Loaded += async (_, _) => { target.Focus(FocusState.Programmatic); if (error.Visibility == Visibility.Visible) RevealFailure(); if (!started) { started = true; await model.LoadAsync(); } };
    }
    private new TextBlock Mute(string value) { var text = Design.Text(value, 12.5); text.Foreground = Design.Brush("text-muted"); return text; }
    private void Refresh()
    {
        if (disposed) return;
        target.IsEnabled = model.Targets.Length > 0 && !model.Transferring && !model.MutationUnconfirmed;
        artifacts.SetSelected(model.IncludeArtifacts); artifacts.IsEnabled = !model.Transferring && !model.MutationUnconfirmed && model.Targets.Length > 0;
        details.Children.Clear(); details.Visibility = model.IncludeArtifacts ? Visibility.Visible : Visibility.Collapsed;
        if (model.IncludeArtifacts)
        {
            details.Children.Add(Mute(Design.T("session.artifacts.scope")));
            if (model.Preview?.Artifacts is { } preview)
            {
                details.Children.Add(Design.Text(Design.T("session.artifacts.count").Replace("{artifacts}", preview.Artifacts.Length.ToString()).Replace("{files}", preview.Files.Length.ToString()), 14));
                if (preview.Files.Length > 0) details.Children.Add(FileList(preview.Files));
                if (preview.Retained.Length > 0) { details.Children.Add(Design.Text(Design.T("session.artifacts.retained"), 14)); details.Children.Add(FileList(preview.Retained.Select(item => item.Name + " — " + Design.T("session.artifacts." + item.Reason)))); }
            }
        }
        loading.Visibility = model.Loading || model.Transferring ? Visibility.Visible : Visibility.Collapsed;
        var failure = model.Targets.Length == 0 ? Design.T("session.no_target_project") : model.Error ?? (model.IncludeArtifacts ? model.Preview?.ArtifactError : null);
        error.Text = failure == null ? "" : (model.MutationUnconfirmed ? Design.L("结果未能确认；不会自动重试。请关闭后刷新会话列表。", "The result could not be confirmed and will not be retried. Close and refresh the conversation list.") + "\n" : "") + failure;
        error.Visibility = failure == null ? Visibility.Collapsed : Visibility.Visible;
        retry.Visibility = failure != null && model.Targets.Length > 0 && !model.MutationUnconfirmed ? Visibility.Visible : Visibility.Collapsed; retry.IsEnabled = !model.Loading;
        confirm.IsEnabled = model.CanTransfer; confirm.Opacity = model.CanTransfer ? 1 : .5; cancel.IsEnabled = !model.Transferring;
        Design.ApplyTypography(this);
        if (failure != null && failure != displayedFailure) RevealFailure();
        displayedFailure = failure;
    }
    private void RevealFailure() => DispatcherQueue.TryEnqueue(() => { if (!disposed && error.Visibility == Visibility.Visible) { UpdateLayout(); error.StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 }); } });
    private ScrollViewer FileList(IEnumerable<string> paths)
    {
        var rows = new StackPanel { Spacing = 4, Padding = new Thickness(18, 0, 0, 0) };
        foreach (var path in paths) { var label = Design.Text(path, 12); label.TextWrapping = TextWrapping.WrapWholeWords; rows.Children.Add(label); }
        return new ScrollViewer { Content = rows, MaxHeight = Design.FontSize(140), HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled };
    }
    public override void HandleEscape() { if (target.IsDropDownOpen) { target.IsDropDownOpen = false; return; } if (!model.Transferring) base.HandleEscape(); }
    public override void Dispose() { disposed = true; target.IsDropDownOpen = false; model.Changed -= Refresh; model.Dispose(); base.Dispose(); }
}
