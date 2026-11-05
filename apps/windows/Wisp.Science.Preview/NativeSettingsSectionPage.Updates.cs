using System.Diagnostics;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Input;
using Wisp.ProjectBrowser;

namespace Wisp.Science.Preview;

internal sealed partial class NativeSettingsSectionPage
{
    private NativeUpdateCheckPage? updateDialog;
    private Control? updatePreviousFocus;
    private ToggleSwitch? updateCheckToggle;
    private bool refreshingUpdatePreference;
    private void OpenUpdateDialog()
    {
        if (updateDialog != null || settingsClient == null || Busy || model.Closed) return;
        updatePreviousFocus = FocusManager.GetFocusedElement(XamlRoot) as Control;
        // Commit this independent preference through the editor so a later
        // General Save/Cancel cannot restore the previous reminder value.
        var check = new NativeUpdateCheckModel(settingsClient, () => model.SetUpdateCheckEnabledAsync(false));
        updateDialog = new(check, Design, () => CloseUpdateDialog(),
            url => Process.Start(new ProcessStartInfo(url) { UseShellExecute = true }));
        ModalChanged?.Invoke(updateDialog);
    }
    private void CloseUpdateDialog(bool restoreFocus = true)
    {
        if (updateDialog == null) return;
        updateDialog.Dispose(); updateDialog = null; ModalChanged?.Invoke(null);
        if (updateCheckToggle != null && model.Values["get_update_check_enabled"] is System.Text.Json.Nodes.JsonValue value
            && value.TryGetValue<bool>(out var enabled))
        {
            refreshingUpdatePreference = true;
            try { updateCheckToggle.IsOn = enabled; }
            finally { refreshingUpdatePreference = false; }
        }
        if (restoreFocus) updatePreviousFocus?.Focus(FocusState.Programmatic);
        updatePreviousFocus = null;
    }
}
