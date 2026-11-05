using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed class NativeBranchMergePage : WorkspaceSheet
{
    private readonly WorkspaceBranchMerge model;
    private readonly Action<FlyoutBase> register;
    private readonly Grid columns = new() { ColumnSpacing = 10, RowSpacing = 10 };
    private readonly StackPanel delta = new() { Spacing = 5 };
    private readonly Grid editor = new() { RowSpacing = 7 };
    private readonly TextBox draft;
    private readonly TextBlock error, meta, loading;
    private readonly Button cancel, regenerate, guided, merge;
    private readonly Border changes;
    private NativeBranchPreview? rendered;
    private NativeBranchGuidancePage? guidance;
    private bool disposed, syncing;
    public NativeBranchMergePage(WorkspaceBranchMerge model, WispDesign design, Action<NativeBranchMergeResult> merged, Action close, Action<FlyoutBase> register)
        : base(design, design.T("branch.merge_title"), close, modal: true, modalWidth: 1100)
    {
        this.model = model; this.register = register;
        Body.Spacing = 12;
        var hint = design.Text(design.T("branch.merge_hint"), 12); hint.Foreground = design.Brush("text-faint"); Body.Children.Add(hint);
        error = design.Text("", 12); error.Foreground = design.Brush("err"); Notices.Children.Add(error);
        meta = design.Text("", 11); meta.Foreground = design.Brush("text-faint"); Body.Children.Add(meta);
        loading = design.Text(design.T("loading"), 12); loading.Margin = new(0, 40, 0, 40); Body.Children.Add(loading);
        var left = new Grid { RowSpacing = 7 }; left.RowDefinitions.Add(new() { Height = GridLength.Auto }); left.RowDefinitions.Add(new());
        left.Children.Add(Heading("branch.branch_work")); var messages = new ScrollViewer { Content = delta, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled }; Grid.SetRow(messages, 1); left.Children.Add(messages);
        changes = new Border { Child = left, Padding = new(9), BorderThickness = new(1), CornerRadius = new(8), BorderBrush = design.Brush("border"), Background = design.Brush("bg-sunken") };
        editor.RowDefinitions.Add(new() { Height = GridLength.Auto }); editor.RowDefinitions.Add(new() { Height = GridLength.Auto }); editor.RowDefinitions.Add(new());
        editor.Children.Add(Heading("branch.summary_draft"));
        var editHint = design.Text(design.T("branch.summary_edit_hint"), 11); editHint.Foreground = design.Brush("text-faint"); Grid.SetRow(editHint, 1); editor.Children.Add(editHint);
        draft = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 220, Padding = new(11), CornerRadius = new(10) };
        design.BindTypography(draft, size: 14); PreferenceFieldAppearance.Apply(draft, design);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(draft, design.T("branch.summary_draft"));
        var menu = new TextCommandBarFlyout(); draft.ContextFlyout = menu; register(menu);
        draft.TextChanged += (_, _) => { if (!syncing) { model.Draft = draft.Text.ReplaceLineEndings("\n"); RefreshButtons(); } }; Grid.SetRow(draft, 2); editor.Children.Add(draft);
        columns.ColumnDefinitions.Add(new()); columns.ColumnDefinitions.Add(new()); columns.RowDefinitions.Add(new()); columns.RowDefinitions.Add(new() { Height = GridLength.Auto });
        columns.Children.Add(changes); columns.Children.Add(editor); Body.Children.Add(columns);
        cancel = Action("settings.cancel", HandleEscape);
        regenerate = Action("branch.regenerate", async () => await model.GenerateAsync());
        guided = Action("branch.guided_generate", OpenGuidance);
        merge = Action("branch.merge", async () => { if (await model.MergeAsync() && !disposed && model.Result is { } result) merged(result); }, true);
        var leftActions = new NativeActionWrap { Gap = 8 }; leftActions.Children.Add(cancel); leftActions.Children.Add(regenerate); leftActions.Children.Add(guided);
        var actions = new Grid { ColumnSpacing = 12 }; actions.ColumnDefinitions.Add(new()); actions.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); actions.Children.Add(leftActions); Grid.SetColumn(merge, 1); actions.Children.Add(merge);
        Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        SizeChanged += (_, _) => Layout(); model.Changed += Refresh; Refresh();
    }
    private TextBlock Heading(string key) { var text = Design.Text(Design.T(key), 12); text.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; return text; }
    private Button Action(string key, Action action, bool primary = false)
    {
        var text = Design.Text(Design.T(key), 13); var button = new Button { Content = text }; Design.ActionButton(button, primary);
        if (primary) { text.Foreground = Design.Brush("on-clay"); button.Resources["ButtonBackgroundDisabled"] = Design.Brush("clay"); button.Resources["ButtonBorderBrushDisabled"] = Design.Brush("clay"); }
        button.Click += (_, _) => action(); return button;
    }
    private void Layout()
    {
        var compact = ActualWidth <= 640;
        columns.ColumnDefinitions[1].Width = compact ? new(0) : new(1, GridUnitType.Star);
        Grid.SetColumn(editor, compact ? 0 : 1); Grid.SetRow(editor, compact ? 1 : 0);
        var height = Math.Max(250, Math.Min(460, ActualHeight - 280)); changes.Height = compact ? 240 : height;
        editor.Height = compact ? 320 : height; columns.Height = double.NaN;
    }
    private void RefreshButtons()
    {
        cancel.IsEnabled = model.CanDismiss; SetDismissEnabled(model.CanDismiss);
        regenerate.IsEnabled = model.CanGenerate; guided.IsEnabled = model.CanGenerate && !string.IsNullOrWhiteSpace(model.Draft); merge.IsEnabled = model.CanMerge;
        merge.Opacity = merge.IsEnabled ? 1 : .5;
        ((TextBlock)merge.Content).Text = Design.T(model.Merging ? "branch.merging" : "branch.merge");
        ((TextBlock)regenerate.Content).Text = Design.T(model.Generating ? "branch.generating" : "branch.regenerate");
    }
    private void Refresh()
    {
        if (disposed) return;
        error.Text = (model.MutationUnconfirmed ? Design.L("回并结果未能确认，不会自动重试。关闭后请检查 main 和分支状态。", "The merge result could not be confirmed and will not be retried. Close and check main and the branch state.") + "\n" : "") + model.Error;
        Notices.Visibility = model.Error == null ? Visibility.Collapsed : Visibility.Visible;
        loading.Visibility = model.Preview == null ? Visibility.Visible : Visibility.Collapsed;
        columns.Visibility = meta.Visibility = model.Preview == null ? Visibility.Collapsed : Visibility.Visible;
        if (model.Preview is { } preview && !ReferenceEquals(rendered, preview))
        {
            rendered = preview; delta.Children.Clear();
            meta.Text = preview.BranchTitle + "   " + Design.Localization.Format("branch.checkpoint", ("n", ((long)preview.CheckpointUserIndex + 1).ToString())) + "   " + Design.Localization.Format("branch.new_messages", ("n", preview.NewMessageCount.ToString()));
            foreach (var message in preview.Messages)
            {
                var body = new StackPanel { Spacing = 3 }; var role = message.Role is "user" or "assistant" or "tool" ? message.Role : "system";
                var label = Design.Text(Design.T("branch.role_" + role), 9); label.Foreground = Design.Brush("text-faint"); body.Children.Add(label);
                var text = Design.Text(message.Text, 11); text.Foreground = Design.Brush("text-muted"); text.IsTextSelectionEnabled = true; body.Children.Add(text);
                delta.Children.Add(new Border { Child = body, Padding = new(8, 7, 8, 7), CornerRadius = new(5), Background = Design.Brush(role == "user" ? "clay-soft" : "bg-elev") });
            }
        }
        if (draft.Text.ReplaceLineEndings("\n") != model.Draft) { syncing = true; draft.Text = model.Draft; syncing = false; }
        RefreshButtons(); Layout();
    }
    private void OpenGuidance()
    {
        if (!model.CanGenerate || guidance != null || string.IsNullOrWhiteSpace(model.Draft)) return;
        guidance = new NativeBranchGuidancePage(Design, async text => { CloseGuidance(); await model.GenerateAsync(text); }, CloseGuidance, register);
        var overlay = (Grid)Content;
        foreach (var layer in overlay.Children) layer.IsHitTestVisible = false;
        guidance.Translation = new System.Numerics.Vector3(0, 0, 48);
        overlay.Children.Add(guidance);
    }
    private void CloseGuidance()
    {
        if (guidance == null) return;
        var overlay = (Grid)Content;
        overlay.Children.Remove(guidance); guidance.Dispose(); guidance = null;
        foreach (var layer in overlay.Children) layer.IsHitTestVisible = true;
        guided.Focus(FocusState.Programmatic);
    }
    public override void HandleEscape() { if (guidance != null) CloseGuidance(); else if (model.CanDismiss) base.HandleEscape(); }
    public override void Dispose() { disposed = true; if (guidance != null) CloseGuidance(); model.Changed -= Refresh; model.Dispose(); base.Dispose(); }
}

internal sealed class NativeBranchGuidancePage : WorkspaceSheet
{
    public NativeBranchGuidancePage(WispDesign design, Action<string> generate, Action close, Action<FlyoutBase> register)
        : base(design, design.T("branch.guidance_title"), close, modal: true, modalWidth: 560)
    {
        var hint = design.Text(design.T("branch.guidance_hint"), 12); hint.Foreground = design.Brush("text-faint"); Body.Children.Add(hint);
        Body.Children.Add(design.Text(design.T("branch.guidance_label"), 12));
        var guidance = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 150, Height = 180, MaxLength = 8000, PlaceholderText = design.T("branch.guidance_placeholder"), Padding = new(11), CornerRadius = new(10) };
        design.BindTypography(guidance, size: 14); PreferenceFieldAppearance.Apply(guidance, design); Body.Children.Add(guidance);
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetName(guidance, design.T("branch.guidance_label"));
        var menu = new TextCommandBarFlyout(); guidance.ContextFlyout = menu; register(menu);
        var cancel = new Button { Content = design.Text(design.T("settings.cancel"), 13) }; design.ActionButton(cancel); cancel.Click += (_, _) => close();
        var submit = new Button { Content = design.Text(design.T("branch.generate"), 13), IsEnabled = false, Opacity = .5 }; design.ActionButton(submit, true); ((TextBlock)submit.Content).Foreground = design.Brush("on-clay");
        submit.Resources["ButtonBackgroundDisabled"] = design.Brush("clay"); submit.Resources["ButtonBorderBrushDisabled"] = design.Brush("clay");
        guidance.TextChanged += (_, _) => { submit.IsEnabled = !string.IsNullOrWhiteSpace(guidance.Text); submit.Opacity = submit.IsEnabled ? 1 : .5; };
        submit.Click += (_, _) => { if (submit.IsEnabled) generate(guidance.Text.ReplaceLineEndings("\n")); };
        var actions = new NativeActionWrap { RightAligned = true }; actions.Children.Add(cancel); actions.Children.Add(submit); Footer.Children.Add(actions); Footer.Visibility = Visibility.Visible;
        Loaded += (_, _) => guidance.Focus(FocusState.Programmatic);
    }
}
