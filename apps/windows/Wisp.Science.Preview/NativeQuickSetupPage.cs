using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Windows.System;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed class NativeQuickSetupPage : WorkspaceSheet
{
    private readonly NativeQuickSetupModel model;
    private readonly Action openConsole, done;
    private readonly TextBlock title;
    private readonly ContentControl pageHeading = new() { IsTabStop = true, UseSystemFocusVisuals = false,
        HorizontalContentAlignment = HorizontalAlignment.Stretch };
    private readonly StackPanel page = new() { Spacing = 8 };
    private readonly StackPanel dots = new() { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
    private readonly Button back, next;
    private readonly PasswordBox key = new() { PasswordRevealMode = PasswordRevealMode.Hidden, CornerRadius = new(8),
        HorizontalAlignment = HorizontalAlignment.Stretch, MinHeight = 36 };
    private int rendered = -1;
    private bool closing;
    internal bool CanLeave => !model.Busy && !model.Environment.Busy && !closing;
    private string T(string label) => Design.T(label);

    public NativeQuickSetupPage(NativeQuickSetupModel model, WispDesign design, Action openConsole, Action done)
        : base(design, design.T("command.setup"), done, modal: true, modalWidth: 520)
    {
        this.model = model; this.openConsole = openConsole; this.done = done;
        AutomationProperties.SetAutomationId(this, "quick-setup");
        title = design.Text("", 17); title.TextAlignment = TextAlignment.Center; title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold;
        pageHeading.Content = title; UseScrollingHeader(pageHeading);
        Body.Padding = new Thickness(22, 20, 22, 0); Body.Children.Add(page);
        BodyScroll.VerticalScrollBarVisibility = ScrollBarVisibility.Hidden;
        Footer.Visibility = Visibility.Visible; Footer.Padding = new Thickness(22, 14, 22, 22); Footer.Spacing = 18;
        Footer.Children.Add(dots);
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 10, HorizontalAlignment = HorizontalAlignment.Right };
        back = Action(T("onboard.back"), model.Back);
        next = Action("", () => _ = AdvanceAsync(), primary: true);
        actions.Children.Add(back); actions.Children.Add(next); Footer.Children.Add(actions);
        Design.BindTypography(key, size: 14); key.Background = Design.Brush("bg-input"); key.BorderBrush = Design.Brush("border-strong");
        AutomationProperties.SetName(key, T("settings.api_key"));
        key.PasswordChanged += (_, _) => SetNextLabel();
        PreviewKeyDown += (_, e) =>
        {
            if (e.Key != VirtualKey.Enter || Modified() || model.Busy) return;
            var focused = FocusManager.GetFocusedElement(XamlRoot);
            if (!ReferenceEquals(focused, pageHeading) && !ReferenceEquals(focused, key) && !ReferenceEquals(focused, next)) return;
            e.Handled = true;
            if (!e.KeyStatus.WasKeyDown) _ = AdvanceAsync();
        };
        model.Changed += Render; model.Environment.Changed += EnvironmentChanged;
        Loaded += (_, _) => pageHeading.Focus(FocusState.Programmatic);
        Render();
    }
    private static bool Modified() => new[] { VirtualKey.Control, VirtualKey.Menu, VirtualKey.Shift, VirtualKey.LeftWindows, VirtualKey.RightWindows }
        .Any(v => (Microsoft.UI.Input.InputKeyboardSource.GetKeyStateForCurrentThread(v) & Windows.UI.Core.CoreVirtualKeyStates.Down) != 0);
    private Button Action(string label, Action click, bool primary = false)
    {
        var button = new Button { Content = label }; Design.ActionButton(button, primary);
        button.Click += (_, _) => click(); return button;
    }
    private async Task AdvanceAsync()
    {
        if (model.Busy || model.Closed || closing) return;
        var secret = model.Step == 2 ? key.Password : "";
        if (model.Step == 2) key.Password = "";
        await model.AdvanceAsync(secret);
    }
    public override void HandleEscape()
    {
        if (model.Busy || closing) return;
        if (model.Step > 0) model.Back(); else Finish();
    }
    private void Finish()
    {
        if (closing) return;
        closing = true; done();
    }
    private void SetNextLabel() => next.Content = T(model.Step switch
    {
        3 => "onboard.start", 2 when string.IsNullOrWhiteSpace(key.Password) => "onboard.apikey.later", _ => "onboard.next"
    });
    private void Render()
    {
        if (model.Closed || closing) return;
        if (model.Finished) { Finish(); return; }
        key.IsEnabled = back.IsEnabled = next.IsEnabled = !model.Busy;
        back.Visibility = model.Step > 0 ? Visibility.Visible : Visibility.Collapsed; SetNextLabel();
        Notices.Children.Clear(); Notices.Visibility = Visibility.Collapsed;
        if (model.Error != null) { Notices.Visibility = Visibility.Visible; Notices.Children.Add(Warn(model.Error)); }
        if (rendered == model.Step) return;
        rendered = model.Step; page.Children.Clear(); dots.Children.Clear();
        // The PasswordBox can outlive its page so Back retains an unfinished entry;
        // detach it before mounting it under a new parent.
        if (key.Parent is Panel parent) parent.Children.Remove(key);
        for (int i = 0; i < 4; i++) dots.Children.Add(new Border { Width = 7, Height = 7,
            Background = Design.Brush(i == rendered ? "clay" : "border-strong"), CornerRadius = new(4) });
        title.Text = T(rendered switch { 0 => "onboard.welcome.title", 1 => "onboard.features.title", 2 => "onboard.apikey.title", _ => "env.title" });
        AutomationProperties.SetName(this, title.Text);
        AutomationProperties.SetName(pageHeading, title.Text);
        if (rendered <= 1)
        {
            var intro = Hint(T(rendered == 0 ? "onboard.welcome.body" : "onboard.features.body"));
            intro.TextAlignment = TextAlignment.Center; intro.Margin = new Thickness(0, 10, 0, 12); page.Children.Add(intro);
        }
        else if (rendered == 2)
        {
            var getKey = new HyperlinkButton { Content = T("onboard.apikey.get_key"), Padding = new Thickness(0), MinHeight = 0,
                Foreground = Design.Brush("text-muted"), HorizontalAlignment = HorizontalAlignment.Left };
            getKey.Click += (_, _) => openConsole(); Design.BindTypography(getKey, size: 12.5);
            AddStep(1, T("onboard.getkey.body"), getKey);
            var fields = new StackPanel { Spacing = 4 }; fields.Children.Add(Design.Text(T("settings.api_key"), 12, "text-muted"));
            fields.Children.Add(key); AddStep(2, T("onboard.apikey.body"), fields);
        }
        else RenderEnvironment();
        BodyScroll.ChangeView(null, 0, null, true);
        if (IsLoaded) pageHeading.Focus(FocusState.Programmatic);
    }
    private TextBlock Hint(string text, double size = 13.5)
    {
        var hint = Design.Text(text, size, "text-muted"); hint.LineHeight = Design.Typography.Scale(size) * 1.55; return hint;
    }
    private void AddStep(int number, string body, FrameworkElement control)
    {
        var row = new Grid { ColumnSpacing = 4, Margin = new Thickness(6, 6, 0, 0) };
        row.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); row.ColumnDefinitions.Add(new());
        row.Children.Add(Hint(number + ".", 13)); var stack = new StackPanel { Spacing = 5 };
        stack.Children.Add(Hint(body)); stack.Children.Add(control); Grid.SetColumn(stack, 1); row.Children.Add(stack); page.Children.Add(row);
    }
    private void EnvironmentChanged() { if (rendered == 3 && !model.Closed && !closing) RenderEnvironment(); }
    private void RenderEnvironment() => PopulateLocalEnvironment(page, model.Environment);
    public override void Dispose()
    {
        model.Changed -= Render; model.Environment.Changed -= EnvironmentChanged; key.Password = "";
        model.Dispose(); base.Dispose();
    }
}
