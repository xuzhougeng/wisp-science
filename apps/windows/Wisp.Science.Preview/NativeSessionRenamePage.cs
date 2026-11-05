using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeSessionRenamePage : WorkspaceSheet
{
    private readonly WorkspaceSessionRename model;
    private readonly TextBox name;
    private readonly Button save, cancel;
    private readonly TextBlock error;
    private bool disposed;
    public NativeSessionRenamePage(WorkspaceSessionRename model, WispDesign design, Action saved, Action close, Action<FlyoutBase> register)
        : base(design, design.T("session.rename_title"), close, modal: true, modalWidth: 480)
    {
        this.model = model;
        HideDismissButton();
        name = new TextBox { Text = model.Title, MinHeight = 36, Padding = new Thickness(11, 8, 11, 8), CornerRadius = new CornerRadius(10) };
        design.BindTypography(name, size: 14); PreferenceFieldAppearance.Apply(name, design);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(name, design.T("session.rename_title"));
        var menu = new TextCommandBarFlyout(); name.ContextFlyout = menu; register(menu); Body.Children.Add(name);
        error = design.Text("", 13); error.Foreground = design.Brush("err"); Body.Children.Add(error);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Spacing = 8 };
        cancel = new Button { Content = design.Text(design.T("settings.cancel"), 14) }; design.ActionButton(cancel, false); cancel.Click += (_, _) => HandleEscape(); actions.Children.Add(cancel);
        save = new Button { Content = design.Text(design.T("settings.save"), 14) }; design.ActionButton(save, true); ((TextBlock)save.Content).Foreground = design.Brush("on-clay"); actions.Children.Add(save);
        async Task Save() { if (await model.SaveAsync() && !disposed) saved(); }
        save.Click += async (_, _) => await Save(); name.KeyDown += async (_, e) => { if (e.Key == Windows.System.VirtualKey.Enter) { e.Handled = true; await Save(); } };
        name.TextChanged += (_, _) => { model.Title = name.Text; Refresh(); };
        Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        model.Changed += Refresh; Refresh();
        Loaded += (_, _) => { name.Focus(FocusState.Programmatic); name.SelectAll(); };
    }
    private void Refresh()
    {
        if (disposed) return;
        save.IsEnabled = model.CanSave; save.Opacity = model.CanSave ? 1 : .5; name.IsReadOnly = model.Busy; cancel.IsEnabled = !model.Busy;
        error.Text = model.Error == null ? "" : (model.MutationUnconfirmed ? Design.L("结果未能确认；不会自动重试。请关闭后刷新会话列表。", "The result could not be confirmed and will not be retried. Close and refresh the conversation list.") + "\n" : "") + model.Error;
        error.Visibility = model.Error == null ? Visibility.Collapsed : Visibility.Visible;
    }
    public override void HandleEscape() { if (!model.Busy) base.HandleEscape(); }
    public override void Dispose() { disposed = true; model.Changed -= Refresh; model.Dispose(); base.Dispose(); }
}
