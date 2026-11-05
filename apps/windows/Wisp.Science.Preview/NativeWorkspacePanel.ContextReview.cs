using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel
{
    private Popup? contextReviewDialog;
    private Action? dismissContextReview;

    private async Task ShowContextReviewAsync(NativeRun run, string context, Popup parent, Button trigger, Func<Task> refresh, CancellationToken token)
    {
        if (disposed || contextReviewDialog != null || XamlRoot is not { } root || !parent.IsOpen) return;
        var review = model.ContextRunReview(context, run, design.Localization);
        var overlay = new Grid { RequestedTheme = design.Dark ? ElementTheme.Dark : ElementTheme.Light };
        NativeDialogVisuals.Backdrop(overlay, design);
        var popup = contextReviewDialog = new Popup { XamlRoot = root, Child = overlay };
        var done = new TaskCompletionSource();
        string T(string key) => design.T("run_review." + key);
        AutomationProperties.SetName(overlay, T("title"));
        var stack = new StackPanel { Spacing = 12 };
        var header = new Grid { ColumnSpacing = 8 }; header.ColumnDefinitions.Add(new()); header.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var caption = new StackPanel { Spacing = 3 }; var title = design.Text(T("title"), 17);
        title.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; caption.Children.Add(title);
        var subtitle = Mute(string.IsNullOrWhiteSpace(run.Title) ? run.Id : run.Title); subtitle.MaxLines = 1;
        subtitle.TextTrimming = TextTrimming.CharacterEllipsis; ToolTipService.SetToolTip(subtitle, run.Id); caption.Children.Add(subtitle); header.Children.Add(caption);
        void Dismiss() { if (review.Confirmation != null) review.CancelConfirmation(); else if (!review.Mutating) popup.IsOpen = false; }
        dismissContextReview = Dismiss;
        var close = design.ToolButton(design.T("settings.cancel"), "close"); close.Click += (_, _) => Dismiss(); Grid.SetColumn(close, 1); header.Children.Add(close); stack.Children.Add(header);
        stack.Children.Add(Mute(T("hint")));
        var pathBar = new NativeActionWrap { Gap = 6 };
        var up = new Button(); design.ActionButton(up); up.MinHeight = 28; up.Padding = new(8, 4, 8, 4);
        var upLabel = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 4 };
        upLabel.Children.Add(design.Icon("arrow-left", 14)); upLabel.Children.Add(design.Text(T("up"), 12)); up.Content = upLabel;
        AutomationProperties.SetName(up, T("up")); up.Click += async (_, _) => await review.NavigateAsync(review.Parent, token);
        var pathText = Mute(""); pathText.FontFamily = design.Font(true); pathBar.Children.Add(up); pathBar.Children.Add(pathText); stack.Children.Add(pathBar);
        var search = new TextBox { PlaceholderText = T("filter"), CornerRadius = new(8), Padding = new(34, 7, 11, 7), MinHeight = 36,
            BorderBrush = design.Brush("border-strong"), BorderThickness = new(1), Background = design.Brush("bg-input") };
        var filter = new Grid(); AutomationProperties.SetName(search, T("filter")); filter.Children.Add(NativeDialogVisuals.Input(search, design));
        var glass = design.Icon("search", 15, design.ColorText("text-faint")); glass.Margin = new(11, 0, 0, 0); glass.HorizontalAlignment = HorizontalAlignment.Left;
        glass.IsHitTestVisible = false; filter.Children.Add(glass); stack.Children.Add(filter);
        var debounce = DispatcherQueue.CreateTimer(); debounce.Interval = TimeSpan.FromMilliseconds(180); debounce.IsRepeating = false;
        search.TextChanged += (_, _) => { review.Filter = search.Text; debounce.Stop(); if (popup.IsOpen) debounce.Start(); };
        debounce.Tick += async (_, _) => { if (popup.IsOpen && !token.IsCancellationRequested) await review.ReadAsync(token: token); };
        var notice = Mute(""); var retry = new Button { Content = design.L("重试", "Retry") }; design.ActionButton(retry);
        retry.Click += async (_, _) => await review.ReadAsync(token: token); stack.Children.Add(notice); stack.Children.Add(retry);
        var list = new StackPanel(); var listing = new ScrollViewer { Content = list, HorizontalContentAlignment = HorizontalAlignment.Stretch,
            HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
        CheckBox FileCheck(string name) {
            var box = new CheckBox { MinWidth = 20, MinHeight = 20, Padding = new(0), HorizontalAlignment = HorizontalAlignment.Center };
            box.Resources["CheckBoxSize"] = 15d; box.Resources["CheckBoxGlyphSize"] = 10d;
            AutomationProperties.SetName(box, name); return box;
        }
        var columns = new StackPanel(); var all = FileCheck(T("select_all"));
        Grid Row(FrameworkElement check, FrameworkElement name, string meta) {
            var row = new Grid { Padding = new(12, 5, 12, 5), ColumnSpacing = 10, BorderBrush = design.Brush("border"), BorderThickness = new(0, 0, 0, 1) };
            row.ColumnDefinitions.Add(new() { Width = new(28) }); row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            row.Children.Add(check); Grid.SetColumn(name, 1); row.Children.Add(name); var size = Mute(meta); size.VerticalAlignment = VerticalAlignment.Center;
            Grid.SetColumn(size, 2); row.Children.Add(size); return row;
        }
        var columnTitle = Mute(T("col_name")); columnTitle.VerticalAlignment = VerticalAlignment.Center;
        var tableHead = Row(all, columnTitle, T("col_size")); tableHead.Background = design.Brush("bg-sunken"); columns.Children.Add(tableHead); columns.Children.Add(listing);
        stack.Children.Add(new Border { Child = columns, BorderThickness = new(1), BorderBrush = design.Brush("border"), CornerRadius = new(7), Background = design.Brush("bg-elev") });
        Button ActionButton(string key, Func<Task> action, bool primary = false) {
            var b = new Button { Content = T(key) }; design.ActionButton(b, primary); b.Padding = new(14, 7, 14, 7);
            if (primary) {
                var disabledFill = new Microsoft.UI.Xaml.Media.SolidColorBrush(design.Brush("clay").Color) { Opacity = .48 };
                b.Resources["ButtonBackgroundDisabled"] = b.Resources["ButtonBorderBrushDisabled"] = disabledFill;
                b.Resources["ButtonForegroundDisabled"] = design.Brush("on-clay");
                if (b.Content is TextBlock label) { label.Foreground = design.Brush("on-clay"); label.FontWeight = Microsoft.UI.Text.FontWeights.SemiBold; }
            }
            b.Click += async (_, _) => await action(); return b;
        }
        var more = ActionButton("more", () => review.ReadAsync(true, token)); stack.Children.Add(more);
        stack.Children.Add(Mute(T("delete_warning")));
        var actions = new NativeActionWrap { Gap = 8, VerticallyCentered = true, RightAligned = true, FirstItemLeading = true };
        Task Confirm(string action) { review.BeginConfirmation(action); return Task.CompletedTask; }
        var cleanup = ActionButton("cleanup_all", () => Confirm("cleanup"));
        var selected = Mute(""); var remove = ActionButton("delete_selected", () => Confirm("delete"));
        var download = ActionButton("download_selected", () => review.DownloadAsync(token), true);
        actions.Children.Add(cleanup); actions.Children.Add(selected); actions.Children.Add(remove); actions.Children.Add(download); stack.Children.Add(actions);
        // Keep confirmation above the list and inside this Escape layer; the parent run list stays mounted.
        var confirmation = new StackPanel { Spacing = 12 }; var warning = design.Text("", 14); confirmation.Children.Add(warning);
        var confirmationActions = new NativeActionWrap { Gap = 6, RightAligned = true };
        var cancel = new Button { Content = design.T("settings.cancel") }; design.ActionButton(cancel); cancel.Click += (_, _) => review.CancelConfirmation();
        var confirm = new Button(); design.ActionButton(confirm); confirm.Click += async (_, _) => {
            var whole = review.Confirmation == "cleanup"; await review.ConfirmAsync(token);
            if (whole && review.Cleaned && popup.IsOpen) popup.IsOpen = false;
        };
        confirmationActions.Children.Add(cancel); confirmationActions.Children.Add(confirm); confirmation.Children.Add(confirmationActions);
        var body = new Grid(); body.Children.Add(stack); body.Children.Add(confirmation);
        var card = new Border { Child = new ScrollViewer { Content = body, HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled, HorizontalContentAlignment = HorizontalAlignment.Stretch },
            Padding = new(22), CornerRadius = new(14), BorderBrush = design.Brush("border-strong"), BorderThickness = new(1),
            Background = design.Brush("bg-elev"), HorizontalAlignment = HorizontalAlignment.Center, VerticalAlignment = VerticalAlignment.Center, TabFocusNavigation = KeyboardNavigationMode.Cycle };
        overlay.Children.Add(card);
        void Resize(XamlRoot sender, XamlRootChangedEventArgs? _) {
            overlay.Width = sender.Size.Width; overlay.Height = sender.Size.Height; card.Width = Math.Clamp(sender.Size.Width - 32, 1, 560);
            card.MaxHeight = Math.Max(1, sender.Size.Height - 32); listing.MaxHeight = Math.Clamp(sender.Size.Height * .46, 64, 360);
        }
        var boxes = new Dictionary<string, CheckBox>(); NativeRunWorkspaceEntry[]? rendered = null;
        void Selection() {
            foreach (var (path, box) in boxes) {
                box.IsChecked = review.Selection.ContainsKey(path); box.IsEnabled = review.CanChange;
                if (box.Parent is Grid row) row.Background = box.IsChecked == true ? design.Brush("clay-soft") : null;
            }
            all.IsChecked = review.Entries.Length > 0 && review.Entries.All(e => review.Selection.ContainsKey(e.Path));
            all.IsEnabled = review.CanChange && boxes.Count > 0;
            selected.Text = review.Selection.Count == 0 ? "" : design.Localization.Format("run_review.selected_n", ("n", review.Selection.Count.ToString()));
            selected.Visibility = review.Selection.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
            cleanup.IsEnabled = review.CanChange; remove.IsEnabled = download.IsEnabled = review.CanChange && review.Selection.Count != 0;
        }
        all.Click += (_, _) => { var check = all.IsChecked == true; foreach (var e in review.Entries) review.Select(e, check); Selection(); };
        void RenderReview() {
            if (!popup.IsOpen || token.IsCancellationRequested) return;
            var confirming = review.Confirmation != null;
            confirmation.Visibility = confirming ? Visibility.Visible : Visibility.Collapsed; stack.Visibility = confirming ? Visibility.Collapsed : Visibility.Visible;
            if (confirming) {
                var whole = review.Confirmation == "cleanup";
                warning.Text = (whole ? design.L("清理整个工作目录？未下载的文件将丢失。", "Clean the entire workspace? Files you have not downloaded will be lost.") :
                    T("delete_warning") + "\n" + string.Join("\n", review.ConfirmedPaths));
                confirm.Content = T(whole ? "cleanup_all" : "delete_selected"); cancel.Focus(FocusState.Programmatic); return;
            }
            pathBar.Visibility = review.Path.Length == 0 ? Visibility.Collapsed : Visibility.Visible; pathText.Text = "/" + review.Path;
            up.IsEnabled = !review.Mutating; close.IsEnabled = search.IsEnabled = !review.Mutating;
            notice.Text = review.Error ?? review.Status ?? ""; notice.Visibility = notice.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
            notice.Foreground = design.Brush(review.Error != null ? "err" : "text-muted");
            retry.Visibility = review.Error != null && !review.Loading && !review.Mutating ? Visibility.Visible : Visibility.Collapsed;
            more.Visibility = review.Truncated ? Visibility.Visible : Visibility.Collapsed; more.IsEnabled = !review.Mutating && !review.Loading;
            if (!ReferenceEquals(rendered, review.Entries) || review.Entries.Length == 0) {
                rendered = review.Entries; boxes.Clear(); list.Children.Clear();
                foreach (var entry in rendered) {
                    var box = FileCheck(entry.Path); boxes[entry.Path] = box;
                    box.Click += (_, _) => { review.Select(entry, box.IsChecked == true); Selection(); };
                    var dir = entry.Kind == "dir"; var name = entry.Path.Split('/').Last();
                    var label = design.Text(name, 13.5); label.TextTrimming = TextTrimming.CharacterEllipsis; label.MaxLines = 1;
                    var contents = new Grid { ColumnSpacing = 6 }; contents.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); contents.ColumnDefinitions.Add(new());
                    contents.Children.Add(design.Icon(dir ? "folder" : NativeGeneratedArtifacts.FileKind(name) == "image" ? "image" : "doc", 16, design.ColorText("text-faint"))); Grid.SetColumn(label, 1); contents.Children.Add(label);
                    var open = new Button { Content = contents, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left };
                    design.QuietButton(open); open.Padding = new(0); open.MinHeight = 24; AutomationProperties.SetName(open, entry.Path);
                    open.Click += async (_, _) => { if (dir) await review.NavigateAsync(entry.Path, token); else { review.Select(entry, !review.Selection.ContainsKey(entry.Path)); Selection(); } };
                    var meta = NativeStoragePresentation.FormatBytes(entry.SizeBytes);
                    if (dir) meta += " · " + (entry.FileCount == 1 ? T("dir_file") : design.Localization.Format("run_review.dir_files", ("n", (entry.FileCount ?? 0).ToString())));
                    list.Children.Add(Row(box, open, meta));
                }
                if (rendered.Length == 0) { var empty = Mute(review.Loading ? design.T("loading") : T("empty")); empty.Margin = new(12, 18, 12, 18); list.Children.Add(empty); }
            }
            Selection();
        }
        var esc = new KeyboardAccelerator { Key = Windows.System.VirtualKey.Escape };
        esc.Invoked += (_, e) => { if (popup.IsOpen) { Dismiss(); e.Handled = true; } }; overlay.KeyboardAccelerators.Add(esc);
        overlay.KeyboardAcceleratorPlacementMode = KeyboardAcceleratorPlacementMode.Hidden;
        Resize(root, null); root.Changed += Resize; review.Changed += RenderReview;
        void ParentClosed(object? sender, object e) => popup.IsOpen = false;
        parent.Closed += ParentClosed; popup.Closed += (_, _) => done.TrySetResult();
        try {
            popup.IsOpen = true; close.Focus(FocusState.Programmatic);
            _ = review.OpenAsync(run.Id, token); await done.Task;
        } finally {
            debounce.Stop(); root.Changed -= Resize; parent.Closed -= ParentClosed; review.Changed -= RenderReview; review.Close();
            dismissContextReview = null; contextReviewDialog = null; popup.Child = null;
            if (!disposed && !token.IsCancellationRequested && parent.IsOpen) {
                if (trigger.IsLoaded) trigger.Focus(FocusState.Programmatic);
                await refresh();
            }
        }
    }
}
