using System.Text.Json.Nodes;
using Wisp.ProjectBrowser.Contracts;

namespace Wisp.ProjectBrowser;

public sealed class NativeQuickSetupModel(INativeSettingsClient host, NativeLocalization locale) : WorkspaceActionModel
{
    public int Step { get; private set; }
    public NativeLocalEnvironmentModel Environment { get; } = new(host, null);
    public bool Finished { get; private set; }
    public static async Task<bool> ShouldShowAsync(INativeSettingsClient host) =>
        (await host.InvokeAsync("get_onboarding_state", new()))?["show"]?.GetValue<bool>() == true;
    public Task DismissAsync() => host.InvokeAsync("dismiss_onboarding", new());

    public void Back()
    {
        if (Closed || Busy || Step == 0) return;
        if (Step == 3 && Environment.Editing && !Environment.Busy) Environment.Cancel();
        Step--; ClearFailure(); Notify();
    }

    public async Task AdvanceAsync(string key)
    {
        if (Closed || Busy || Finished) return;
        if (Step == 3) { Finished = true; Notify(); return; }
        if (Step != 2 || string.IsNullOrWhiteSpace(key)) { Step++; ClearFailure(); Notify(); return; }

        // save_model activates the last profile written. Use the same order and
        // payload as the WebView, and never put the pasted key in a profile or error.
        await RunAsync(async () =>
        {
            string? flashId = null;
            foreach (var name in new[] { "deepseek-v4-pro", "deepseek-v4-flash" })
            {
                var profile = new JsonObject { ["id"] = "", ["label"] = "", ["provider"] = "openai",
                    ["api_url"] = "https://api.deepseek.com", ["model"] = name, ["max_tokens"] = 8192,
                    ["reasoning_effort"] = "", ["supports_vision"] = false, ["use_for_vision"] = false,
                    ["use_for_image_generation"] = false, ["use_for_video_generation"] = false };
                var args = new JsonObject { ["profile"] = profile, ["key"] = key,
                    ["useForVision"] = false, ["useForImageGeneration"] = false, ["useForVideoGeneration"] = false };
                if (await host.InvokeAsync("save_model", args) is not JsonArray models)
                    throw new InvalidDataException();
                if (Closed) return false;
                if (name == "deepseek-v4-flash")
                    flashId = models.OfType<JsonObject>().FirstOrDefault(p => p["active"]?.GetValue<bool>() == true
                        && p["model"]?.ToString() == name && p["api_url"]?.ToString().TrimEnd('/') == "https://api.deepseek.com")?["id"]?.ToString();
            }
            if (string.IsNullOrWhiteSpace(flashId)) throw new InvalidDataException();
            var specialists = await host.InvokeAsync("list_specialists", new()) as JsonArray ?? throw new InvalidDataException();
            if (Closed) return false;
            if (specialists.OfType<JsonObject>().FirstOrDefault(s => s["id"]?.ToString() == "reader"
                && string.IsNullOrWhiteSpace(s["model_id"]?.ToString()))?.DeepClone() is JsonObject reader)
            {
                reader["model_id"] = flashId;
                await host.InvokeAsync("save_specialist_cmd", new() { ["spec"] = reader });
            }
            return true;
        }, _ => Step++, _ => locale.Choose("未能确认模型配置。请在「设置 → 模型」中检查已保存的模型，或跳过此步继续。",
            "Model setup could not be confirmed. Check the saved profiles in Settings → Models, or skip this step to continue."));
    }

    public override void Dispose() { Environment.Dispose(); base.Dispose(); }
}
