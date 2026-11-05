using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Automation;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeExampleCopyPage : WorkspaceSheet
{
    private readonly NativeExampleCopy model;
    private readonly ComboBox target = new() { HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly Button confirm, cancel;
    private readonly TextBlock error;
    public NativeExampleCopyPage(NativeExampleCopy model, WispDesign design, Action<string, string> copied, Action close)
        : base(design, design.T("session.copy_demo_title"), close, modal: true, modalWidth: 520)
    {
        this.model = model; HideDismissButton(); Body.Spacing = 12;
        var hint = design.Text(design.T("session.copy_demo_hint").Replace("{title}", model.Source.Title), 13); hint.Foreground = design.Brush("text-muted"); Body.Children.Add(hint);
        var field = new StackPanel { Spacing = 6 }; field.Children.Add(design.Text(design.T("session.target_project"), 13));
        foreach (var project in model.Targets) target.Items.Add(new ComboBoxItem { Content = project.Name, Tag = project.Id });
        target.SelectedIndex = target.Items.Count > 0 ? 0 : -1; design.BindTypography(target, size: 14); PreferencePickerAppearance.Apply(target, design);
        AutomationProperties.SetName(target, design.T("session.target_project")); field.Children.Add(target); Body.Children.Add(field);
        error = design.Text("", 13); error.Foreground = design.Brush("err"); Body.Children.Add(error);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Right };
        cancel = new Button { Content = design.T("settings.cancel") }; design.ActionButton(cancel); cancel.Click += (_, _) => { if (!model.Busy) close(); };
        confirm = new Button { Content = design.T("session.copy_action") }; design.ActionButton(confirm, true);
        confirm.Click += async (_, _) => { if (target.SelectedItem is ComboBoxItem { Tag: string id } && await model.CopyAsync(id) && model.Result is { } session) copied(id, session); };
        actions.Children.Add(cancel); actions.Children.Add(confirm); Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        model.Changed += Refresh; Refresh();
    }
    private void Refresh()
    {
        error.Text = model.Targets.Length == 0 ? Design.T("session.no_target_project") : (model.Unconfirmed ? Design.L("结果未能确认；请关闭后检查目标项目，不会自动重试。\n", "The result is unconfirmed. Close and check the target project; this will not be retried automatically.\n") : "") + model.Error;
        error.Visibility = string.IsNullOrEmpty(error.Text) ? Visibility.Collapsed : Visibility.Visible;
        confirm.IsEnabled = !model.Busy && !model.Unconfirmed && model.Result == null && model.Targets.Length > 0;
        cancel.IsEnabled = !model.Busy; target.IsEnabled = !model.Busy && !model.Unconfirmed;
    }
    public override void HandleEscape() { if (target.IsDropDownOpen) { target.IsDropDownOpen = false; return; } if (!model.Busy) base.HandleEscape(); }
    public override void Dispose() { target.IsDropDownOpen = false; model.Changed -= Refresh; model.Dispose(); base.Dispose(); }
}
