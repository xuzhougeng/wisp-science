using System.Text.Json.Nodes;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private NativeLiveAppearanceModel? inputPreferences;
    private readonly StackPanel inputPreferenceNotice = new() { Spacing = 8 };
    private Action<JsonObject>? applyInputPreferences;
    private NativeInputPreferences? appliedInputPreferences;
    private ComboBox? inputShortcut;
    private ToggleSwitch? inputSelection;
    private bool refreshingInputPreferences;

    private void ChangeInputPreference(string key, bool value)
    {
        if (model.Closed || refreshingInputPreferences || inputPreferences?.Draft == null) return;
        inputPreferences.Set(key, JsonValue.Create(value));
    }

    private void RefreshInputPreferences()
    {
        if (model.Closed || inputPreferences == null) return;
        if (inputPreferences.Draft is { } draft)
        {
            var current = NativeInputPreferences.From(draft);
            refreshingInputPreferences = true;
            try
            {
                if (inputShortcut != null)
                    inputShortcut.SelectedItem = inputShortcut.Items.Cast<ComboBoxItem>().FirstOrDefault(item =>
                        (string)item.Tag == (current.SendWithModifier ? "modifier" : "enter"));
                if (inputSelection != null) inputSelection.IsOn = current.SelectionPopupEnabled;
            }
            finally { refreshingInputPreferences = false; }
            if (appliedInputPreferences != current)
            {
                appliedInputPreferences = current;
                applyInputPreferences?.Invoke(draft);
            }
        }
        inputPreferenceNotice.Children.Clear();
        if (inputPreferences.Error is { } error)
        {
            inputPreferenceNotice.Children.Add(Warn(L(
                "交互设置已应用，但保存未确认。修改已保留；点击重试以再次保存。\n",
                "Interaction settings are applied, but saving was not confirmed. Changes are retained; choose Retry to save again.\n") + error));
            var retry = Button(L("重试", "Retry"), () => { inputPreferences.Retry(); return Task.CompletedTask; });
            retry.IsEnabled = !inputPreferences.Saving; inputPreferenceNotice.Children.Add(retry);
        }
        inputPreferenceNotice.Visibility = inputPreferences.Error == null ? Visibility.Collapsed : Visibility.Visible;
        LockConfirmation();
    }
}
