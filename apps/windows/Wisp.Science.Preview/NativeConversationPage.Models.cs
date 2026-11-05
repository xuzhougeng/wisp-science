using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeConversationPage
{
    private readonly Flyout modelMenu = new() { Placement = FlyoutPlacementMode.TopEdgeAlignedRight };
    private readonly StackPanel modelMenuBody = new(), modelChoices = new();
    private readonly ScrollViewer modelMenuScroll = new() { HorizontalScrollBarVisibility = ScrollBarVisibility.Disabled,
        VerticalScrollBarVisibility = ScrollBarVisibility.Auto };
    private readonly Button manageModels = new();
    private readonly TextBlock modelMenuStatus = new() { TextWrapping = TextWrapping.Wrap, Margin = new(10, 6, 10, 6) };
    private readonly List<(Button Button, ComposerModelMenuChoice Choice)> modelChoiceButtons = [];
    private readonly Action? openModelSettings;
    private bool modelMenuOpen;
    private readonly NativeTransientNotice acpDraftNotice = new();
    private string? modelMenuFingerprint;
    private (string? Project, string? Session, bool History) modelMenuOwner;
    private bool AcpSelected => model.ComposerAgentId != null;
    private string? ConversationOperationError => model.CreationFailureDetail is { } detail
        ? design.L("新建会话未确认成功，请刷新列表后检查：", "Conversation creation was not confirmed. Refresh the session list to check: ") + detail
        : model.DraftFastFailureDetail is { } fastDetail
            ? design.L("无法确认草稿的 Fast 模式。请核对闪电按钮后继续。", "Draft Fast mode was not confirmed. Check the lightning button before continuing.") + (fastDetail.Length == 0 ? "" : "\n" + fastDetail)
        : model.DraftModelFailureDetail is { } modelDetail
            ? design.L("默认模型修改结果尚未确认，请重新读取核对：", "Default model change was not confirmed. Reload to check: ") + modelDetail
            : model.ResumeFailureDetail is { } resumeDetail
                ? design.L("继续执行的结果尚未确认，请重新读取会话后再继续；不会自动重试。\n", "Resume was not confirmed. Reload the conversation before continuing; it will not be retried automatically.\n") + resumeDetail
            : model.OperationError switch
            {
                "Open a conversation before compacting its context." => design.L("请先打开一个会话，再压缩其上下文。", "Open a conversation before compacting its context."),
                "ACP conversations cannot be compacted from this desktop view." => design.L("ACP 会话无法在此桌面界面中压缩上下文。", "ACP conversations cannot be compacted from this desktop view."),
                "Wait for the conversation to finish before compacting its context." => design.L("请等待当前会话运行结束，再压缩其上下文。", "Wait for the conversation to finish before compacting its context."),
                _ => model.OperationError
            };
    private bool CanPickModel => !preparingSession && !model.Busy && !model.Effort.Busy && !model.Options.Busy
        && (model.CanPrepareProjectDraft || model.CanAttach && model.Snapshot is { Running: false });

    private void BuildModelPicker()
    {
        models.Padding = new(11, 0, 11, 0); models.BorderThickness = new(1);
        models.HorizontalContentAlignment = HorizontalAlignment.Stretch;
        AutomationProperties.SetAutomationId(models, "composer-model-trigger");
        modelMenuBody.Children.Add(modelChoices); modelMenuBody.Children.Add(acpSettingsSection);
        modelMenuBody.Children.Add(modelMenuStatus);
        manageModels.HorizontalAlignment = HorizontalAlignment.Stretch;
        manageModels.HorizontalContentAlignment = HorizontalAlignment.Left;
        manageModels.Padding = new(10, 9, 10, 9); manageModels.BorderThickness = new(0, 1, 0, 0);
        design.QuietButton(manageModels); design.BindTypography(manageModels, size: 12.5);
        modelMenuBody.Children.Add(manageModels);
        manageModels.Click += (_, _) => { modelMenu.Hide(); openModelSettings?.Invoke(); };
        modelMenuScroll.Content = modelMenuBody; modelMenu.Content = modelMenuScroll;
        modelMenu.Opened += (_, _) => modelMenuOpen = true;
        modelMenu.Closed += (_, _) => { modelMenuOpen = false; foreach (var selector in acpSelectors) selector.IsDropDownOpen = false; };
        models.Click += (_, _) =>
        {
            if (modelMenuOpen) { modelMenu.Hide(); return; }
            composerOptions.Hide(); optionSubmenu.Hide(); composeMenu.Hide(); effortMenu.Hide();
            RefreshModelPicker(); RefreshAcpSettings(); modelMenu.ShowAt(models);
            DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low,
                () => { if (!disposed && modelMenuOpen) modelMenuScroll.ChangeView(null, 0, null, true); });
        };
        Unloaded += (_, _) => { modelMenu.Hide(); CloseModelSwitchConfirmation(false); CloseCompactionDialog(); acpDraftNotice.Hide(); };
        Loaded += (_, _) => RefreshCompactionDialog();
    }

    private void RefreshModelPicker()
    {
        var owner = (Project: model.CurrentProjectId, Session: model.CurrentSessionId, History: model.ShowingHistory);
        if (owner != modelMenuOwner) { modelMenu.Hide(); CloseModelSwitchConfirmation(false); acpDraftNotice.Hide(); modelMenuOwner = owner; }
        if (!CanPickModel) CloseModelSwitchConfirmation(false);
        var choices = model.ModelMenuChoices;
        var label = choices.FirstOrDefault(c => c.Selected)?.Label
            ?? (AcpSelected ? design.L("ACP Agent", "ACP Agent") : design.L("默认", "Default"));
        var trigger = new Grid { ColumnSpacing = 5 };
        trigger.ColumnDefinitions.Add(new()); trigger.ColumnDefinitions.Add(new() { Width = GridLength.Auto });
        var text = design.Text(label, 12); text.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
        text.TextWrapping = TextWrapping.NoWrap; text.TextTrimming = TextTrimming.CharacterEllipsis;
        text.VerticalAlignment = VerticalAlignment.Center; text.Foreground = design.Brush("text-muted"); trigger.Children.Add(text);
        var chevron = design.Icon("chevron-down", 11, design.ColorText("text-faint")); Grid.SetColumn(chevron, 1); trigger.Children.Add(chevron);
        models.Content = trigger; models.BorderBrush = design.Brush("border-strong"); models.Background = design.Brush("bg-elev");
        models.Visibility = choices.Length > 0 || AcpSelected ? Visibility.Visible : Visibility.Collapsed;
        // Read-only/running sessions can inspect their selected backend and settings.
        models.IsEnabled = !model.Loading && !disposed && (model.HasProjectDraft || model.Snapshot != null);
        AutomationProperties.SetName(models, design.L("对话模型", "Conversation model") + ": " + label);
        ToolTipService.SetToolTip(models, label);
        manageModels.Content = design.T("models.manage"); manageModels.Visibility = openModelSettings == null ? Visibility.Collapsed : Visibility.Visible;
        manageModels.Foreground = design.Brush("clay-strong"); manageModels.BorderBrush = design.Brush("border");
        modelMenuStatus.Text = ConversationOperationError ?? model.ConnectionError ?? "";
        modelMenuStatus.Visibility = modelMenuStatus.Text.Length == 0 ? Visibility.Collapsed : Visibility.Visible;
        modelMenuStatus.Foreground = design.Brush("text-muted"); modelMenuStatus.FontSize = design.FontSize(11); modelMenuStatus.FontFamily = design.Font();
        modelMenuBody.Width = Math.Max(220, Math.Min(340, (XamlRoot?.Size.Width ?? 368) - 28));
        modelMenuScroll.MaxHeight = Math.Max(120, Math.Min(308, (XamlRoot?.Size.Height ?? 640) / 2 - 14));
        var fingerprint = System.Text.Json.JsonSerializer.Serialize(choices) + $"/{owner}/{design.Localization.Locale}/{design.Dark}/{design.Typography}";
        if (fingerprint != modelMenuFingerprint)
        {
            modelMenuFingerprint = fingerprint;
            var presenter = new Style(typeof(FlyoutPresenter));
            presenter.Setters.Add(new Setter(Control.BackgroundProperty, design.Brush("bg-elev")));
            presenter.Setters.Add(new Setter(Control.BorderBrushProperty, design.Brush("border-strong")));
            presenter.Setters.Add(new Setter(Control.CornerRadiusProperty, new CornerRadius(12)));
            presenter.Setters.Add(new Setter(Control.PaddingProperty, new Thickness(6)));
            modelMenu.FlyoutPresenterStyle = presenter;
            modelChoices.Children.Clear(); modelChoiceButtons.Clear();
            var acpHeading = false;
            foreach (var choice in choices)
            {
                if (choice.Acp && !acpHeading)
                {
                    acpHeading = true; var heading = design.Text("ACP", 11); heading.Foreground = design.Brush("text-faint");
                    heading.Margin = new(10, 5, 10, 2); modelChoices.Children.Add(heading);
                }
                var row = new Grid { ColumnSpacing = 8 };
                row.ColumnDefinitions.Add(new()); row.ColumnDefinitions.Add(new() { Width = new(14) });
                var labels = new StackPanel { Spacing = 1 };
                var name = design.Text(choice.Label, 13); name.FontWeight = Microsoft.UI.Text.FontWeights.Medium;
                name.TextWrapping = TextWrapping.NoWrap; name.TextTrimming = TextTrimming.CharacterEllipsis; labels.Children.Add(name);
                if (choice.Detail.Length > 0)
                {
                    var detail = design.Text(choice.Detail, 11); detail.FontFamily = design.Font(true);
                    detail.Foreground = design.Brush("text-faint"); detail.TextWrapping = TextWrapping.NoWrap; detail.TextTrimming = TextTrimming.CharacterEllipsis;
                    labels.Children.Add(detail);
                }
                row.Children.Add(labels);
                if (choice.Selected) { var check = design.Icon("check", 14, design.ColorText("clay-strong")); Grid.SetColumn(check, 1); row.Children.Add(check); }
                var button = new Button { Content = row, HorizontalAlignment = HorizontalAlignment.Stretch,
                    HorizontalContentAlignment = HorizontalAlignment.Stretch, Padding = new(10, 8, 10, 8), MinHeight = 0,
                    Background = design.Brush("bg-elev"), BorderThickness = new(0), CornerRadius = new(6) };
                button.Resources["ButtonBackgroundPointerOver"] = design.Brush("bg-sunken");
                button.Resources["ButtonBackgroundDisabled"] = design.Brush("bg-elev");
                button.Resources["ButtonBorderBrushDisabled"] = design.Brush("bg-elev");
                AutomationProperties.SetName(button, choice.Label);
                AutomationProperties.SetAutomationId(button, "composer-model-" + choice.Id);
                if (choice.Selected) AutomationProperties.SetHelpText(button, design.L("当前选中", "Selected"));
                button.Click += async (_, _) =>
                {
                    modelMenu.Hide();
                    if (disposed || owner != (model.CurrentProjectId, model.CurrentSessionId, model.ShowingHistory) || !CanPickModel || choice.Selected) return;
                    if (choice.Acp)
                    {
                        if (model.CanSelectAcpInPlace) model.SelectProvisionalAgent(choice.Id[4..]);
                        else if (createAcp != null)
                        {
                            var revision = model.AcpDraftRevision;
                            await createAcp(choice.Id[4..]);
                            if (!disposed && revision != model.AcpDraftRevision
                                && owner.Project == model.CurrentProjectId && owner.Session != model.CurrentSessionId
                                && model.ComposerAgentId == choice.Id[4..])
                            {
                                var target = model.CurrentSessionId; var acceptedRevision = model.AcpDraftRevision;
                                // The shell reparents this page while adopting the new row.
                                // Show after that layout turn, and recheck the destination.
                                DispatcherQueue.TryEnqueue(Microsoft.UI.Dispatching.DispatcherQueuePriority.Low, () =>
                                {
                                    if (!disposed && IsLoaded && XamlRoot?.Content is Grid windowRoot && acceptedRevision == model.AcpDraftRevision
                                        && owner.Project == model.CurrentProjectId && target == model.CurrentSessionId)
                                        acpDraftNotice.Show(windowRoot, design, design.T("composer.acp_new_session_toast"));
                                });
                            }
                        }
                    }
                    else if (!model.AcpModelLocked && !model.RestoreProvisionalHttpModel(choice.Id)) await RequestHttpModelSwitchAsync(choice, owner);
                };
                modelChoices.Children.Add(button); modelChoiceButtons.Add((button, choice));
            }
        }
        foreach (var (button, choice) in modelChoiceButtons)
        {
            button.IsEnabled = CanPickModel && (choice.Acp ? !model.AssistantMode && (choice.Selected || model.CanSelectAcpInPlace || createAcp != null) : !model.AcpModelLocked);
            ToolTipService.SetToolTip(button, !choice.Acp && model.AcpModelLocked ? design.T("models.locked_hint")
                : choice.Acp && !choice.Selected && !model.CanSelectAcpInPlace ? design.L("使用此 ACP Agent 新建会话", "Start a new conversation with this ACP Agent") : choice.Tooltip);
        }
    }
}
