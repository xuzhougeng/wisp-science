using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Windows.ApplicationModel.DataTransfer;
using Windows.Storage;
using System.Text.Json;
using Wisp.ProjectBrowser;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.Science.Preview;

internal sealed partial class NativeWorkspacePanel
{
    private readonly StackPanel fileHeader = new() { Spacing = 10 };
    private readonly Func<Task<IReadOnlyList<string>>>? pickFiles;
    private readonly Func<string, Task<string?>>? pickDownload;
    private readonly Microsoft.UI.Dispatching.DispatcherQueueTimer transferTimer;
    private bool readingTransfers;
    private readonly StackPanel transferStatus = new() { Spacing = 4 };
    private readonly StackPanel fileRows = new() { Spacing = 2 };
    private Button? uploadButton;
    private readonly List<Button> downloadButtons = [];
    private string fileSort = "name";
    private bool pickingFiles;
    private string? fileCopyNotice;

    private void RenderFiles(string query)
    {
        uploadButton = null;
        if (model.FileLocations is { } locations)
        {
            var source = new Grid { ColumnSpacing = 10 };
            source.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            source.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            var label = Mute(design.T("files.source")); label.VerticalAlignment = VerticalAlignment.Center; source.Children.Add(label);
            var selected = locations.Locations.FirstOrDefault(l => l.ContextId == model.FileContext);
            var sourceButton = design.ToolButton(FileLocationLabel(selected), "chevron-down", true);
            var sourceContent = new Grid { ColumnSpacing = 12 };
            sourceContent.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
            sourceContent.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var sourceName = design.Text(FileLocationLabel(selected), 12); sourceName.TextWrapping = TextWrapping.NoWrap;
            sourceName.TextTrimming = TextTrimming.CharacterEllipsis; sourceContent.Children.Add(sourceName);
            var arrow = design.Icon("chevron-down", 14); Grid.SetColumn(arrow, 1); sourceContent.Children.Add(arrow);
            sourceButton.Content = sourceContent; sourceButton.HorizontalContentAlignment = HorizontalAlignment.Stretch;
            sourceButton.BorderThickness = new Thickness(1); sourceButton.BorderBrush = design.Brush("border");
            sourceButton.HorizontalAlignment = HorizontalAlignment.Stretch;
            sourceButton.IsEnabled = !model.FileTransferBusy && !model.FileActionBusy && !pickingFiles;
            AutomationProperties.SetName(sourceButton, design.T("files.source") + ": " + FileLocationLabel(selected));
            var menu = new MenuFlyout();
            foreach (var item in locations.Locations)
            {
                var location = item;
                var option = new ToggleMenuFlyoutItem { Text = FileLocationLabel(location), IsChecked = location.ContextId == model.FileContext };
                option.Click += async (_, _) => {
                    fileSort = "name"; await FileReadAsync(model.SelectFileLocationAsync(location.ContextId, lifetime.Token));
                };
                menu.Items.Add(option);
            }
            TrackMenu(menu); sourceButton.Flyout = menu;
            Grid.SetColumn(sourceButton, 1); source.Children.Add(sourceButton); fileHeader.Children.Add(source);
        }
        var crumb = new Grid { ColumnSpacing = 6 };
        crumb.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        crumb.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) });
        var up = FileButton(design.T("files.up"), "up");
        up.IsEnabled = !model.Loading && !model.FileTransferBusy && model.Path is not ("." or "/" or "~");
        up.Click += async (_, _) => await NavigateFilesAsync(model.Parent); crumb.Children.Add(up);
        FrameworkElement path;
        if (model.RemoteFiles)
        {
            var input = new TextBox { Text = model.Path, FontSize = design.FontSize(12), IsEnabled = !model.Loading && !model.FileTransferBusy };
            AutomationProperties.SetName(input, design.T("files.go_to"));
            input.KeyDown += async (_, e) => {
                if (e.Key == Windows.System.VirtualKey.Enter && !string.IsNullOrWhiteSpace(input.Text)) {
                    e.Handled = true; await NavigateFilesAsync(input.Text.Trim());
                }
            };
            path = input;
        }
        else
        {
            var text = Mute(model.Path == "." ? model.FileLocations?.LocalRoot ?? design.T("files.local_project") : model.Path);
            text.TextWrapping = TextWrapping.NoWrap; text.TextTrimming = TextTrimming.CharacterEllipsis;
            text.VerticalAlignment = VerticalAlignment.Center; ToolTipService.SetToolTip(text, text.Text); path = text;
        }
        Grid.SetColumn(path, 1); crumb.Children.Add(path); fileHeader.Children.Add(crumb);
        var actions = new NativeActionWrap { Gap = 6 };
        if (pickFiles != null)
        {
            var upload = FileButton(design.T(model.FileTransferBusy ? "files.uploading" : "files.upload"), "upload", true);
            uploadButton = upload; upload.IsEnabled = model.CanUploadFiles && !pickingFiles;
            upload.Click += async (_, _) => await PickAndUploadAsync(); actions.Children.Add(upload);
        }
        if (!model.RemoteFiles)
            foreach (var (key, icon, action) in new[] { ("files.new_file", "doc", NativePanelFileAction.CreateFile), ("files.new_directory", "folder", NativePanelFileAction.CreateDirectory) })
            {
                var create = FileButton(design.T(key), icon, true); create.IsEnabled = model.CanMutateLocalFiles;
                create.Click += async (_, _) => await PromptFileAsync(action); actions.Children.Add(create);
            }
        var refresh = FileButton(design.T("files.refresh"), "sync", true); refresh.IsEnabled = !model.Loading && !model.FileTransferBusy;
        refresh.Click += async (_, _) => await FileReadAsync(model.RefreshAsync("files", cancellationToken: lifetime.Token)); actions.Children.Add(refresh);
        if (!model.RemoteFiles && model.RecursiveFileSearch)
        {
            var select = FileButton(design.T(model.SelectingFiles ? "settings.cancel" : "files.select_entries"), model.SelectingFiles ? "close" : "check", true);
            select.IsEnabled = model.CanSelectFiles;
            select.Click += (_, _) => { model.ToggleFileSelectionMode(); fileCopyNotice = null; Render(); };
            actions.Children.Add(select);
        }
        var sortButton = FileButton(design.T("files.sort"), "sort", true);
        sortButton.IsEnabled = !model.SearchingFiles;
        var sortMenu = new MenuFlyout();
        foreach (var sort in new[] { "name", "size", "modified" })
        {
            var item = new ToggleMenuFlyoutItem { Text = design.T("files.sort_" + sort), IsChecked = fileSort == sort };
            item.Click += (_, _) => { fileSort = sort; Render(); }; sortMenu.Items.Add(item);
        }
        TrackMenu(sortMenu); sortButton.Flyout = sortMenu; actions.Children.Add(sortButton);
        fileHeader.Children.Add(actions);
        if (model.SelectingFiles && !model.RemoteFiles)
        {
            var selection = new NativeActionWrap { Gap = 6 };
            var all = new Button { Content = design.T("sidebar.select_all"), IsEnabled = model.CanSelectFiles };
            all.Click += (_, _) => { model.SelectAllFiles(); fileCopyNotice = null; Render(); }; selection.Children.Add(all);
            var copy = FileButton(design.T("files.copy_relative_path"), "copy");
            var absolute = FileButton(design.T("files.copy_absolute_path"), "clipboard");
            copy.IsEnabled = absolute.IsEnabled = model.CanSelectFiles && !model.FileCopyBusy && model.SelectedFiles.Count > 0;
            copy.Click += async (_, _) => await CopyFilePathsAsync(false);
            absolute.Click += async (_, _) => await CopyFilePathsAsync(true);
            selection.Children.Add(copy); selection.Children.Add(absolute);
            selection.Children.Add(Mute(design.L($"已选择 {model.SelectedFiles.Count} 项", $"{model.SelectedFiles.Count} selected")));
            fileHeader.Children.Add(selection);
        }
        if (fileCopyNotice != null) fileHeader.Children.Add(Mute(fileCopyNotice));
        if (model.FileLocations?.ReadOnly == true)
            fileHeader.Children.Add(Mute(design.L("只读会话", "Read-only conversation")));
        else if (pickFiles != null)
            fileHeader.Children.Add(Mute(model.RemoteFiles
                ? design.T("files.remote_upload_hint")
                : design.T("files.local_upload_hint")));

        RenderFileTransferStatus(); body.Children.Add(transferStatus);
        RenderFileRows(query); body.Children.Add(fileRows);
    }
    private void RenderFileTransferStatus()
    {
        transferStatus.Children.Clear();
        if (model.FileTransfer is { } transfer)
        {
            foreach (var item in transfer.Items)
            {
                var status = item.Status switch {
                    "succeeded" => design.T(model.FileTransferIsDownload ? "transfer.downloaded" : "transfer.uploaded"),
                    "failed" or "timed_out" or "lost" => design.T("transfer.failed"), "cancelled" => design.T("transfer.cancelled"),
                    _ => design.T(model.FileTransferIsDownload ? "transfer.downloading" : "transfer.uploading") };
                var message = Mute(status + " · " + System.IO.Path.GetFileName(item.SourcePath) + (item.Error is { } error ? "\n" + error : ""));
                if (item.Status is "failed" or "timed_out" or "lost") message.Foreground = design.Brush("err");
                transferStatus.Children.Add(message);
                if (!WorkspaceConversationModel.RunTerminal(item.Status))
                {
                    var progress = item.RunId != null && model.FileTransferRuns.TryGetValue(item.RunId, out var run) ? NativeRunPresentation.Progress(run, design.Localization) : null;
                    transferStatus.Children.Add(new ProgressBar { Minimum = 0, Maximum = 100, Value = progress?.Percent ?? 0,
                        IsIndeterminate = progress?.Percent == null, Height = 3, Margin = new Thickness(0, 2, 0, 8) });
                }
            }
        }
        if (model.FileTransferReadError is { } transferError) transferStatus.Children.Add(Mute(design.L("无法更新传输状态，请刷新。", "Could not update transfer status. Refresh to check.") + "\n" + transferError));
    }
    private void RenderFileRows(string query)
    {
        fileRows.Children.Clear(); downloadButtons.Clear();
        if (model.FileSearchLoading) fileRows.Children.Add(new ProgressBar { IsIndeterminate = true, Height = 3 });
        if (model.FileSearchError is { } searchError) fileRows.Children.Add(Mute(searchError));
        var entries = model.SearchingFiles && model.RecursiveFileSearch
            ? model.FileSearchHits.Select(h => (Entry: new NativePanelFile(h.Name, h.IsDir, h.Size, null), Full: h.Path)).ToArray()
            : NativeFileListPresentation.VisibleFiles(model.Files, query, fileSort).Select(f => (Entry: f, Full: WorkspacePanelModel.Child(model.Path, f.Name))).ToArray();
        foreach (var (entry, full) in entries)
        {
            var captured = entry;
            var row = new Grid(); row.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); row.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            var content = new Grid { ColumnSpacing = 10 };
            content.ColumnDefinitions.Add(new() { Width = GridLength.Auto }); content.ColumnDefinitions.Add(new() { Width = new GridLength(1, GridUnitType.Star) }); content.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
            content.Children.Add(design.Icon(captured.IsDir ? "folder" : "doc", 16));
            var name = design.Text(captured.Name, 13); name.VerticalAlignment = VerticalAlignment.Center;
            name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis;
            var names = new StackPanel(); names.Children.Add(name);
            if (model.SearchingFiles && full.Contains('/'))
            {
                var parent = Mute(full[..full.LastIndexOf('/')]); parent.TextWrapping = TextWrapping.NoWrap; parent.TextTrimming = TextTrimming.CharacterEllipsis;
                names.Children.Add(parent);
            }
            Grid.SetColumn(names, 1); content.Children.Add(names);
            var meta = fileSort == "modified" && !model.SearchingFiles ? Modified(captured.ModifiedUnixMillis) : captured.IsDir ? "" : NativeFileListPresentation.Size(captured.Size);
            var detail = Mute(meta); detail.VerticalAlignment = VerticalAlignment.Center; Grid.SetColumn(detail, 2); content.Children.Add(detail);
            var button = new Button { Content = content, HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch,
                IsEnabled = !model.Loading && !model.FileTransferBusy };
            design.QuietButton(button); button.Padding = new Thickness(8); button.MinHeight = 36;
            if (model.SelectingFiles && model.SelectedFiles.Contains(full)) button.Background = design.Brush("clay-soft");
            AutomationProperties.SetItemStatus(button, model.SelectingFiles
                ? model.SelectedFiles.Contains(full) ? design.L("已选择", "Selected") : design.L("未选择", "Not selected") : "");
            AutomationProperties.SetName(button, full + (captured.IsDir ? design.L("，文件夹", ", folder") : ", " + meta)); ToolTipService.SetToolTip(button, full);
            button.Click += async (_, _) => {
                if (model.SelectingFiles) { model.ToggleFileSelection(full); fileCopyNotice = null; Render(); return; }
                if (captured.IsDir) await NavigateFilesAsync(full); else await FileReadAsync(model.ReadFileAsync(full, lifetime.Token));
            };
            if (!model.RemoteFiles && !captured.IsDir)
            {
                button.CanDrag = true;
                // Button handles the pointer gesture before WinUI's automatic drag
                // recognizer. Start a data drag after movement, keeping clicks intact.
                Windows.Foundation.Point? pressed = null;
                uint pointerId = 0; var dragging = false;
                button.AddHandler(UIElement.PointerPressedEvent, new PointerEventHandler((_, e) => {
                    var point = e.GetCurrentPoint(button);
                    if (point.Properties.IsLeftButtonPressed) { pressed = point.Position; pointerId = point.PointerId; }
                }), true);
                button.AddHandler(UIElement.PointerMovedEvent, new PointerEventHandler(async (_, e) => {
                    var point = e.GetCurrentPoint(button);
                    if (dragging || pressed is not { } origin || point.PointerId != pointerId || !point.Properties.IsLeftButtonPressed
                        || Math.Abs(point.Position.X - origin.X) + Math.Abs(point.Position.Y - origin.Y) < 8) return;
                    pressed = null;
                    if (disposed || model.FileDrag(full) == null) return;
                    dragging = true; e.Handled = true; button.ReleasePointerCaptures();
                    try { await button.StartDragAsync(point); }
                    catch (Exception ex) { if (!disposed) await ShowErrorAsync(ex.Message); }
                    finally { dragging = false; }
                }), true);
                button.AddHandler(UIElement.PointerReleasedEvent, new PointerEventHandler((_, _) => pressed = null), true);
                button.PointerCanceled += (_, _) => pressed = null;
                button.PointerCaptureLost += (_, _) => pressed = null;
                button.DragStarting += (_, e) => {
                    if (model.FileDrag(full) is not { } drag) { e.Cancel = true; return; }
                    e.Data.SetData(NativeFileDrag.Format, JsonSerializer.Serialize(drag));
                    e.Data.RequestedOperation = DataPackageOperation.Copy;
                    e.Data.Properties.Title = captured.Name;
                };
            }
            row.Children.Add(button);
            if (model.RemoteFiles && !captured.IsDir && pickDownload != null)
            {
                var download = FileButton(design.T("artifact.download") + " · " + captured.Name, "download");
                downloadButtons.Add(download); download.IsEnabled = model.CanDownloadFiles && !pickingFiles;
                download.Click += async (_, _) => await PickAndDownloadAsync(full, captured.Name);
                Grid.SetColumn(download, 1); row.Children.Add(download);
            }
            if (!model.RemoteFiles && model.CanSelectFiles)
            {
                var menu = new MenuFlyout();
                var copy = new MenuFlyoutItem { Text = design.T("files.copy_relative_path") };
                copy.Click += async (_, _) => await CopyFilePathsAsync(false, full); menu.Items.Add(copy);
                var absolute = new MenuFlyoutItem { Text = design.T("files.copy_absolute_path") };
                absolute.Click += async (_, _) => await CopyFilePathsAsync(true, full); menu.Items.Add(absolute);
                if (!captured.IsDir && conversation != null && model.FileDrag(full) is { } drag && conversation.CanReferenceFile(drag))
                {
                    var reference = new MenuFlyoutItem { Text = design.L("在对话中引用", "Reference in conversation") };
                    reference.Click += async (_, _) => { if (await conversation.ReferenceFileAsync(drag, lifetime.Token) && !disposed) close(); };
                    menu.Items.Add(reference);
                }
                if (model.CanMutateLocalFiles)
                {
                    menu.Items.Add(new MenuFlyoutSeparator());
                    var rename = new MenuFlyoutItem { Text = design.T("files.rename") }; rename.Click += async (_, _) => await PromptFileAsync(NativePanelFileAction.Rename, captured.Name);
                    var delete = new MenuFlyoutItem { Text = design.L("删除…", "Delete…") }; delete.Click += async (_, _) => await PromptFileAsync(NativePanelFileAction.Delete, captured.Name);
                    menu.Items.Add(rename); menu.Items.Add(delete);
                }
                TrackMenu(menu); button.ContextFlyout = menu;
                var more = FileButton(design.L(captured.Name + " 的操作", "Actions for " + captured.Name), "more"); more.Flyout = menu; Grid.SetColumn(more, 1); row.Children.Add(more);
            }
            fileRows.Children.Add(row);
        }
        if (entries.Length == 0 && !model.Loading && !model.FileSearchLoading && model.FileSearchError == null && model.Error == null)
        {
            if (query.Length == 0 && !model.RemoteFiles)
            {
                var empty = new NativePanelEmptyState(design, "files");
                var label = design.Text(design.T("right.browse_files"), 13); label.Foreground = design.Brush("clay");
                label.HorizontalAlignment = HorizontalAlignment.Center; label.TextDecorations = Windows.UI.Text.TextDecorations.Underline;
                empty.Children.Add(label);
                var browse = new Button { Content = empty, Padding = new Thickness(0), BorderThickness = new Thickness(0),
                    HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Stretch };
                design.QuietButton(browse);
                AutomationProperties.SetName(browse, design.T("right.browse_files"));
                AutomationProperties.SetAutomationId(browse, "files-empty-browse");
                browse.Click += async (_, _) => await FileReadAsync(model.RefreshAsync("files", cancellationToken: lifetime.Token));
                if (model.Preview == null)
                {
                    panelEmpty.Content = browse; panelEmpty.Visibility = Visibility.Visible; scroll.Visibility = Visibility.Collapsed;
                }
                return;
            }
            fileRows.Children.Add(design.EmptyState("folder", query.Length > 0 ? design.T("files.no_matches")
                : model.RemoteFiles ? design.T("files.empty_remote") : design.L("此文件夹为空", "This folder is empty"),
                query.Length > 0 ? design.L("尝试其他关键词。", "Try another search.") : model.FileLocations?.ReadOnly == true ? ""
                : model.RemoteFiles ? design.L("上传文件到此目录。", "Upload files to this folder.")
                : design.L("上传文件，或新建文件和文件夹。", "Upload files, or create a file or folder.")));
        }
    }
    private string FileLocationLabel(NativeFileLocation? location) => location?.Kind == "ssh" ? location.Label + " · SSH" : design.T("files.local_project");
    private Button FileButton(string label, string icon, bool text = false)
    {
        var button = design.ToolButton(label, icon, text); button.Padding = new Thickness(8, 6, 8, 6); return button;
    }
    private static string Modified(ulong? value) => value is > 0 and <= 253402300799999
        ? DateTimeOffset.FromUnixTimeMilliseconds((long)value.Value).LocalDateTime.ToString("g") : "";
    private async Task FileReadAsync(Task read) { Render(); try { await read; } catch (OperationCanceledException) { } Render(); }
    private async Task NavigateFilesAsync(string path) { fileCopyNotice = null; await FileReadAsync(model.RefreshAsync("files", path, lifetime.Token)); }
    private async Task CopyFilePathsAsync(bool absolute, string? clicked = null)
    {
        fileCopyNotice = null;
        var read = model.CopyFilePathsAsync(absolute, clicked, lifetime.Token); Render();
        var text = await read;
        if (disposed) return;
        if (text != null)
        {
            try { var data = new DataPackage(); data.SetText(text); Clipboard.SetContent(data); fileCopyNotice = design.L("路径已复制", "Paths copied"); }
            catch (Exception ex) { await ShowErrorAsync(ex.Message); }
        }
        Render();
    }
    private async Task PickAndUploadAsync()
    {
        if (pickingFiles || pickFiles == null || model.CaptureFileDestination() is not { } destination) return;
        pickingFiles = true; Render();
        try { var paths = await pickFiles(); if (!disposed) await FileReadAsync(model.UploadFilesAsync(destination, paths, lifetime.Token)); }
        catch (Exception ex) { if (!disposed) await ShowErrorAsync(ex.Message); }
        finally { pickingFiles = false; Render(); }
    }
    private async Task PickAndDownloadAsync(string path, string name)
    {
        if (pickingFiles || pickDownload == null || model.CaptureFileDownload() is not { } source) return;
        pickingFiles = true; Render();
        try { var destination = await pickDownload(name); if (!disposed) await FileReadAsync(model.DownloadFileAsync(source, path, destination, lifetime.Token)); }
        catch (Exception ex) { if (!disposed) await ShowErrorAsync(ex.Message); }
        finally { pickingFiles = false; Render(); }
    }
    private async void PollFileTransfers(Microsoft.UI.Dispatching.DispatcherQueueTimer sender, object args)
    {
        if (disposed || readingTransfers || model.FileTransferBusy) return;
        readingTransfers = true;
        try
        {
            var previousFiles = model.Files;
            await model.RefreshFileTransfersAsync(lifetime.Token);
            if (disposed) return;
            if (!model.HasPendingFileTransfers || model.FileTransferReadError != null) transferTimer.Stop();
            if (model.Tabs.Selected != "files") return;
            RenderFileTransferStatus();
            if (uploadButton != null) uploadButton.IsEnabled = model.CanUploadFiles && !pickingFiles;
            foreach (var button in downloadButtons) button.IsEnabled = model.CanDownloadFiles && !pickingFiles;
            if (!ReferenceEquals(previousFiles, model.Files)) RenderFileRows(filter.Text.Trim());
        }
        finally { readingTransfers = false; }
    }
    private void EnableFileDrop(UIElement target)
    {
        target.AllowDrop = true;
        target.DragOver += (_, e) => {
            if (model.CanUploadFiles && !pickingFiles && e.DataView.Contains(StandardDataFormats.StorageItems)) {
                e.AcceptedOperation = DataPackageOperation.Copy; e.Handled = true;
            }
        };
        target.Drop += async (_, e) => {
            if (!e.DataView.Contains(StandardDataFormats.StorageItems) || model.CaptureFileDestination() is not { } destination) return;
            var deferral = e.GetDeferral(); e.Handled = true;
            try {
                var paths = (await e.DataView.GetStorageItemsAsync()).OfType<StorageFile>().Select(f => f.Path).Distinct().ToArray();
                if (!disposed) await FileReadAsync(model.UploadFilesAsync(destination, paths, lifetime.Token));
            }
            catch (Exception ex) { if (!disposed) await ShowErrorAsync(ex.Message); }
            finally { deferral.Complete(); }
        };
    }
}
