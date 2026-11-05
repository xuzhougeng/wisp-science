using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeSessionRulesPage : WorkspaceSheet
{
    private readonly WorkspaceSessionRules model;
    private readonly Button reload, cancel;
    private readonly TextBlock error;
    private bool disposed;
    public NativeSessionRulesPage(WorkspaceSessionRules model, WispDesign design, Action saved, Action close)
        : base(design, design.T("confirm.title"), close, modal: true, modalWidth: 480)
    {
        this.model = model;
        HideDismissButton();
        var hint = design.Text(design.T("session.reload_rules_hint"), 13); design.BindLineHeight(hint, 13, 1.55); hint.Foreground = design.Brush("text-muted"); Body.Children.Add(hint);
        error = design.Text("", 13); error.Foreground = design.Brush("err"); Body.Children.Add(error);
        var actions = new NativeActionWrap { Gap = 10, RowGap = 8, HorizontalAlignment = HorizontalAlignment.Right };
        cancel = new Button { Content = design.T("settings.cancel") }; design.BindTypography(cancel, size: 13.5); design.ActionButton(cancel, false); cancel.Click += (_, _) => HandleEscape(); actions.Children.Add(cancel);
        reload = new Button { Content = design.T("session.reload_rules_action") }; design.BindTypography(reload, size: 13.5); design.ActionButton(reload, true); actions.Children.Add(reload);
        reload.Click += async (_, _) => { if (await model.ReloadAsync() && !disposed) saved(); };
        Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        model.Changed += Refresh; Refresh();
    }
    private void Refresh()
    {
        if (disposed) return;
        reload.IsEnabled = model.CanReload; reload.Opacity = model.CanReload ? 1 : .5; cancel.IsEnabled = !model.Busy;
        error.Text = model.Error == null ? "" : (model.MutationUnconfirmed
            ? Design.L("结果未能确认；不会自动重试。请关闭后重新打开会话菜单检查规则状态。", "The result could not be confirmed and will not be retried. Close and reopen the conversation menu to check the rules status.") + "\n" : "") + model.Error;
        error.Visibility = model.Error == null ? Visibility.Collapsed : Visibility.Visible;
        if (model.Error != null) DispatcherQueue.TryEnqueue(() =>
        {
            if (disposed) return;
            BodyScroll.UpdateLayout();
            BodyScroll.ChangeView(null, BodyScroll.ScrollableHeight, null, disableAnimation: true);
        });
    }
    public override void HandleEscape() { if (!model.Busy) base.HandleEscape(); }
    public override void Dispose() { disposed = true; model.Changed -= Refresh; model.Dispose(); base.Dispose(); }
}
