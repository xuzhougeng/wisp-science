using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeExplorationStartPage : WorkspaceSheet
{
    private readonly WorkspaceExplorationStart model;
    private readonly TextBox name = new();
    private readonly TextBlock error;
    private readonly Border errorCard;
    private readonly Button cancel, create;
    private bool disposed;
    public NativeExplorationStartPage(WorkspaceExplorationStart model, WispDesign design,
        Action<NativeExplorationSummary> completed, Action close)
        : base(design, design.T("exploration.start_title"), close, modal: true, modalWidth: 520)
    {
        this.model = model;
        var hint = design.Text(design.T("exploration.start_hint"), 13); hint.Foreground = design.Brush("text-muted");
        design.BindLineHeight(hint, 13, 1.55); Body.Children.Add(hint);
        var field = new StackPanel { Spacing = 6 }; field.Children.Add(design.Text(design.T("exploration.name"), 13));
        design.BindTypography(name, size: 13); name.Text = model.Name; name.CornerRadius = new(8);
        name.Background = design.Brush("bg-input"); name.BorderBrush = design.Brush("border"); name.BorderThickness = new(1);
        AutomationProperties.SetName(name, design.T("exploration.name")); AutomationProperties.SetAutomationId(name, "exploration-name");
        name.TextChanged += (_, _) => { model.Name = name.Text; Refresh(); }; field.Children.Add(NativeDialogVisuals.Input(name, design)); Body.Children.Add(field);
        error = design.Text("", 12); error.Foreground = design.Brush("err"); design.BindLineHeight(error, 12, 1.5);
        errorCard = new Border { Child = error, Padding = new(11, 9, 11, 9), CornerRadius = new(5), BorderThickness = new(1),
            BorderBrush = design.Brush("err"), Background = design.Brush("bg-sunken") }; Body.Children.Add(errorCard);
        var actions = new NativeActionWrap { Gap = 10, RowGap = 8, RightAligned = true };
        cancel = new Button { Content = design.T("settings.cancel") }; design.ActionButton(cancel); cancel.Click += (_, _) => { if (model.CanDismiss) close(); };
        create = new Button { Content = design.T("exploration.create") }; design.ActionButton(create, true);
        create.Resources["ButtonBackgroundDisabled"] = create.Resources["ButtonBorderBrushDisabled"] = design.Brush("clay");
        create.Resources["ButtonForegroundDisabled"] = design.Brush("on-clay");
        AutomationProperties.SetAutomationId(create, "exploration-create");
        create.Click += async (_, _) => { if (await model.CreateAsync() && !disposed && model.Result is { } result) completed(result); };
        actions.Children.Add(cancel); actions.Children.Add(create); Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        model.Changed += Refresh; Loaded += (_, _) => { name.Focus(FocusState.Programmatic); name.SelectAll(); }; Refresh();
    }
    private void Refresh()
    {
        if (disposed) return;
        name.IsEnabled = !model.Busy && !model.Unconfirmed; cancel.IsEnabled = model.CanDismiss; SetDismissEnabled(model.CanDismiss);
        create.IsEnabled = model.CanSubmit; create.Opacity = model.CanSubmit ? 1 : .5;
        ((TextBlock)create.Content).Text = Design.T(model.Busy ? "loading" : "exploration.create");
        error.Text = (model.Unconfirmed ? Design.L("创建结果未能确认；不会自动重试。关闭后请检查探索列表。", "Creation could not be confirmed and will not be retried. Close and check the exploration list.") + "\n" : "") + model.Error;
        errorCard.Visibility = model.Error != null || model.Unconfirmed ? Visibility.Visible : Visibility.Collapsed;
        if (errorCard.Visibility == Visibility.Visible) DispatcherQueue.TryEnqueue(() => { if (!disposed) errorCard.StartBringIntoView(); });
    }
    public override void HandleEscape() { if (model.CanDismiss) base.HandleEscape(); }
    public override void Dispose() { disposed = true; model.Changed -= Refresh; model.Dispose(); base.Dispose(); }
}
